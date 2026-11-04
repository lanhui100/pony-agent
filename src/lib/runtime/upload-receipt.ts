/**
 * P1-3：手动上传 trace 的**回执文案**（纯函数，不碰状态机、不发请求）。
 *
 * 契约真源：trace 报告 P1-3。后端 `upload_session_trace` 从「返回 turn 数」改为返回结构化回执
 * `{ turns, traceIds, failures }`（camelCase）。本模块把它翻成一句用户可读的中文反馈：
 *
 * 1. 全成功 ⇒ 含 turn 数、**每个** trace id 的前 8 位，并点明 Langfuse；
 * 2. 部分成功（有成功也有失败）⇒ 含「部分成功」与失败条数；
 * 3. 无可上传（三项皆空）⇒ 含「没有可上传」，且**不得**出现「成功」字样
 *    （否则用户会把「没东西可传」误读成「传成功了」）；
 * 4. 空/畸形输入 ⇒ 非空兜底文案，**绝不 throw**。
 *
 * 注意：这里的措辞不是「已成功上传至 Langfuse」的同义改写 —— 后端回执明确区分
 * 「完成 flush 的 turn 数」与「失败批次」，文案必须同构地反映这个事实。
 */

/** 后端回执形状（与服务端 `UploadTraceReceipt` 的 camelCase 序列化一一对应）。 */
export type UploadTraceReceipt = {
  turns?: number | null;
  traceIds?: string[] | null;
  failures?: string[] | null;
};

/** trace id 的短展示口径（与 UI 一致：前 8 位）。 */
const SHORT_ID_LENGTH = 8;

/** 无法解析回执时的兜底文案（非空、不误导为成功）。 */
const FALLBACK_TEXT = "上传结果未知：未能解析服务端回执，请查看日志后再试。";

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
}

function asCount(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

/** 把 trace id 列表渲染成「短 id（前 8 位）」的可读片段。 */
function formatTraceIds(traceIds: string[]): string {
  return traceIds
    .map((id) => id.trim().slice(0, SHORT_ID_LENGTH))
    .filter((id) => id.length > 0)
    .join("、");
}

/**
 * 把后端回执翻成一句用户可读的中文反馈。
 *
 * @param receipt 后端 `upload_session_trace` 的返回值（可能为 `null`/缺字段的畸形对象）
 */
export function formatUploadFeedback(receipt: UploadTraceReceipt | null | undefined): string {
  if (receipt === null || typeof receipt !== "object") {
    return FALLBACK_TEXT;
  }

  const turns = asCount(receipt.turns);
  const traceIds = asStringArray(receipt.traceIds);
  const failures = asStringArray(receipt.failures);

  // 无可上传：turn 数、trace id、失败原因三者皆空。
  if (turns === 0 && traceIds.length === 0 && failures.length === 0) {
    return "当前会话没有可上传的 Trace 数据。";
  }

  const shortIds = formatTraceIds(traceIds);

  // 部分成功：既有成功导出，也有失败批次 —— 必须同时说清两件事。
  if (failures.length > 0 && traceIds.length > 0) {
    return `部分成功：已上传 ${turns} 轮对话（trace: ${shortIds}）至 Langfuse；另有 ${failures.length} 条记录导出失败。`;
  }

  // 全部失败：不得说「成功」。
  if (failures.length > 0) {
    return `上传未完成：${failures.length} 条记录导出失败。`;
  }

  // 全成功：turn 数 + 每个 trace 的短 id + Langfuse。
  const idPart = shortIds.length > 0 ? `（trace: ${shortIds}）` : "";
  return `已成功上传 ${turns} 轮对话${idPart}至 Langfuse。`;
}
