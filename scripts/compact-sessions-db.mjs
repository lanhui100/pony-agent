#!/usr/bin/env node
/**
 * 存量数据止血脚本：压缩 sessions.db 中过大的工具结果文本。
 *
 * 背景：TurnToolActivity.resultText 曾无大小约束，WebSearch 等工具把完整
 * 搜索结果写入 trace，导致单个 timeline 条目 222KB、会话 blob 46MB，
 * 前端加载会话时 JSON.parse 卡死主线程。
 *
 * 本脚本：
 *   1. 备份 sessions.db（含 -wal / -shm）
 *   2. 递归遍历每个 session 的 session_data 与 session_turn_traces.trace_data
 *   3. 截断所有 resultText / result_text 超过 32KB 的字段
 *   4. 写回并报告压缩量
 *
 * 用法：
 *   node scripts/compact-sessions-db.mjs            # 实际执行
 *   node scripts/compact-sessions-db.mjs --dry-run  # 只报告不写
 *
 * 注意：执行前请关闭 Pony Agent 应用（避免 SQLite 写锁冲突）。
 */
import { DatabaseSync } from "node:sqlite";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_RESULT_TEXT_CHARS = 32 * 1024;
const DRY_RUN = process.argv.includes("--dry-run");

const dbPath = join(process.env.LOCALAPPDATA, "PonyAgent", "sessions.db");
if (!existsSync(dbPath)) {
  console.error(`sessions.db 不存在: ${dbPath}`);
  process.exit(1);
}

// ── 1. 备份 ────────────────────────────────────────────────
const backupDir = join(dirname(dbPath), "backups");
mkdirSync(backupDir, { recursive: true });
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
for (const suffix of ["", "-wal", "-shm"]) {
  const src = dbPath + suffix;
  if (existsSync(src)) {
    const dst = join(backupDir, `sessions.db${suffix}.${stamp}.bak`);
    copyFileSync(src, dst);
    console.log(`备份: ${dst}`);
  }
}

// ── 2. 递归截断工具 ────────────────────────────────────────
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

// ── 3. 处理数据库 ──────────────────────────────────────────
const db = new DatabaseSync(dbPath);
const stats = { sessions: 0, traces: 0, truncated: 0, savedBytes: 0 };

const sessions = db.prepare("SELECT conversation_id, session_data FROM sessions").all();
for (const row of sessions) {
  let parsed;
  try {
    parsed = JSON.parse(row.session_data);
  } catch {
    console.warn(`跳过无法解析的 session: ${row.conversation_id}`);
    continue;
  }
  const before = JSON.stringify(parsed).length;
  truncateResultText(parsed, stats);
  const after = JSON.stringify(parsed).length;
  if (after !== before) {
    stats.sessions += 1;
    if (!DRY_RUN) {
      db.prepare("UPDATE sessions SET session_data = ? WHERE conversation_id = ?").run(
        JSON.stringify(parsed),
        row.conversation_id
      );
    }
  }
}

const traces = db.prepare("SELECT session_id, turn_id, trace_data FROM session_turn_traces").all();
for (const row of traces) {
  let parsed;
  try {
    parsed = JSON.parse(row.trace_data);
  } catch {
    continue;
  }
  const before = JSON.stringify(parsed).length;
  truncateResultText(parsed, stats);
  const after = JSON.stringify(parsed).length;
  if (after !== before) {
    stats.traces += 1;
    if (!DRY_RUN) {
      db.prepare(
        "UPDATE session_turn_traces SET trace_data = ? WHERE session_id = ? AND turn_id = ?"
      ).run(JSON.stringify(parsed), row.session_id, row.turn_id);
    }
  }
}

db.close();

// ── 4. 报告 ────────────────────────────────────────────────
console.log(`\n${DRY_RUN ? "[DRY-RUN] " : ""}压缩结果:`);
console.log(`  修改的 session 数: ${stats.sessions}`);
console.log(`  修改的 trace 数:   ${stats.traces}`);
console.log(`  截断的 resultText: ${stats.truncated}`);
console.log(`  预计节省:          ${(stats.savedBytes / 1024 / 1024).toFixed(2)} MB`);
if (DRY_RUN) {
  console.log("\n（dry-run 模式，未写库。去掉 --dry-run 执行实际压缩。）");
}