#!/usr/bin/env node
/**
 * PA-089 阶段 4：影子校验脚本——对比 blob loader 与规范化 loader 产物。
 *
 * 校验维度（spec 3.5/7）：
 * - 消息：逐条对比（role/content/status/ordinal/turn_id）
 * - turns：turn_id 集合 + ordinal
 * - traces：turn_id 集合 + 顺序
 * - history_nodes：node_id 集合 + turn_id + refs
 * - cursor：visible/active/branch_head
 * - 元数据：title/summary/turn_count
 *
 * 输出：每个 session 的差异清单 + 首个差异定位。
 *
 * 用法：
 *   node scripts/verify-normalized-shadow.mjs            # 校验全部
 *   node scripts/verify-normalized-shadow.mjs <sessionId> # 校验单个
 */
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";

const dbPath = join(process.env.LOCALAPPDATA ?? "", "PonyAgent", "sessions.db");
const db = new DatabaseSync(dbPath, { readOnly: true });

const targetSession = process.argv[2] ?? null;

// ── blob loader 产物 ───────────────────────────────────────
function blobLoader(sessionId) {
  const row = db.prepare("SELECT session_data FROM sessions WHERE conversation_id = ?").get(sessionId);
  if (!row) return null;
  const d = JSON.parse(row.session_data);
  // blob 侧 trace：从旧 session_turn_traces 表读（Authoritative 会话 blob 已剥离 trace）
  const tableTraces = db
    .prepare("SELECT turn_id, trace_data FROM session_turn_traces WHERE session_id = ? ORDER BY trace_order")
    .all(sessionId);
  return {
    messages: (d.history ?? []).map((m, i) => ({
      ordinal: i,
      role: m.role ?? "unknown",
      content: m.content ?? "",
      turnId: m.turnId ?? null,
      status: m.status ?? null,
    })),
    traceTurnIds: tableTraces.map((t) => t.turn_id),
    nodeIds: (d.historyNodes ?? []).map((n) => n.nodeId).sort(),
    nodeTurnIds: (d.historyNodes ?? []).map((n) => n.turnId ?? null).sort(),
    cursor: {
      visibleNodeId: d.historyCursor?.visibleNodeId ?? null,
      activeBranchId: d.historyCursor?.activeBranchId ?? null,
      branchHeadNodeId: d.historyCursor?.branchHeadNodeId ?? null,
    },
    title: d.title ?? "",
    summary: d.summary ?? "",
    turnCount: d.turnCount ?? 0,
  };
}

// ── 规范化 loader 产物 ────────────────────────────────────
function normalizedLoader(sessionId) {
  const s = db.prepare("SELECT * FROM normalized_sessions WHERE session_id = ?").get(sessionId);
  if (!s) return null;
  const messages = db
    .prepare("SELECT * FROM normalized_messages WHERE session_id = ? ORDER BY ordinal")
    .all(sessionId);
  const turns = db.prepare("SELECT * FROM normalized_turns WHERE session_id = ? ORDER BY ordinal").all(sessionId);
  const traces = db.prepare("SELECT * FROM normalized_turn_traces WHERE session_id = ? ORDER BY trace_order").all(sessionId);
  const nodes = db.prepare("SELECT * FROM normalized_history_nodes WHERE session_id = ?").all(sessionId);
  const cursor = db.prepare("SELECT * FROM normalized_history_cursor WHERE session_id = ?").get(sessionId);
  return {
    messages: messages.map((m) => ({
      ordinal: m.ordinal,
      role: m.role,
      content: m.content,
      turnId: m.turn_id,
      status: m.status ? JSON.parse(m.status) : null,
    })),
    traceTurnIds: traces.map((t) => t.turn_id),
    nodeIds: nodes.map((n) => n.node_id).sort(),
    nodeTurnIds: nodes.map((n) => n.turn_id).sort(),
    cursor: {
      visibleNodeId: cursor?.visible_node_id ?? null,
      activeBranchId: cursor?.active_branch_id ?? null,
      branchHeadNodeId: cursor?.branch_head_node_id ?? null,
    },
    title: s.title ?? "",
    summary: s.summary ?? "",
    turnCount: s.turn_count ?? 0,
  };
}

// ── 对比 ───────────────────────────────────────────────────
function compare(label, blobVal, normVal) {
  const blobJson = JSON.stringify(blobVal);
  const normJson = JSON.stringify(normVal);
  if (blobJson === normJson) return null;
  // 定位首个差异
  let firstDiff = -1;
  const maxLen = Math.max(blobJson.length, normJson.length);
  for (let i = 0; i < maxLen; i++) {
    if (blobJson[i] !== normJson[i]) {
      firstDiff = i;
      break;
    }
  }
  return {
    label,
    blob: blobJson.slice(Math.max(0, firstDiff - 40), firstDiff + 60),
    norm: normJson.slice(Math.max(0, firstDiff - 40), firstDiff + 60),
    at: firstDiff,
  };
}

// ── 主流程 ─────────────────────────────────────────────────
const sessions = db.prepare("SELECT conversation_id FROM sessions").all();
let totalDiffs = 0;
let checked = 0;

for (const { conversation_id: sessionId } of sessions) {
  if (targetSession && sessionId !== targetSession) continue;
  const blob = blobLoader(sessionId);
  const norm = normalizedLoader(sessionId);
  if (!blob || !norm) {
    console.log(`[跳过] ${sessionId}（blob 或规范化数据缺失）`);
    continue;
  }
  checked += 1;

  const diffs = [];
  const push = (label, b, n) => {
    const d = compare(label, b, n);
    if (d) diffs.push(d);
  };

  push("messages", blob.messages, norm.messages);
  push("traceTurnIds", blob.traceTurnIds, norm.traceTurnIds);
  push("nodeIds", blob.nodeIds, norm.nodeIds);
  push("nodeTurnIds", blob.nodeTurnIds, norm.nodeTurnIds);
  push("cursor", blob.cursor, norm.cursor);
  push("title", blob.title, norm.title);
  push("summary", blob.summary, norm.summary);
  push("turnCount", blob.turnCount, norm.turnCount);

  if (diffs.length === 0) {
    console.log(`[一致] ${sessionId}（messages ${blob.messages.length} / traces ${blob.traceTurnIds.length} / nodes ${blob.nodeIds.length}）`);
  } else {
    totalDiffs += diffs.length;
    console.log(`[差异] ${sessionId}（${diffs.length} 处）`);
    for (const d of diffs) {
      console.log(`  - ${d.label}: blob=...${d.blob}... norm=...${d.norm}...`);
    }
  }
}

console.log(`\n影子校验结果: ${checked} 会话检查, ${totalDiffs} 处差异`);
if (totalDiffs === 0) {
  console.log("✅ 全部一致（影子校验通过）");
} else {
  console.log("❌ 存在差异（需排查）");
}
db.close();