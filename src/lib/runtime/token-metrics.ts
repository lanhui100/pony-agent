/**
 * Wave-15 纯函数契约空桩（Protocol Agent 唯一编写方）。
 *
 * 契约：.dev-team/contracts/wave-15-token-metrics.md（S1 / S3 / S4 与 §7 契约演进 v2）。
 * 本文件当前只冻结导出签名，函数体一律为单行未实现哨兵抛错；
 * 实施由 Executor 在后续任务中完成，严禁在此处提前写入业务逻辑。
 * 参数名以 `_` 前缀规避 tsconfig `noUnusedParameters`（空桩期无读取），
 * 位置签名、导出名与返回类型为冻结契约。
 */
import type { ProviderCallCacheRecord, TurnTraceRecord } from "@/types/runtime";

/** S1 当前上下文输入：最近一次已完成模型调用的 `inputTokens`（禁止跨跳累加）。 */
export function resolveCurrentContextInputTokens(
  _turn: TurnTraceRecord | null | undefined
): number | null {
  throw new Error("NotImplemented");
}

/** S3 单次调用生成速度（t/s）：`outputTokens` 与本次调用耗时/TTFT 的纯函数解析。 */
export function resolveCallGenerationSpeed(
  _record: ProviderCallCacheRecord | null | undefined
): number | null {
  throw new Error("NotImplemented");
}

/** S4 判定用：轮内 provider call 数 = max(providerCallRecords.length, canonical call_model 条目数)。 */
export function countTurnProviderCalls(
  _turn: TurnTraceRecord | null | undefined
): number {
  throw new Error("NotImplemented");
}
