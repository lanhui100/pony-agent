#!/usr/bin/env node
/**
 * PA-089 阶段 6：切换脚本（6a freeze / 6b retire / rollback）。
 *
 * 6a（freeze）：设置 phase=frozen + 切换点 backup + 建 tombstone 表（防旧版重建 sessions）
 * 6b（retire）：设置 phase=retired + 删 blob 字段（可选，需 --delete-blob）+ 最终 backup
 * rollback：设置 phase 回 legacy + 删 tombstone
 *
 * 注意：
 * - 6a 是**运行时切换点**——执行后应用重启将走规范化读取（写仍双写）
 * - 6b 的 --delete-blob 是**不可逆操作**（删旧 sessions 表数据），执行前必须验证 backup
 * - 全部操作要求 Pony Agent 已退出
 *
 * 用法：
 *   node scripts/finalize-normalized.mjs --freeze        # 6a：frozen + tombstone + 切换点 backup
 *   node scripts/finalize-normalized.mjs --retire        # 6b：retired（保留 blob，观察后执行）
 *   node scripts/finalize-normalized.mjs --delete-blob   # 6b 删除 blob（不可逆，需先 --retire）
 *   node scripts/finalize-normalized.mjs --rollback      # 回滚到 legacy 读取
 *   node scripts/finalize-normalized.mjs --status        # 查看当前 phase
 */
import { DatabaseSync } from "node:sqlite";
import { existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { execSync } from "node:child_process";

const FREEZE = process.argv.includes("--freeze");
const RETIRE = process.argv.includes("--retire");
const DELETE_BLOB = process.argv.includes("--delete-blob");
const ROLLBACK = process.argv.includes("--rollback");
const STATUS = process.argv.includes("--status");

const dbPath = join(process.env.LOCALAPPDATA ?? "", "PonyAgent", "sessions.db");
if (!existsSync(dbPath)) {
  console.error(`sessions.db 不存在: ${dbPath}`);
  process.exit(1);
}

// ── 进程检测（fail-closed）────────────────────────────────
function detectRunningApp() {
  try {
    const out = execSync('tasklist /FI "IMAGENAME eq pony-agent.exe" /FO CSV /NH', {
      encoding: "utf8",
      timeout: 15000,
    });
    const pids = out
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean)
      .map((line) => line.match(/"pony-agent\.exe","(\d+)"/))
      .filter(Boolean)
      .map((m) => m[1]);
    return pids.length > 0 ? pids : null;
  } catch (err) {
    throw new Error(`无法检测 Pony Agent 进程状态（${err.message}），为安全起见中止`);
  }
}

function backup(db, label) {
  const backupDir = join(dirname(dbPath), "backups");
  mkdirSync(backupDir, { recursive: true });
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const backupPath = join(backupDir, `sessions.db.${stamp}.${label}.bak`);
  db.exec(`VACUUM INTO '${backupPath.replace(/'/g, "''")}'`);
  const integrity = db.prepare("PRAGMA integrity_check").get();
  console.log(`备份: ${backupPath}（integrity: ${integrity?.integrity_check ?? "?"}）`);
  return backupPath;
}

function getPhase(db) {
  const row = db.prepare("SELECT value FROM store_metadata WHERE key = 'storage.normalized.v1.phase'").get();
  return row?.value ?? "legacy";
}

function setPhase(db, phase) {
  db.prepare("INSERT OR REPLACE INTO store_metadata (key, value) VALUES ('storage.normalized.v1.phase', ?)").run(phase);
  console.log(`phase → ${phase}`);
}

// ── tombstone 表（防旧版 CREATE TABLE IF NOT EXISTS 重建 sessions）──
// 保留名为 sessions 的视图：旧版 CREATE TABLE IF NOT EXISTS sessions 会因
// 表已存在而跳过（不重建），新代码用 normalized 表不受影响。
// 视图加 INSTEAD OF trigger：INSERT/UPDATE/DELETE 转发到 session_blobs——
// 兼容所有写路径（save_store/upsert/remove 仍写 sessions 名）。
function installTombstone(db) {
  // 旧 sessions 表重命名为 session_blobs（保留数据），建 tombstone 视图 + 转发 trigger
  db.exec("ALTER TABLE sessions RENAME TO session_blobs");
  db.exec(`CREATE VIEW sessions AS SELECT conversation_id, title, updated_at_ms, session_data FROM session_blobs`);
  db.exec(`CREATE TRIGGER IF NOT EXISTS trg_sessions_insert INSTEAD OF INSERT ON sessions
    BEGIN
      INSERT OR REPLACE INTO session_blobs (conversation_id, title, updated_at_ms, session_data)
      VALUES (NEW.conversation_id, NEW.title, NEW.updated_at_ms, NEW.session_data);
    END;
    CREATE TRIGGER IF NOT EXISTS trg_sessions_update INSTEAD OF UPDATE ON sessions
    BEGIN
      INSERT OR REPLACE INTO session_blobs (conversation_id, title, updated_at_ms, session_data)
      VALUES (OLD.conversation_id, NEW.title, NEW.updated_at_ms, NEW.session_data);
    END;
    CREATE TRIGGER IF NOT EXISTS trg_sessions_delete INSTEAD OF DELETE ON sessions
    BEGIN
      DELETE FROM session_blobs WHERE conversation_id = OLD.conversation_id;
    END;`);
  console.log("tombstone 已装：sessions → session_blobs（视图 + INSTEAD OF trigger 转发读写）");
}

function removeTombstone(db) {
  db.exec("DROP TRIGGER IF EXISTS trg_sessions_insert");
  db.exec("DROP TRIGGER IF EXISTS trg_sessions_update");
  db.exec("DROP TRIGGER IF EXISTS trg_sessions_delete");
  db.exec("DROP VIEW IF EXISTS sessions");
  db.exec("ALTER TABLE session_blobs RENAME TO sessions");
  console.log("tombstone 已移除：session_blobs → sessions");
}

// ── 主流程 ─────────────────────────────────────────────────
const db = new DatabaseSync(dbPath);
db.exec("PRAGMA busy_timeout=5000");

if (STATUS) {
  console.log(`当前 phase: ${getPhase(db)}`);
  const sessionsTable = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='sessions'").get();
  const blobsTable = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='session_blobs'").get();
  console.log(`sessions 表: ${sessionsTable ? "存在" : "无"} | session_blobs 表: ${blobsTable ? "存在" : "无"}`);
  db.close();
  process.exit(0);
}

if (!FREEZE && !RETIRE && !DELETE_BLOB && !ROLLBACK) {
  console.error("未指定操作（--freeze / --retire / --delete-blob / --rollback / --status）");
  process.exit(1);
}

if (!DELETE_BLOB) {
  const pids = detectRunningApp();
  if (pids) {
    console.error(`Pony Agent 正在运行（PID: ${pids.join(", ")}），请先退出。`);
    process.exit(1);
  }
}

const phase = getPhase(db);

if (FREEZE) {
  if (phase === "frozen" || phase === "observing" || phase === "retired") {
    console.log(`当前 phase 已是 ${phase}，无需 freeze`);
    db.close();
    process.exit(0);
  }
  backup(db, "freeze-point");
  installTombstone(db);
  setPhase(db, "frozen");
  console.log("6a 完成：frozen + tombstone + 切换点 backup。应用重启后将走规范化读取（写仍双写）。");
} else if (RETIRE) {
  if (phase !== "frozen" && phase !== "observing") {
    console.error(`当前 phase 是 ${phase}，需先 --freeze`);
    db.close();
    process.exit(1);
  }
  setPhase(db, "retired");
  console.log("6b 完成：retired。应用将走规范化读取，blob 仍保留（可回滚）。");
} else if (DELETE_BLOB) {
  if (phase !== "retired") {
    console.error(`当前 phase 是 ${phase}，需先 --retire（观察窗口确认后）`);
    db.close();
    process.exit(1);
  }
  const backupPath = backup(db, "pre-delete-blob");
  console.log(`⚠️  删除 blob 前备份: ${backupPath}`);
  // 删除 blob 数据（session_blobs 表清空，保留结构供回滚）
  db.exec("DELETE FROM session_blobs");
  console.log("blob 数据已删除（session_blobs 表结构保留）。此操作不可逆——恢复需用备份。");
} else if (ROLLBACK) {
  if (phase === "legacy") {
    console.log("已在 legacy phase，无需回滚");
    db.close();
    process.exit(0);
  }
  removeTombstone(db);
  setPhase(db, "legacy");
  console.log("回滚完成：legacy 读取恢复（blob 数据需手动从备份恢复）。");
}

db.close();