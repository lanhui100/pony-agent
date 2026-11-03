// wave-60 §C8：「双击 Esc」的**纯判定状态机**（桌面端实现，与 Rust
// `crates/pony-agent-core/src/agent/stop_sequence.rs` 的 `StopSequenceState` 判据逐条对齐）。
//
// 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C8。
// 判据（本模块只表达状态，**不**决定谁去发停止请求）：
// - 判定窗口内**同一身份** = (`sessionId`, `turnId`, `generation`) 全等；
// - 第 1 次 `press`（流式中）仅「上膛」⇒ 返回 `false`，**不**发停止请求；
// - 窗口内第 2 次同身份 `press` ⇒ 返回 `true`，调用方据此发**恰好一次**停止请求，随即复位；
// - 超出窗口 / 身份变化 / turn 结束 / 会话切换（`reset`）⇒ 上膛失效并重新上膛。
//
// 非目标：不读写 DOM、不持定时器、不产生副作用（`nowMs` 由调用方注入，便于单测）。

/** NFR `stop_sequence_ms`：双击 Esc 的判定窗口缺省值（毫秒）。 */
export const STOP_SEQUENCE_MS = 500;

/** 双 Esc 的身份三元组：任一字段变化即视为「身份变化 ⇒ 上膛失效」。 */
export interface StopIdentity {
  sessionId: string | null;
  turnId: string | null;
  generation: number;
}

function sameIdentity(left: StopIdentity, right: StopIdentity): boolean {
  return (
    left.sessionId === right.sessionId &&
    left.turnId === right.turnId &&
    left.generation === right.generation
  );
}

/**
 * 纯判定状态机（不持锁、不产生副作用；调用方负责在正确时机 `reset`）。
 *
 * 语义与 Rust 侧一一对应：命中即复位（保证「恰好一次」），超窗/换身份只重新上膛。
 */
export class StopSequenceState {
  private readonly windowMs: number;
  private armed: StopIdentity | null = null;
  private armedAtMs = 0;

  /**
   * @param intervalMs 判定窗口（`stop_sequence_ms`，NFR 缺省 500；测试可注入毫秒级小值）。
   */
  constructor(intervalMs: number = STOP_SEQUENCE_MS) {
    this.windowMs = intervalMs;
  }

  /** 当前生效的判定窗口（毫秒）。 */
  get intervalMs(): number {
    return this.windowMs;
  }

  /** 记一次按键。返回 `true` 表示「命中双击 ⇒ 应发恰好一次停止请求」。 */
  press(identity: StopIdentity, nowMs: number): boolean {
    const armed = this.armed;
    const hit =
      armed !== null &&
      sameIdentity(armed, identity) &&
      nowMs >= this.armedAtMs &&
      nowMs - this.armedAtMs < this.windowMs;

    if (hit) {
      this.armed = null;
      this.armedAtMs = 0;
      return true;
    }

    this.armed = { ...identity };
    this.armedAtMs = nowMs;
    return false;
  }

  /** 清空上膛状态（turn 结束 / 会话切换 / 退出流式态时由调用方触发）。 */
  reset(): void {
    this.armed = null;
    this.armedAtMs = 0;
  }
}
