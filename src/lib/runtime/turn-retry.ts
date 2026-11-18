/**
 * P2-7：失败轮的「可重试」判定与续跑文案（纯函数，不碰状态机、不发请求）。
 *
 * 背景（实测 pa_01a1e908 / pa_01a1e928）：运行时的本地兜底让「上游连续失败」的轮次在
 * 界面上与正常结束同形——用户只看到「说到一半停了」。后端已经把失败码钉进 turn_end
 * （`failure_code`，见 `agent/turn_event.rs`），本模块是**前端侧同一口径**的桥：
 * 只按稳定语义码判定可重试性，绝不让 UI 自己发明关键词表去猜上游。
 *
 * 语义（与后端 `retry::classify_error_class` + `turn_flow::failure_code_of` 对齐）：
 * - 可重试：`timeout` / `network` / `rate_limited` / `server_error`（瞬时故障，重试有意义）；
 * - 不可重试：`quota` / `auth` / `bad_request`（重试只会重复失败）与 `other`（未知即不承诺）。
 * - 畸形输入（`null` / 非字符串 / 空串）⇒ `classifyFailureText` 返回 `"other"`、提示返回 `null`，**绝不 throw**。
 */

/** 失败语义码（与后端 `classify_error_class` 的取值面逐项对齐）。 */
export type TurnFailureCode =
  | "timeout"
  | "network"
  | "rate_limited"
  | "quota"
  | "auth"
  | "bad_request"
  | "server_error"
  | "other";

/** 可重试的失败码（瞬时故障）。唯一真值源，UI 不得另立清单。 */
const RETRYABLE_CODES: ReadonlySet<TurnFailureCode> = new Set<TurnFailureCode>([
  "timeout",
  "network",
  "rate_limited",
  "server_error",
]);

/** 判据顺序即优先级：与后端码表同序（限流/配额先于超时，鉴权先于网络）。 */
const CODE_PATTERNS: ReadonlyArray<readonly [TurnFailureCode, RegExp]> = [
  ["quota", /\bquota\b|额度|配额|usage limit|allowance/i],
  ["rate_limited", /rate[_ -]?limit|429|\btoo many requests\b|限流/i],
  ["timeout", /\btimeout\b|timed out|operation timed out|超时/i],
  ["auth", /401|403|unauthor|forbidden|api[_ -]?key|鉴权/i],
  ["network", /type=connect|type=request|connection (reset|refused)|dns|network|SendRequest/i],
  ["bad_request", /400|404|413|422|bad request|invalid[_ -]?request/i],
  ["server_error", /50[0234]|52[0-7]|53[0]|server error|bad gateway|服务端/i],
];

/** 续跑时提交的用户消息（与用户手打「继续」逐字一致，不另造隐藏协议）。 */
export const RETRY_RESUME_PROMPT = "继续";

/** 失败轮「可重试」提示。 */
export interface TurnRetryHint {
  /** 失败码（稳定语义值）。 */
  code: TurnFailureCode;
  /** 是否值得重试。 */
  retryable: boolean;
  /** 状态行短标签，含契约要求的「可重试」字样。 */
  label: string;
  /** 悬浮说明（给出可行动的解释，不重复标签文本）。 */
  title: string;
}

/**
 * 失败文本 → 稳定失败码。
 *
 * @param text 失败原因原文（`turn:failed` 的 error / 历史 `errorDetail`）
 * @returns 语义码；无法判定 ⇒ `"other"`
 */
export function classifyFailureText(text: unknown): TurnFailureCode {
  if (typeof text !== "string" || !text.trim()) {
    return "other";
  }
  for (const [code, pattern] of CODE_PATTERNS) {
    if (pattern.test(text)) {
      return code;
    }
  }
  return "other";
}

/**
 * 失败轮的可重试提示。
 *
 * @param text 失败原因原文
 * @returns 可重试 ⇒ 带「上游失败，可重试」标签；不可重试 ⇒ `label` 指出不可重试原因，
 *          `retryable=false`；无失败原因 ⇒ `null`（调用方不渲染）。
 */
export function failedTurnRetryHint(text: unknown): TurnRetryHint | null {
  if (typeof text !== "string" || !text.trim()) {
    return null;
  }
  const code = classifyFailureText(text);
  if (RETRYABLE_CODES.has(code)) {
    return {
      code,
      retryable: true,
      label: "上游失败，可重试",
      title:
        code === "timeout"
          ? "上游响应超时（瞬时故障）：点此续跑，或直接输入「继续」"
          : code === "network"
            ? "与上游的连接中断（瞬时故障）：点此续跑，或直接输入「继续」"
            : code === "rate_limited"
              ? "上游限流：稍后点此续跑，或直接输入「继续」"
              : "上游服务端错误：点此续跑，或直接输入「继续」",
    };
  }
  return {
    code,
    retryable: false,
    label: "上游失败，不可重试",
    title:
      code === "quota"
        ? "上游额度耗尽：重试不会改善，请更换 provider 或等待配额恢复"
        : code === "auth"
          ? "上游鉴权失败：请检查 API Key / 权限配置"
          : code === "bad_request"
            ? "请求被上游拒绝：请检查模型与参数配置"
            : "失败原因无法归类：请查看 trace 详情",
  };
}
