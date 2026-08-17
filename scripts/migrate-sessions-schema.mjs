#!/usr/bin/env node
/**
 * PA-089 阶段 2：规范化表回填脚本。
 *
 * 把存量会话（blob session_data + session_turn_traces 表）回填到 normalized_* 表。
 * 按 spec 3.6 定稿契约：
 * - 复合主键（session 前缀）
 * - trace 三源合并（表 ∪ 顶层 ∪ 节点，继承 PA-090）
 * - raw_json 逃生舱（往返无损）
 * - message_id 派生 + ordinal 全局重新编号
 * - turns 并集重建（消息 ∪ trace）
 * - checksum 版本化 SHA-256
 * - per-session 事务 + 幂等 marker + fail-closed
 *
 * 用法：
 *   node scripts/migrate-sessions-schema.mjs            # 实际执行
 *   node scripts/migrate-sessions-schema.mjs --dry-run  # 只报告不写
 *   node scripts/migrate-sessions-schema.mjs --force    # 强制重跑
 *
 * 前置条件：Pony Agent 必须已退出；normalized_* 表已由应用启动建好（阶段 1）。
 * 需要 Node >= 22（node:sqlite）。
 */
import { DatabaseSync } from "node:sqlite";
import { existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { execSync } from "node:child_process";
import { createHash } from "node:crypto";

const SPEC_VERSION = "pa089.v2";
const DRY_RUN = process.argv.includes("--dry-run");
const FORCE = process.argv.includes("--force");

const dbPath = join(process.env.LOCALAPPDATA ?? "", "PonyAgent", "sessions.db");
if (!existsSync(dbPath)) {
  console.error(`sessions.db 不存在: ${dbPath}`);
  process.exit(1);
}

// ── 0. 进程检测（fail-closed）──────────────────────────────
function detectRunningApp() {
  try {
    // 用 tasklist 检测（比 PowerShell 更可靠，无执行策略问题）
    const out = execSync('tasklist /FI "IMAGENAME eq pony-agent.exe" /FO CSV /NH', {
      encoding: "utf8",
      timeout: 15000,
    });
    const lines = out.split(/\r?\n/).map((s) => s.trim()).filter(Boolean);
    // CSV 格式: "pony-agent.exe","1234","Console","1","12,345 K"
    const pids = lines
      .map((line) => line.match(/"pony-agent\.exe","(\d+)"/))
      .filter(Boolean)
      .map((m) => m[1]);
    return pids.length > 0 ? pids : null;
  } catch (err) {
    throw new Error(`无法检测 Pony Agent 进程状态（${err.message}），为安全起见中止迁移`);
  }
}

// ── 1. 备份（VACUUM INTO）─────────────────────────────────
function backup(db, dbPath) {
  const backupDir = join(dirname(dbPath), "backups");
  mkdirSync(backupDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const backupPath = join(backupDir, `sessions.db.${stamp}.normalized.bak`);
  db.exec(`VACUUM INTO '${backupPath.replace(/'/g, "''")}'`);
  const integrity = db.prepare("PRAGMA integrity_check").get();
  console.log(`备份: ${backupPath}（integrity: ${integrity?.integrity_check ?? "?"}）`);
  return backupPath;
}

// ── 2. 工具 ────────────────────────────────────────────────
function sha256(text) {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

// 版本化 checksum：类型标签 + 长度前缀 + 值（防 NUL 歧义）
function checksumField(tag, value) {
  const bytes = Buffer.from(String(value), "utf8");
  return `${tag}:${bytes.length}:${bytes.toString("utf8")}`;
}

// trace 三源合并（继承 PA-090：表 > 顶层 > 节点，按 turn_id 去重取最新）
function mergeTraceSources(tableTraces, topLevelTraces, nodeTraces) {
  const byId = new Map();
  const ordered = [];
  const upsert = (trace, source) => {
    const key = trace.turnId ?? trace.turn_id;
    if (!key) throw new Error("trace 缺少 turnId");
    const updatedAt = trace.updatedAt ?? trace.updated_at ?? 0;
    const existing = byId.get(key);
    if (!existing) {
      byId.set(key, { trace, source, updatedAt });
      ordered.push(key);
    } else if (updatedAt > existing.updatedAt || (updatedAt === existing.updatedAt && source < existing.source)) {
      byId.set(key, { trace, source, updatedAt });
    }
  };
  for (const t of topLevelTraces) upsert(t, 1);
  for (const node of nodeTraces) for (const t of node) upsert(t, 2);
  for (const t of tableTraces) upsert(t, 0);
  return ordered.map((key) => byId.get(key).trace);
}

// ── 3. 主流程 ──────────────────────────────────────────────
const db = new DatabaseSync(dbPath);
db.exec("PRAGMA busy_timeout=5000");
db.exec("PRAGMA foreign_keys = ON");

const stats = { migrated: 0, skipped: 0, failed: 0 };

if (!DRY_RUN) {
  const pids = detectRunningApp();
  if (pids) {
    console.error(`Pony Agent 正在运行（PID: ${pids.join(", ")}），请先退出后再执行回填。`);
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

    // 幂等检查
    const markerRow = db
      .prepare("SELECT value FROM store_metadata WHERE key = ?")
      .get(`storage.normalized.v1:${sessionId}`);
    if (markerRow && !FORCE) {
      stats.skipped += 1;
      console.log(`[跳过] ${sessionId}（已有回填 marker）`);
      continue;
    }

    // ── 读取三源 ──
    const tableRows = db
      .prepare("SELECT turn_id, trace_data FROM session_turn_traces WHERE session_id = ?")
      .all(sessionId);
    const tableTraces = tableRows.map((r) => {
      const t = JSON.parse(r.trace_data);
      if (!t || typeof t !== "object") throw new Error(`表 trace 解析失败: ${r.turn_id}`);
      return t;
    });
    const topTraces = Array.isArray(parsed.turnTraceHistory) ? parsed.turnTraceHistory : [];
    const nodes = Array.isArray(parsed.historyNodes) ? parsed.historyNodes : [];
    const nodeTraces = nodes.map((n) => {
      if (!Array.isArray(n.turnTraceHistory)) return [];
      return n.turnTraceHistory.map((t) => {
        if (!t || typeof t !== "object") throw new Error(`节点 trace 解析失败: ${n.nodeId}`);
        return t;
      });
    });
    const union = mergeTraceSources(tableTraces, topTraces, nodeTraces);

    // ── 消息（message_id 派生 + ordinal 全局编号）──
    const history = Array.isArray(parsed.history) ? parsed.history : [];
    const messages = history.map((m, index) => {
      const turnId = m.turnId ?? null;
      const role = m.role ?? "unknown";
      const baseId = turnId ? `${turnId}-${role}` : `unknown-${role}`;
      // 同 (turn, role) 组内序号
      const sameGroup = history
        .slice(0, index)
        .filter((x) => (x.turnId ?? null) === turnId && (x.role ?? "unknown") === role).length;
      const messageId = sameGroup > 0 ? `${baseId}-${sameGroup + 1}` : baseId;
      return {
        messageId,
        turnId,
        ordinal: index,
        role,
        content: m.content ?? "",
        reasoningContent: m.reasoningContent ?? null,
        status: m.status ?? null,
        modelName: m.modelName ?? null,
        tokenCount: m.tokenCount ?? null,
        attachmentsJson: JSON.stringify(m.attachments ?? []),
        createdAtMs: null,
      };
    });

    // ── turns（消息 ∪ trace 并集）──
    const messageTurnIds = new Set(messages.map((m) => m.turnId).filter(Boolean));
    const traceTurnIds = new Set(union.map((t) => t.turnId ?? t.turn_id));
    const allTurnIds = [...new Set([...messageTurnIds, ...traceTurnIds])];
    // ordinal：消息 turn 按首个 user ordinal；trace-only turn 按 trace_order 追加
    const turns = [];
    let nextOrdinal = 0;
    for (const turnId of allTurnIds) {
      const turnMessages = messages.filter((m) => m.turnId === turnId);
      const firstUser = turnMessages.find((m) => m.role === "user");
      const trace = union.find((t) => (t.turnId ?? t.turn_id) === turnId);
      const ordinal = firstUser ? firstUser.ordinal : nextOrdinal;
      if (!firstUser) nextOrdinal += 1;
      turns.push({
        turnId,
        ordinal,
        phase: trace?.phase ?? null,
        status: firstUser?.status ?? null,
        userMessageId: turnMessages.find((m) => m.role === "user")?.messageId ?? null,
        assistantMessageId: turnMessages.find((m) => m.role === "assistant")?.messageId ?? null,
        startedAtMs: trace?.emittedAtMs ?? null,
        completedAtMs: trace?.updatedAt ?? null,
        createdAtMs: null,
        updatedAtMs: trace?.updatedAt ?? 0,
      });
    }

    // ── trace 子表 ──
    const traceRows = union.map((trace, index) => ({
      turnId: trace.turnId ?? trace.turn_id,
      traceOrder: index,
      phase: trace.phase ?? null,
      providerName: trace.providerName ?? null,
      providerModel: trace.providerModel ?? null,
      providerMode: trace.providerMode ?? null,
      sessionSummary: trace.sessionSummary ?? null,
      fallbackReason: trace.fallbackReason ?? null,
      error: trace.error ?? null,
      inputTokens: trace.inputTokens ?? null,
      outputTokens: trace.outputTokens ?? null,
      totalTokens: trace.totalTokens ?? null,
      firstTokenLatencyMs: trace.firstTokenLatencyMs ?? null,
      turnDurationMs: trace.turnDurationMs ?? null,
      updatedAtMs: trace.updatedAt ?? 0,
      extensionJson: JSON.stringify({
        v: 1,
        eventId: trace.eventId ?? null,
        eventType: trace.eventType ?? null,
        eventVersion: trace.eventVersion ?? null,
        sequence: trace.sequence ?? null,
        emittedAtMs: trace.emittedAtMs ?? null,
        title: trace.title ?? null,
        providerRequestedName: trace.providerRequestedName ?? null,
        providerProtocol: trace.providerProtocol ?? null,
        providerSource: trace.providerSource ?? null,
        cacheHitInputTokens: trace.cacheHitInputTokens ?? null,
        reasoningTokens: trace.reasoningTokens ?? null,
        buildContextObservation: trace.buildContextObservation ?? null,
        providerCallRecords: trace.providerCallRecords ?? [],
        hookTraceRecords: trace.hookTraceRecords ?? [],
      }),
      rawJson: JSON.stringify(trace),
      steps: (trace.traceSteps ?? []).map((s, i) => ({
        ordinal: i,
        kind: s.kind ?? null,
        state: s.state ?? null,
        label: s.label ?? null,
        text: s.text ?? null,
        error: s.error ?? null,
        durationMs: s.durationMs ?? null,
        extensionJson: JSON.stringify({ v: 1, id: s.id ?? null }),
        rawJson: JSON.stringify(s),
      })),
      timeline: (trace.traceTimeline ?? []).map((e) => ({
        entryId: e.id ?? `entry-${e.sequence ?? 0}`,
        sequence: e.sequence ?? 0,
        kind: e.kind ?? null,
        label: e.label ?? null,
        state: e.state ?? null,
        text: e.text ?? null,
        reasoningContent: e.reasoningContent ?? null,
        durationMs: e.durationMs ?? null,
        extensionJson: JSON.stringify({ v: 1 }),
        rawJson: JSON.stringify(e),
      })),
      activities: (trace.toolActivities ?? []).map((a) => ({
        activityId: a.id ?? `tool-${a.name ?? "unknown"}`,
        timelineEntryId: null,
        parentActivityId: a.parentActivityId ?? null,
        name: a.name ?? "unknown",
        canonicalToolName: a.canonicalToolName ?? null,
        status: a.status ?? "done",
        description: a.description ?? null,
        argumentsPreview: (a.argumentsText ?? "").slice(0, 32 * 1024),
        resultPreview: (a.resultText ?? "").slice(0, 32 * 1024),
        resultBytes: (a.resultText ?? "").length,
        resultTruncated: (a.resultText ?? "").length > 32 * 1024 ? 1 : 0,
        errorJson: a.error ? JSON.stringify(a.error) : null,
        durationSeconds: a.durationSeconds ?? null,
        createdAtMs: null,
        extensionJson: JSON.stringify({
          v: 1,
          displayNameZh: a.displayNameZh ?? null,
          artifacts: a.artifacts ?? [],
          capabilityInvocation: a.capabilityInvocation ?? null,
          argumentsText: a.argumentsText ?? null,
          resultText: a.resultText ?? null,
        }),
        rawJson: JSON.stringify(a),
      })),
    }));

    // ── history_nodes（snapshot_json 快照）──
    const historyNodes = nodes.map((n) => ({
      nodeId: n.nodeId,
      parentNodeId: n.parentNodeId ?? null,
      branchId: n.branchId ?? "branch-main",
      forkedFromNodeId: n.forkedFromNodeId ?? null,
      kind: n.kind ?? "checkpoint",
      turnId: n.turnId ?? null,
      turnTraceRefsJson: JSON.stringify(n.turnTraceRefs ?? []),
      runId: n.runId ?? null,
      workspaceRefJson: JSON.stringify(n.workspaceRef ?? null),
      summary: n.summary ?? "",
      title: n.title ?? "",
      createdAtMs: n.createdAtMs ?? 0,
      snapshotJson: JSON.stringify({
        v: 1,
        history: n.history ?? [],
        providerNativeTranscript: n.providerNativeTranscript ?? [],
        longTermMemoryEntries: n.longTermMemoryEntries ?? [],
        memoryWriteEvidence: n.memoryWriteEvidence ?? [],
        memoryWriteHookTraceRecords: n.memoryWriteHookTraceRecords ?? [],
        turnCount: n.turnCount ?? 0,
        lastReferencedFile: n.lastReferencedFile ?? null,
        turnTraceHistory: n.turnTraceHistory ?? [],
      }),
    }));

    // ── per-session 事务 ──
    if (DRY_RUN) {
      console.log(
        `[回填] ${sessionId}（messages ${messages.length} / turns ${turns.length} / traces ${union.length} / nodes ${historyNodes.length}）`
      );
      stats.migrated += 1;
      continue;
    }

    db.exec("BEGIN IMMEDIATE");
    try {
      // normalized_sessions
      db.prepare(
        `INSERT OR REPLACE INTO normalized_sessions
         (session_id, workspace_id, title, summary, turn_count, last_referenced_file, created_at_ms,
          updated_at_ms, state_version, trace_migration_state, turn_trace_refs_json,
          provider_native_transcript_json, history_state_evidence_json, memory_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      ).run(
        sessionId,
        parsed.workspaceId ?? null,
        parsed.title ?? "",
        parsed.summary ?? "",
        parsed.turnCount ?? 0,
        parsed.lastReferencedFile ?? null,
        null,
        parsed.updatedAtMs ?? 0,
        0,
        parsed.traceMigrationState ?? "legacy_blob",
        JSON.stringify(parsed.turnTraceRefs ?? []),
        JSON.stringify(parsed.providerNativeTranscript ?? []),
        JSON.stringify(parsed.historyStateEvidence ?? []),
        JSON.stringify({
          v: 1,
          longTermMemoryEntries: parsed.longTermMemoryEntries ?? [],
          memoryWriteEvidence: parsed.memoryWriteEvidence ?? [],
          memoryWriteHookTraceRecords: parsed.memoryWriteHookTraceRecords ?? [],
        })
      );

      // messages
      const insertMsg = db.prepare(
        `INSERT OR REPLACE INTO normalized_messages
         (session_id, message_id, turn_id, ordinal, role, content, reasoning_content, status,
          model_name, token_count, attachments_json, created_at_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      for (const m of messages) {
        insertMsg.run(
          sessionId, m.messageId, m.turnId, m.ordinal, m.role, m.content, m.reasoningContent,
          m.status, m.modelName, m.tokenCount, m.attachmentsJson, m.createdAtMs
        );
      }

      // turns
      const insertTurn = db.prepare(
        `INSERT OR REPLACE INTO normalized_turns
         (session_id, turn_id, ordinal, phase, status, user_message_id, assistant_message_id,
          started_at_ms, completed_at_ms, created_at_ms, updated_at_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      for (const t of turns) {
        insertTurn.run(
          sessionId, t.turnId, t.ordinal, t.phase, t.status, t.userMessageId, t.assistantMessageId,
          t.startedAtMs, t.completedAtMs, t.createdAtMs, t.updatedAtMs
        );
      }

      // turn_traces + 子表
      const insertTrace = db.prepare(
        `INSERT OR REPLACE INTO normalized_turn_traces
         (session_id, turn_id, trace_order, phase, provider_name, provider_model, provider_mode,
          session_summary, fallback_reason, error, input_tokens, output_tokens, total_tokens,
          first_token_latency_ms, turn_duration_ms, updated_at_ms, extension_json, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      const insertStep = db.prepare(
        `INSERT OR REPLACE INTO normalized_trace_steps
         (session_id, turn_id, ordinal, kind, state, label, text, error, duration_ms, extension_json, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      const insertTimeline = db.prepare(
        `INSERT OR REPLACE INTO normalized_trace_timeline
         (session_id, turn_id, entry_id, sequence, kind, label, state, text, reasoning_content, duration_ms, extension_json, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      const insertActivity = db.prepare(
        `INSERT OR REPLACE INTO normalized_tool_activities
         (session_id, turn_id, activity_id, timeline_entry_id, parent_activity_id, name, canonical_tool_name,
          status, description, arguments_preview, result_preview, result_bytes, result_truncated,
          error_json, duration_seconds, created_at_ms, extension_json, raw_json, timeline_variants_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      for (const tr of traceRows) {
        insertTrace.run(
          sessionId, tr.turnId, tr.traceOrder, tr.phase, tr.providerName, tr.providerModel, tr.providerMode,
          tr.sessionSummary, tr.fallbackReason, tr.error, tr.inputTokens, tr.outputTokens, tr.totalTokens,
          tr.firstTokenLatencyMs, tr.turnDurationMs, tr.updatedAtMs, tr.extensionJson, tr.rawJson
        );
        for (const s of tr.steps) {
          insertStep.run(sessionId, tr.turnId, s.ordinal, s.kind, s.state, s.label, s.text, s.error, s.durationMs, s.extensionJson, s.rawJson);
        }
        for (const e of tr.timeline) {
          insertTimeline.run(sessionId, tr.turnId, e.entryId, e.sequence, e.kind, e.label, e.state, e.text, e.reasoningContent, e.durationMs, e.extensionJson, e.rawJson);
        }
        for (const a of tr.activities) {
          insertActivity.run(
            sessionId, tr.turnId, a.activityId, a.timelineEntryId, a.parentActivityId, a.name, a.canonicalToolName,
            a.status, a.description, a.argumentsPreview, a.resultPreview, a.resultBytes, a.resultTruncated,
            a.errorJson, a.durationSeconds, a.createdAtMs, a.extensionJson, a.rawJson, JSON.stringify([])
          );
        }
      }

      // history_nodes / branches / cursor
      const insertNode = db.prepare(
        `INSERT OR REPLACE INTO normalized_history_nodes
         (session_id, node_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id,
          turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms, snapshot_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      for (const n of historyNodes) {
        insertNode.run(
          sessionId, n.nodeId, n.parentNodeId, n.branchId, n.forkedFromNodeId, n.kind, n.turnId,
          n.turnTraceRefsJson, n.runId, n.workspaceRefJson, n.summary, n.title, n.createdAtMs, n.snapshotJson
        );
      }
      const insertBranch = db.prepare(
        `INSERT OR REPLACE INTO normalized_history_branches
         (session_id, branch_id, base_node_id, head_node_id, forked_from_branch_id, forked_from_node_id, label, created_at_ms, updated_at_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`
      );
      for (const b of parsed.historyBranches ?? []) {
        insertBranch.run(
          sessionId, b.branchId, b.baseNodeId ?? null, b.headNodeId ?? null,
          b.forkedFromBranchId ?? null, b.forkedFromNodeId ?? null, b.label ?? "", b.createdAtMs ?? 0, b.updatedAtMs ?? 0
        );
      }
      const cursor = parsed.historyCursor ?? {};
      db.prepare(
        `INSERT OR REPLACE INTO normalized_history_cursor
         (session_id, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id, cursor_version, mode, checkout_mode, checkout_status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`
      ).run(
        sessionId, cursor.visibleNodeId ?? null, cursor.activeBranchId ?? null, cursor.branchHeadNodeId ?? null,
        cursor.workspaceNodeId ?? null, cursor.cursorVersion ?? 0, cursor.mode ?? "live",
        cursor.checkoutMode ?? null, cursor.checkoutStatus ?? null
      );

      // checksum（版本化 SHA-256，覆盖规范化表）
      const checksumParts = [];
      for (const table of [
        "normalized_sessions", "normalized_messages", "normalized_turns", "normalized_turn_traces",
        "normalized_trace_steps", "normalized_trace_timeline", "normalized_tool_activities",
        "normalized_history_nodes", "normalized_history_branches", "normalized_history_cursor",
      ]) {
        const rows = db.prepare(`SELECT * FROM "${table}" WHERE session_id = ? ORDER BY 1`).all(sessionId);
        for (const r of rows) {
          checksumParts.push(checksumField("row", JSON.stringify(r)));
        }
      }
      const checksum = `sha256:v1:${sha256(checksumParts.join("\n"))}`;

      // 幂等 marker
      db.prepare("INSERT OR REPLACE INTO store_metadata (key, value) VALUES (?, ?)").run(
        `storage.normalized.v1:${sessionId}`,
        JSON.stringify({
          spec_version: SPEC_VERSION,
          completed: true,
          checksum,
          migrated_at_ms: Date.now(),
        })
      );

      db.exec("COMMIT");
      stats.migrated += 1;
      console.log(`[回填] ${sessionId}（messages ${messages.length} / turns ${turns.length} / traces ${union.length} / nodes ${historyNodes.length}）`);
    } catch (err) {
      db.exec("ROLLBACK");
      throw err;
    }
  } catch (err) {
    stats.failed += 1;
    console.error(`[失败] ${sessionId}: ${err.message}`);
  }
}

if (!DRY_RUN) {
  try {
    db.exec("VACUUM");
    db.exec("PRAGMA wal_checkpoint(TRUNCATE)");
  } catch {
    console.warn("VACUUM/checkpoint 失败（不影响数据回填）");
  }
}
db.close();

console.log(`\n${DRY_RUN ? "[DRY-RUN] " : ""}回填结果:`);
console.log(`  回填: ${stats.migrated} | 跳过: ${stats.skipped} | 失败: ${stats.failed}`);
if (DRY_RUN) {
  console.log("\n（dry-run 模式：未写库、未写标记、未备份。去掉 --dry-run 执行实际回填。）");
}