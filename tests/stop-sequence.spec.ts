// wave-60 C8（TS 侧）黑盒验收（独立测试者，红相先写）：双击 Esc 的纯判定状态机。
//
// 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C8：
// - 判定窗口 `stop_sequence_ms = 500`（**可注入**）；
// - 同一身份 = (sessionId, turnId, generation) 全等；
// - 第 1 次 press 只上膛（返回 false，不发停止请求）；窗口内第 2 次同身份 ⇒ true（**恰好一次**）；
// - 超窗 / 身份变化 / 会话切换（reset）⇒ 上膛失效；
// - 与 Rust `StopSequenceState::press` 判据逐条对齐（同判据表覆盖两端）。
//
// 纪律：真实断言、AAA、禁止 it.skip / 空断言 / catch 吞异常。
import { beforeEach, describe, expect, it } from "vitest";
import { STOP_SEQUENCE_MS, StopSequenceState, type StopIdentity } from "@/lib/runtime/stop-sequence";

const WINDOW_MS = 500;

function identity(sessionId: string, turnId: string, generation: number): StopIdentity {
  return { sessionId, turnId, generation };
}

describe("wave-60 C8 TS 双击 Esc 判定", () => {
  let state: StopSequenceState;

  beforeEach(() => {
    state = new StopSequenceState(WINDOW_MS);
  });

  it("c8_default_window_matches_nfr", () => {
    // Arrange / Act
    const defaultState = new StopSequenceState();

    // Assert：NFR stop_sequence_ms = 500（缺省值可注入，但不得漂移）
    expect(STOP_SEQUENCE_MS).toBe(500);
    expect(defaultState.intervalMs).toBe(500);
    expect(state.intervalMs).toBe(WINDOW_MS);
  });

  it("c8_first_press_only_arms", () => {
    // Arrange
    const id = identity("session-c8", "turn-c8", 1);

    // Act
    const triggered = state.press(id, 1_000);

    // Assert：第 1 次按键不得发停止请求
    expect(triggered).toBe(false);
  });

  it("c8_second_press_within_window_fires_exactly_once", () => {
    // Arrange
    const id = identity("session-c8", "turn-c8", 1);
    state.press(id, 1_000);

    // Act
    const second = state.press(id, 1_000 + WINDOW_MS - 1);
    const third = state.press(id, 1_000 + WINDOW_MS);

    // Assert
    expect(second).toBe(true);
    expect(third).toBe(false);
  });

  it("c8_press_outside_window_rearms_instead_of_firing", () => {
    // Arrange
    const id = identity("session-c8", "turn-c8", 1);
    state.press(id, 1_000);

    // Act
    const late = state.press(id, 1_000 + WINDOW_MS + 1);
    const withinNewWindow = state.press(id, 1_000 + WINDOW_MS + 2);

    // Assert
    expect(late).toBe(false);
    expect(withinNewWindow).toBe(true);
  });

  it("c8_identity_change_invalidates_armed_state", () => {
    // Arrange
    state.press(identity("session-c8", "turn-c8", 1), 1_000);

    // Act / Assert：turn 变化
    expect(state.press(identity("session-c8", "turn-c8-other", 1), 1_100)).toBe(false);

    // Act / Assert：session 变化
    state.reset();
    state.press(identity("session-c8", "turn-c8", 1), 1_000);
    expect(state.press(identity("session-c8-other", "turn-c8", 1), 1_100)).toBe(false);

    // Act / Assert：generation 变化
    state.reset();
    state.press(identity("session-c8", "turn-c8", 1), 1_000);
    expect(state.press(identity("session-c8", "turn-c8", 2), 1_100)).toBe(false);
  });

  it("c8_reset_clears_armed_state", () => {
    // Arrange
    const id = identity("session-c8", "turn-c8", 1);
    state.press(id, 1_000);

    // Act
    state.reset();
    const afterReset = state.press(id, 1_050);

    // Assert
    expect(afterReset).toBe(false);
  });

  it("c8_window_is_injectable", () => {
    // Arrange
    const id = identity("session-c8", "turn-c8", 1);
    const narrow = new StopSequenceState(40);
    const wide = new StopSequenceState(5_000);

    // Act / Assert：40ms 窗口下 50ms 间隔已超窗
    narrow.press(id, 1_000);
    expect(narrow.press(id, 1_050)).toBe(false);

    // Act / Assert：5000ms 窗口下 50ms 间隔仍命中
    wide.press(id, 1_000);
    expect(wide.press(id, 1_050)).toBe(true);
  });
});
