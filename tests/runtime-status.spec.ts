import { describe, expect, it } from "vitest";
import { resolveAgentRuntimeStatus } from "@/lib/runtime/status";
import type { ToolActivity } from "@/types/runtime";

describe("resolveAgentRuntimeStatus", () => {
  it("returns null when session is idle and not submitting", () => {
    const status = resolveAgentRuntimeStatus({
      isSubmitting: false,
      phase: "idle",
      toolActivities: [],
      turnStartedAt: null,
      streamingReasoning: false,
    });
    expect(status).toBeNull();
  });

  it("identifies connecting phase", () => {
    const status = resolveAgentRuntimeStatus({
      isSubmitting: true,
      phase: "connecting",
      toolActivities: [],
      turnStartedAt: Date.now() - 1000,
      streamingReasoning: false,
    });
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("connecting");
    expect(status?.label).toBe("准备就绪中...");
  });

  it("identifies thinking phase when reasoning stream is active", () => {
    const status = resolveAgentRuntimeStatus({
      isSubmitting: true,
      phase: "calling_model",
      toolActivities: [],
      turnStartedAt: Date.now() - 3000,
      streamingReasoning: true,
    });
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("thinking");
    expect(status?.label).toBe("思考中...");
  });

  it("identifies responding / generating phase when calling model without reasoning", () => {
    const status = resolveAgentRuntimeStatus({
      isSubmitting: true,
      phase: "calling_model",
      toolActivities: [],
      turnStartedAt: Date.now() - 2000,
      streamingReasoning: false,
    });
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("responding");
    expect(status?.label).toBe("正在回复...");
  });

  it("identifies executing tool phase with tool name and description", () => {
    const runningTool: ToolActivity = {
      id: "tool-1",
      name: "bash",
      status: "running",
      description: "执行命令: cargo test",
    };
    const status = resolveAgentRuntimeStatus({
      isSubmitting: true,
      phase: "calling_tool",
      toolActivities: [runningTool],
      turnStartedAt: Date.now() - 5000,
      streamingReasoning: false,
    });
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("executing_tool");
    expect(status?.toolName).toBe("bash");
    expect(status?.label).toContain("执行工具");
    expect(status?.label).toContain("bash");
  });

  it("identifies waiting user approval phase", () => {
    const status = resolveAgentRuntimeStatus({
      isSubmitting: true,
      phase: "waiting_user",
      toolActivities: [],
      turnStartedAt: Date.now() - 8000,
      streamingReasoning: false,
    });
    expect(status).not.toBeNull();
    expect(status?.mode).toBe("waiting_approval");
    expect(status?.label).toBe("等待确认授权...");
  });
});
