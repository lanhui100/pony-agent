/**
 * B011：Run 工具结果的「可呈现元信息」最小切片。
 *
 * 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §波次四 · B-2 → B011
 * （promote 结果根级 `timedOut/promoted/jobId` 在 UI 侧没有消费方，界面会按 `status` 把它
 * 判成「成功完成」）。本模块只负责**纯解析**，不改变任何状态机 / 轨迹语义。
 *
 * 设计约束（契约）：
 * 1. **绝不 throw**：非 JSON、空串、`null`/`undefined`、非对象 JSON、数组一律 fail-safe
 *    回落为 `{ promoted: false, timedOut: false }`（宁可少显示，也不能让一条历史工具结果炸掉渲染）。
 * 2. 只读**载荷根级**字段（`promoted` / `timedOut` / `jobId`），不做嵌套推断。
 * 3. 只有布尔 `true` 才算真（字符串 `"true"` 不算），避免脏数据把普通结果误标成后台作业。
 */

export type PromotedToolResultMeta = {
  /** 结果是否由 Run 到期 promote 成后台作业产生。 */
  promoted: boolean;
  /** 结果是否到期（promote 结果与 promote 关闭时的 timeout 错误都会为 true）。 */
  timedOut: boolean;
  /** promote 成功时的后台作业 id；非 promote 或字段非字符串时缺省。 */
  jobId?: string;
};

/** fail-safe 基线：非 promote / 非超时。 */
const NOT_PROMOTED: PromotedToolResultMeta = { promoted: false, timedOut: false };

/**
 * 把工具结果文本解析成可呈现元信息。
 *
 * @param text 工具结果文本（后端 `ToolResult.output`，通常是 JSON 字符串）
 */
export function parsePromotedToolResult(text?: string | null): PromotedToolResultMeta {
  if (typeof text !== "string" || text.trim().length === 0) {
    return NOT_PROMOTED;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return NOT_PROMOTED;
  }

  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return NOT_PROMOTED;
  }

  const payload = parsed as Record<string, unknown>;
  const promoted = payload.promoted === true;
  const timedOut = payload.timedOut === true;
  const rawJobId = payload.jobId;
  const jobId =
    typeof rawJobId === "string" && rawJobId.trim().length > 0 ? rawJobId.trim() : undefined;

  return jobId ? { promoted, timedOut, jobId } : { promoted, timedOut };
}

/**
 * promote 结果的徽标文案（中文短标签）。
 *
 * @returns promoted 时为含「后台」的标签；否则 `null`（调用方据此不渲染徽标）。
 */
export function promotedToolBadgeLabel(meta: PromotedToolResultMeta): string | null {
  return meta.promoted ? "已转后台" : null;
}
