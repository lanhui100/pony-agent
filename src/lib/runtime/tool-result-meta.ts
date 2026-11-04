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
 * 把工具结果文本解析成根级 JSON 对象；任何非法输入（非字符串/空白/非 JSON/非对象/数组）
 * 一律返回 `null`，**绝不 throw**（与 `parsePromotedToolResult` 同一容错口径）。
 */
function parseToolResultObject(text?: string | null): Record<string, unknown> | null {
  if (typeof text !== "string" || text.trim().length === 0) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    return null;
  }
  return parsed as Record<string, unknown>;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

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

/**
 * P1-2：工具退出码元信息（纯呈现层）。
 *
 * - `exitCode`：先读错误载荷 `error.exitCode`，缺失回落根级 `exitCode`（后端两种形态都出现过）；
 * - `exitCodeClass`：只读 `error.exitCodeClass`（P0-3 后端分类），缺失则**不写该键**
 *   （不伪造分类，避免成功结果被标成失败）。
 *
 * 非法输入 ⇒ `{}`（fail-safe，与 [`parsePromotedToolResult`] 同口径）。
 */
export type ToolExitMeta = {
  exitCode?: number | null;
  exitCodeClass?: string;
};

/** 读取某层的 `exitCode`：数字 ⇒ 原值；显式 `null` ⇒ `null`；缺失/非数字 ⇒ `undefined`。 */
function readExitCode(source: Record<string, unknown> | null): number | null | undefined {
  if (!source) {
    return undefined;
  }
  const value = source.exitCode;
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (value === null) {
    return null;
  }
  return undefined;
}

/**
 * 把工具结果文本解析成退出码元信息。
 *
 * @param text 工具结果文本（后端 `ToolResult.output`，通常是 JSON 字符串）
 */
export function parseToolExitMeta(text?: string | null): ToolExitMeta {
  const payload = parseToolResultObject(text);
  if (!payload) {
    return {};
  }

  const errorRecord = asRecord(payload.error);
  const fromError = readExitCode(errorRecord);
  const exitCode = fromError !== undefined ? fromError : readExitCode(payload);

  const meta: ToolExitMeta = {};
  if (exitCode !== undefined) {
    meta.exitCode = exitCode;
  }

  const rawClass = errorRecord?.exitCodeClass;
  if (typeof rawClass === "string" && rawClass.trim().length > 0) {
    meta.exitCodeClass = rawClass.trim();
  }

  return meta;
}

/**
 * 把退出码分类翻成中文短标签（仅对「可行动」的分类给出提示）。
 *
 * `general_failure` / 缺失 / 未知分类 ⇒ `null`（兜底分类不制造噪音）。
 */
export function exitCodeClassLabel(meta: ToolExitMeta): string | null {
  switch (meta.exitCodeClass) {
    case "compile_or_panic":
      return "编译/panic 失败";
    case "command_not_found":
      return "命令不存在";
    case "not_executable":
      return "不可执行";
    case "interrupted":
      return "被中断";
    case "killed":
      return "已终止";
    case "terminated":
      return "已终止";
    default:
      return null;
  }
}
