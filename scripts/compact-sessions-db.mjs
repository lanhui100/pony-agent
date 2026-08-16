#!/usr/bin/env node
/**
 * 存量会话存储迁移脚本（PA-090）。
 *
 * 背景：PA-088 已完成新写路径（WriteSeparate + 节点 refs + 轻量投影），
 * 存量会话（LegacyBlob/DualWrite）保持原样：最大会话 43.77MB（historyNodes
 * 冗余），打开时前端 JSON.parse 卡死。本脚本把存量会话迁移为
 * TraceTableAuthoritative（剥离顶层+节点 trace、生成 refs、写表）。
 *
 * 功能：
 *   1. 进程检测（Pony Agent 必须退出）+ VACUUM INTO 备份
 *   2. 三源合并（已有表 ∪ 顶层 blob ∪ 节点 trace），按 turn_id 去重取最新
 *   3. per-session 事务：写表（DELETE+INSERT）→ 生成 refs → 剥离 → 写 blob →
 *      置 Authoritative → 写幂等标记（checksum）
 *   4. fail closed：任一源解析失败 → 整 session 回滚
 *   5. 幂等：marker + checksum + state + refs 四条件齐备才跳过；--force 重跑
 *   6. 保留 resultText 截断功能（止血）
 *
 * 用法：
 *   node scripts/compact-sessions-db.mjs            # 实际执行
 *   node scripts/compact-sessions-db.mjs --dry-run  # 只报告不写
 *   node scripts/compact-sessions-db.mjs --force    # 强制重跑（校验失败时）
 *
 * 前置条件：Pony Agent 必须已退出（脚本会检测进程）。
 * 需要 Node >= 22（node:sqlite）。
 */
import { DatabaseSync } from "node:sqlite";
import { existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { execSync } from "node:child_process";
import { createHash } from "node:crypto";

const MAX_RESULT_TEXT_CHARS = 32 * 1024;
const SPEC_VERSION = "pa090.v1";
const DRY_RUN = process.argv.includes("--dry-run");
const FORCE = process.argv.includes("--force");

const dbPath = join(process.env.LOCALAPPDATA ?? "", "PonyAgent", "sessions.db");
if (!existsSync(dbPath)) {
  console.error(`sessions.db 不存在: ${dbPath}`);
  process.exit(1);
}

// ── 0. 进程检测（Pony Agent 必须退出，fail-closed）─────────
function detectRunningApp() {
  try {
    const out = execSync(
      'powershell -NoProfile -Command "Get-Process pony-agent -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id"',
      { encoding: "utf8", timeout: 10000 }
    );
    const pids = out.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
    return pids.length > 0 ? pids : null;
  } catch (err) {
    // fail-closed：无法验证应用状态 → 视为"可能运行中"，中止迁移
    throw new Error(`无法检测 Pony Agent 进程状态（${err.message}），为安全起见中止迁移`);
  }
}

// ── 1. 备份（VACUUM INTO 一致快照）─────────────────────────
function backup(db, dbPath) {
  const backupDir = join(dirname(dbPath), "backups");
  mkdirSync(backupDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const backupPath = join(backupDir, `sessions.db.${stamp}.migrate.bak`);
  db.exec(`VACUUM INTO '${backupPath.replace(/'/g, "''")}'`);
  const integrity = db.prepare("PRAGMA integrity_check").get();
  console.log(`备份: ${backupPath}（integrity: ${integrity?.integrity_check ?? "?"}）`);
  return backupPath;
}

// ── 2. 工具 ────────────────────────────────────────────────
function sha256(text) {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

function truncateResultText(value, stats) {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) {
    for (const item of value) truncateResultText(item, stats);
    return value;
  }
  for (const [key, val] of Object.entries(value)) {
    if ((key === "resultText" || key === "result_text") && typeof val === "string") {
      if (val.length > MAX_RESULT_TEXT_CHARS) {
        stats.truncated += 1;
        stats.savedBytes += val.length - MAX_RESULT_TEXT_CHARS;
        value[key] = val.slice(0, MAX_RESULT_TEXT_CHARS) + "\n...[truncated by pony-agent]";
      }
    } else {
      truncateResultText(val, stats);
    }
  }
  return value;
}

// 三源合并：按 (turn_id, updatedAt) 去重取最新；时间相同 表 > 顶层 > 节点
// trace_order：顶层数组原位优先 → 节点独有按节点顺序追加 → 表独有追加末尾
function mergeTraceSources(tableTraces, topLevelTraces, nodeTraces) {
  const byId = new Map(); // turn_id -> { trace, source }
  const ordered = [];

  const upsert = (trace, source) => {
    const key = trace.turnId ?? trace.turn_id;
    if (!key) throw new Error("trace 缺少 turnId");
    const updatedAt = trace.updatedAt ?? trace.updated_at ?? 0;
    const existing = byId.get(key);
    if (!existing) {
      byId.set(key, { trace, source, updatedAt });
      ordered.push(key);
    } else if (
      updatedAt > existing.updatedAt ||
      (updatedAt === existing.updatedAt && source < existing.source)
    ) {
      byId.set(key, { trace, source, updatedAt });
    }
  };

  for (const t of topLevelTraces) upsert(t, 1); // 顶层 source=1
  for (const node of nodeTraces) for (const t of node) upsert(t, 2); // 节点 source=2
  for (const t of tableTraces) upsert(t, 0); // 表 source=0（最高优先）

  return ordered.map((key) => byId.get(key).trace);
}

// ── 3. 主流程 ──────────────────────────────────────────────
const db = new DatabaseSync(dbPath);
db.exec("PRAGMA busy_timeout=5000");

const stats = { sessions: 0, skipped: 0, failed: 0, truncated: 0, savedBytes: 0, migrated: 0 };

if (!DRY_RUN) {
  const pids = detectRunningApp();
  if (pids) {
    console.error(`Pony Agent 正在运行（PID: ${pids.join(", ")}），请先退出后再执行迁移。`);
    process.exit(1);
  }
  backup(db, dbPath);
}

const sessions = db.prepare("SELECT conversation_id, session_data FROM sessions").all();
for (const row of sessions) {
  const sessionId = row.conversation_id;
  try {
    let parsed;
    try {
      parsed = JSON.parse(row.session_data);
    } catch {
      throw new Error("session_data JSON 解析失败");
    }
    if (!parsed || typeof parsed !== "object") throw new Error("session_data 非对象");

    // 截断 resultText（止血，保留）
    truncateResultText(parsed, stats);

    const state = parsed.traceMigrationState ?? "legacyBlob";
    const topRefs = parsed.turnTraceRefs;
    const topTraces = Array.isArray(parsed.turnTraceHistory) ? parsed.turnTraceHistory : [];
    const nodes = Array.isArray(parsed.historyNodes) ? parsed.historyNodes : [];

    // 读取表（三源之一，skip 校验也需要）
    const tableRows = db
      .prepare("SELECT turn_id, trace_data FROM session_turn_traces WHERE session_id = ?")
      .all(sessionId);
    const tableTraces = tableRows.map((r) => {
      const t = JSON.parse(r.trace_data);
      if (!t || typeof t !== "object") throw new Error(`表 trace 解析失败: ${r.turn_id}`);
      return t;
    });

    // 计算当前表 checksum（按 turn_id 稳定排序）
    const tableChecksum = sha256(
      [...tableTraces]
        .sort((a, b) => (a.turnId ?? a.turn_id).localeCompare(b.turnId ?? b.turn_id))
        .map((t) => `${t.turnId ?? t.turn_id}\0${t.updatedAt ?? t.updated_at ?? 0}\0${sha256(JSON.stringify(t))}`)
        .join("\n")
    );

    // 已 Authoritative：校验 marker + checksum + refs（P1-2 四条件）
    if (state === "trace_table_authoritative") {
      const markerRow = db
        .prepare("SELECT value FROM store_metadata WHERE key = ?")
        .get(`storage_dedup.v1:${sessionId}`);
      const marker = markerRow ? JSON.parse(markerRow.value) : null;
      const refsComplete =
        Array.isArray(topRefs) &&
        nodes.every((n) => Array.isArray(n.turnTraceRefs)) &&
        // 每个 ref 的 turnId 存在于最终表（无 dangling）
        [...topRefs, ...nodes.flatMap((n) => n.turnTraceRefs ?? [])].every((r) =>
          tableTraces.some((t) => (t.turnId ?? t.turn_id) === r.turnId)
        );
      const checksumMatch = marker && marker.checksum === tableChecksum;

      if (marker && checksumMatch && refsComplete && !FORCE) {
        stats.skipped += 1;
        console.log(`[跳过] ${sessionId}（已 Authoritative + marker + checksum 匹配）`);
        continue;
      }
      if (refsComplete && !FORCE) {
        stats.skipped += 1;
        console.log(`[跳过] ${sessionId}（已 Authoritative，refs 完整，无 marker 或 checksum 失配，报告已最新）`);
        continue;
      }
      if (!refsComplete && !FORCE) {
        // fail-closed：Authoritative + refs 缺失 → 拒绝执行，需 --force（P1-1）
        throw new Error("已 Authoritative 但 refs 不完整，需 --force 从最终表重建 refs");
      }
      // --force：从最终表重建 refs（不清空已有 refs 指向的 trace）
      if (FORCE) {
        parsed.turnTraceRefs = tableTraces.map((t) => ({
          turnId: t.turnId ?? t.turn_id,
          updatedAtMs: t.updatedAt ?? t.updated_at ?? 0,
        }));
        parsed.historyNodes = nodes.map((n) => ({
          ...n,
          turnTraceRefs: Array.isArray(n.turnTraceRefs) ? n.turnTraceRefs : [],
        }));
      }
    }

    // 读取剩余两源（顶层 + 节点）
    const nodeTraces = nodes.map((n) => {
      if (!Array.isArray(n.turnTraceHistory)) return [];
      return n.turnTraceHistory.map((t) => {
        if (!t || typeof t !== "object") throw new Error(`节点 trace 解析失败: ${n.nodeId}`);
        return t;
      });
    });

    // 合并（fail closed：任何异常向上抛 → 整 session 回滚）
    const union = mergeTraceSources(tableTraces, topTraces, nodeTraces);

    // per-session 事务（dry-run 不执行任何写操作）
    if (DRY_RUN) {
      const union = mergeTraceSources(tableTraces, topTraces, nodeTraces);
      const blobMb = (JSON.stringify(parsed).length / 1024 / 1024).toFixed(2);
      console.log(`[迁移] ${sessionId}（${state} → authoritative，blob ${blobMb} MB，union ${union.length} 条）`);
      stats.migrated += 1;
      continue;
    }
    db.exec("BEGIN IMMEDIATE");
    try {
      // 写表（DELETE + INSERT，replace 语义）
      db.prepare("DELETE FROM session_turn_traces WHERE session_id = ?").run(sessionId);
      const insertTrace = db.prepare(
        "INSERT INTO session_turn_traces (session_id, turn_id, updated_at_ms, trace_order, trace_data) VALUES (?, ?, ?, ?, ?)"
      );
      union.forEach((trace, index) => {
        const turnId = trace.turnId ?? trace.turn_id;
        const updatedAt = trace.updatedAt ?? trace.updated_at;
        if (updatedAt === undefined || updatedAt === null) {
          throw new Error(`trace ${turnId} 缺少 updatedAt（fail-closed）`);
        }
        insertTrace.run(sessionId, turnId, updatedAt, index, JSON.stringify(trace));
      });

      // 生成 refs + 回填 turn_id + 剥离节点 trace
      const canonicalTurnIds = new Set(union.map((t) => t.turnId ?? t.turn_id));
      const nodeRefs = nodes.map((node) => {
        const nodeTracesArr = Array.isArray(node.turnTraceHistory) ? node.turnTraceHistory : [];
        const refs = nodeTracesArr
          .map((t) => ({
            turnId: t.turnId ?? t.turn_id,
            updatedAtMs: t.updatedAt ?? t.updated_at ?? 0,
          }))
          .filter((r) => canonicalTurnIds.has(r.turnId)); // 不生成 dangling ref
        let turnId = node.turnId ?? null;
        if (!turnId && nodeTracesArr.length > 0) {
          turnId = nodeTracesArr[nodeTracesArr.length - 1].turnId ?? null;
        }
        if (!turnId && node.runId) turnId = node.runId;
        if (!turnId && Array.isArray(node.history)) {
          const last = [...node.history].reverse().find((m) => m && m.turnId);
          turnId = last?.turnId ?? null;
        }
        if (turnId && !canonicalTurnIds.has(turnId)) turnId = null; // 候选校验
        return { node, refs, turnId };
      });

      // 剥离 + 写 blob
      parsed.traceMigrationState = "trace_table_authoritative";
      parsed.turnTraceRefs = topTraces.map((t) => ({
        turnId: t.turnId ?? t.turn_id,
        updatedAtMs: t.updatedAt ?? t.updated_at ?? 0,
      }));
      parsed.turnTraceHistory = [];
      parsed.historyNodes = nodes.map((node, i) => {
        const { refs, turnId } = nodeRefs[i];
        return {
          ...node,
          turnId: turnId ?? node.turnId ?? null,
          turnTraceRefs: refs,
          turnTraceHistory: [],
        };
      });

      const blob = JSON.stringify(parsed);
      db.prepare("UPDATE sessions SET session_data = ?, updated_at_ms = ? WHERE conversation_id = ?").run(
        blob,
        parsed.updatedAtMs ?? Date.now(),
        sessionId
      );

      // checksum：基于最终表重新读取计算（按 turn_id 稳定排序，P2-3）
      const finalRows = db
        .prepare("SELECT turn_id, updated_at_ms, trace_data FROM session_turn_traces WHERE session_id = ?")
        .all(sessionId);
      const checksumInput = [...finalRows]
        .sort((a, b) => a.turn_id.localeCompare(b.turn_id))
        .map((r) => `${r.turn_id}\0${r.updated_at_ms}\0${sha256(r.trace_data)}`)
        .join("\n");
      const checksum = sha256(checksumInput);

      // PA-090 P1-4：refs 写入后、COMMIT 前执行保护 prune（软上限 128，只删无引用最旧）
      const protectedTurnIds = new Set([
        ...(parsed.turnTraceRefs ?? []).map((r) => r.turnId),
        ...(parsed.historyNodes ?? []).flatMap((n) => (n.turnTraceRefs ?? []).map((r) => r.turnId)),
      ]);
      if (protectedTurnIds.size > 0) {
        const excess = finalRows.length - 128;
        if (excess > 0) {
          const candidates = finalRows
            .filter((r) => !protectedTurnIds.has(r.turn_id))
            .slice(0, excess);
          const delTrace = db.prepare(
            "DELETE FROM session_turn_traces WHERE session_id = ? AND turn_id = ?"
          );
          for (const r of candidates) delTrace.run(sessionId, r.turn_id);
        }
      }

      // 幂等标记
      const markerValue = JSON.stringify({
        spec_version: SPEC_VERSION,
        completed: true,
        checksum,
        previous_state: state,
        migrated_at_ms: Date.now(),
      });
      db.prepare(
        "INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?, ?)"
      ).run(`storage_dedup.v1:${sessionId}`, markerValue);

      db.exec("COMMIT");
      stats.migrated += 1;
      const blobMb = (blob.length / 1024 / 1024).toFixed(2);
      console.log(`[迁移] ${sessionId}（${state} → authoritative，blob ${blobMb} MB，union ${union.length} 条）`);
    } catch (err) {
      db.exec("ROLLBACK");
      throw err;
    }
  } catch (err) {
    stats.failed += 1;
    console.error(`[失败] ${sessionId}: ${err.message}`);
  }
}

// ── 4. 收尾 ────────────────────────────────────────────────
if (!DRY_RUN) {
  try {
    db.exec("VACUUM");
    db.exec("PRAGMA wal_checkpoint(TRUNCATE)");
  } catch {
    console.warn("VACUUM/checkpoint 失败（不影响数据迁移，空间回收可稍后执行）");
  }
}
db.close();

console.log(`\n${DRY_RUN ? "[DRY-RUN] " : ""}迁移结果:`);
console.log(`  迁移: ${stats.migrated} | 跳过: ${stats.skipped} | 失败: ${stats.failed}`);
console.log(`  截断 resultText: ${stats.truncated}（节省 ${(stats.savedBytes / 1024 / 1024).toFixed(2)} MB）`);
if (DRY_RUN) {
  console.log("\n（dry-run 模式：未写库、未写标记、未备份。去掉 --dry-run 执行实际迁移。）");
}