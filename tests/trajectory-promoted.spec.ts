// B016 前端「时间线语义」红相：promote 结果必须在轨迹 cell 与 time-line hop 上**加性**可见。
//
// 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §B016（只做加性字段，**不动 state 枚举**，
// 避免牵动 V10 冻结快照）。
//
// 覆盖：
// 1. 轨迹层：`deriveTrajectoryRecords`（轨迹 cell 构建器）对 `toolActivities[].resultText` 为
//    promote 载荷的 tool ⇒ 该 cell 上 `promoted === true`；普通成功 tool ⇒ falsy。
// 2. 时间线层：`buildFallbackRuntimeTraceTimeline` 产出的 `call_tool` 条目同理 ⇒ `promoted === true`。
// 3. 状态语义不变：promoted 的 cell.status / entry.state 仍必须是 `completed`（不得引入新状态）。
//
// 命名说明（Lead 授权「先读代码确认导出名」）：
// - 轨迹构建器实际导出名为 `deriveTrajectoryRecords`（`src/lib/runtime/trajectory-core.ts`）；
// - 时间线构建器实际导出名为 `buildFallbackRuntimeTraceTimeline`（`src/lib/runtime/trace.ts`）。
//
// 红相预期：`promoted` 字段尚未实现 ⇒ 断言失败（字段为 undefined）。

import { describe, expect, it } from "vitest";
import { deriveTrajectoryRecords } from "@/lib/runtime/trajectory-core";
import { buildFallbackRuntimeTraceTimeline } from "@/lib/runtime/trace";
import type { ToolActivity, TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

/** promote 载荷（Run 到期转后台）：与真实 Run 结果同形。 */
const PROMOTE_RESULT_TEXT = JSON.stringify({
  ok: true,
  timedOut: true,
  promoted: true,
  jobId: "abc",
  command: "sleep 5"
});

/** 普通成功载荷（未超时）：不得被误标为 promoted。 */
const PLAIN_SUCCESS_TEXT = JSON.stringify({ ok: true, timedOut: false, exitCode: 0 });

/** 只读加性字段的窄视图（避免测试依赖实现内部类型定义）。 */
type PromotedView = { promoted?: boolean };

function toolActivity(overrides: Partial<ToolActivity> = {}): ToolActivity {
  return {
    id: "act-1",
    name: "workspace_run_command",
    status: "done",
    description: "执行命令 sleep 5",
    resultText: PLAIN_SUCCESS_TEXT,
    ...overrides
  };
}

/** 手搓一条 tool 时间线条目（隔离轨迹层，不依赖 trace.ts 的构建结果）。 */
function toolTimelineEntry(activity: ToolActivity): TraceTimelineEntry {
  return {
    id: "entry-tool-1",
    kind: "call_tool",
    label: "CALL TOOL #1",
    state: "completed",
    sequence: 2,
    toolActivities: [activity]
  };
}

/** 用最小 TurnTraceRecord 驱动轨迹构建器（只填被测路径用到的字段）。 */
function buildTrajectoryCell(activity: ToolActivity) {
  const turns = [
    {
      turnId: "turn-1",
      title: "Turn 1",
      phase: "completed",
      traceTimeline: [toolTimelineEntry(activity)]
    }
  ] as unknown as TurnTraceRecord[];
  return deriveTrajectoryRecords(turns)[0];
}

/** 用 fallback 时间线构建器产出 hop 条目（与运行时无 traceTimeline 时的真实兜底同路径）。 */
function buildTimelineEntries(activities: ToolActivity[]): TraceTimelineEntry[] {
  return buildFallbackRuntimeTraceTimeline({
    turnId: "turn-1",
    messages: [],
    phase: "completed",
    toolActivities: activities,
    providerPatch: {
      providerName: "test-provider",
      providerProtocol: "openai",
      providerModel: "test-model",
      providerSource: "config",
      providerMode: "chat"
    },
    terminalState: "completed"
  });
}

describe("deriveTrajectoryRecords - promote 标记（加性字段）", () => {
  it("promote 载荷的 tool cell 必须带 promoted=true", () => {
    // Arrange
    const activity = toolActivity({ resultText: PROMOTE_RESULT_TEXT });

    // Act
    const cell = buildTrajectoryCell(activity);

    // Assert
    expect(cell).toBeDefined();
    expect((cell as unknown as PromotedView).promoted).toBe(true);
  });

  it("普通成功 tool cell 的 promoted 必须为 falsy", () => {
    // Arrange
    const activity = toolActivity({ resultText: PLAIN_SUCCESS_TEXT });

    // Act
    const cell = buildTrajectoryCell(activity);

    // Assert
    expect(Boolean((cell as unknown as PromotedView).promoted)).toBe(false);
  });

  it("既有状态语义不变：promoted 的 cell.status 仍为 completed", () => {
    // Arrange
    const activity = toolActivity({ resultText: PROMOTE_RESULT_TEXT });

    // Act
    const cell = buildTrajectoryCell(activity);

    // Assert：B016 只加字段，不改 state 枚举（V10 冻结快照依赖 completed）。
    expect(cell.status).toBe("completed");
    expect(cell.isError).toBe(false);
  });
});

describe("buildFallbackRuntimeTraceTimeline - promote 标记（加性字段）", () => {
  it("promote 载荷的 call_tool hop 必须带 promoted=true", () => {
    // Arrange
    const activities = [toolActivity({ resultText: PROMOTE_RESULT_TEXT })];

    // Act
    const timeline = buildTimelineEntries(activities);
    const toolEntries = timeline.filter((entry) => entry.kind === "call_tool");

    // Assert
    expect(toolEntries.length).toBeGreaterThan(0);
    expect((toolEntries[0] as unknown as PromotedView).promoted).toBe(true);
  });

  it("普通成功 hop 的 promoted 必须为 falsy", () => {
    // Arrange
    const activities = [toolActivity({ resultText: PLAIN_SUCCESS_TEXT })];

    // Act
    const timeline = buildTimelineEntries(activities);
    const toolEntries = timeline.filter((entry) => entry.kind === "call_tool");

    // Assert
    expect(toolEntries.length).toBeGreaterThan(0);
    expect(Boolean((toolEntries[0] as unknown as PromotedView).promoted)).toBe(false);
  });

  it("既有状态语义不变：promoted hop 的 state 仍为 completed（不得引入新状态）", () => {
    // Arrange
    const activities = [toolActivity({ resultText: PROMOTE_RESULT_TEXT })];

    // Act
    const timeline = buildTimelineEntries(activities);
    const toolEntry = timeline.find((entry) => entry.kind === "call_tool");

    // Assert
    expect(toolEntry).toBeDefined();
    expect(toolEntry!.state).toBe("completed");
    expect(toolEntry!.error ?? null).toBeNull();
  });
});
