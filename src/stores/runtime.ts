import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import type { HealthPayload, RuntimePhase, ToolActivity, TraceStep } from "../types/runtime";

type RuntimeState = {
  phase: RuntimePhase;
  health: HealthPayload | null;
  error: string | null;
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
};

const defaultToolActivities: ToolActivity[] = [
  {
    id: "tool-1",
    name: "terminal.exec",
    status: "planned",
    summary: "预留给后续真实工具调用接入。"
  },
  {
    id: "tool-2",
    name: "mcp.call",
    status: "planned",
    summary: "后续统一承接外部 MCP 工具能力。"
  }
];

const defaultTraceSteps: TraceStep[] = [
  { id: "step-1", label: "Plan", state: "completed" },
  { id: "step-2", label: "Reason", state: "active" },
  { id: "step-3", label: "CallTool", state: "pending" },
  { id: "step-4", label: "Observe", state: "pending" },
  { id: "step-5", label: "Done", state: "pending" }
];

export const useRuntimeStore = defineStore("runtime", {
  state: (): RuntimeState => ({
    phase: "idle",
    health: null,
    error: null,
    toolActivities: defaultToolActivities,
    traceSteps: defaultTraceSteps
  }),
  getters: {
    phaseLabel(state): string {
      const labels: Record<RuntimePhase, string> = {
        idle: "空闲",
        connecting: "连接中",
        ready: "已就绪",
        calling_model: "模型调用中",
        calling_tool: "工具调用中",
        failed: "失败"
      };

      return labels[state.phase];
    }
  },
  actions: {
    async fetchHealth() {
      this.phase = "connecting";
      this.error = null;

      try {
        const payload = await invoke<HealthPayload>("health_check");
        this.health = payload;
        this.phase = "ready";
        this.traceSteps = this.traceSteps.map((step) =>
          step.id === "step-2"
            ? { ...step, state: "completed" }
            : step.id === "step-5"
              ? { ...step, state: "active" }
              : step
        );
      } catch (error) {
        this.error = `Rust 后端连接失败：${String(error)}`;
        this.phase = "failed";
      }
    }
  }
});
