import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import type {
  ChatMessage,
  HealthPayload,
  RuntimePhase,
  ToolActivity,
  TraceStep,
  TurnInput,
  TurnResult
} from "../types/runtime";

type RuntimeState = {
  phase: RuntimePhase;
  health: HealthPayload | null;
  error: string | null;
  draftMessage: string;
  sessionSummary: string;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerMode: string;
  fallbackReason: string | null;
  isSubmitting: boolean;
  messages: ChatMessage[];
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
    draftMessage: "帮我解释一下这一轮 run_turn() 做了什么。",
    sessionSummary: "",
    providerRequestedName: "",
    providerName: "",
    providerProtocol: "",
    providerModel: "",
    providerMode: "",
    fallbackReason: null,
    isSubmitting: false,
    messages: [
      {
        id: "msg-user-seed",
        role: "user",
        content: "帮我搭一个适合调试 Rust agent runtime 的工作台。"
      },
      {
        id: "msg-agent-seed",
        role: "assistant",
        content: "当前界面已经接上 Vue + Pinia，接下来开始把真实模型调用和诊断信息接进 run_turn()。"
      }
    ],
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
    setDraftMessage(message: string) {
      this.draftMessage = message;
    },
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
    },
    async submitTurn() {
      const payload: TurnInput = { message: this.draftMessage.trim() };
      if (!payload.message) {
        return;
      }

      this.isSubmitting = true;
      this.error = null;
      this.phase = "calling_model";

      try {
        const result = await invoke<TurnResult>("run_turn", { input: payload });

        this.messages.push({
          id: `user-${Date.now()}`,
          role: "user",
          content: result.userMessage
        });
        this.messages.push({
          id: `assistant-${Date.now() + 1}`,
          role: "assistant",
          content: result.assistantMessage
        });

        this.phase = result.phase;
        this.traceSteps = result.traceSteps;
        this.toolActivities = result.toolActivities;
        this.sessionSummary = result.sessionSummary;
        this.providerRequestedName = result.providerRequestedName;
        this.providerName = result.providerName;
        this.providerProtocol = result.providerProtocol;
        this.providerModel = result.providerModel;
        this.providerMode = result.providerMode;
        this.fallbackReason = result.fallbackReason ?? null;
        this.draftMessage = "";
      } catch (error) {
        this.error = `本轮执行失败：${String(error)}`;
        this.phase = "failed";
      } finally {
        this.isSubmitting = false;
      }
    }
  }
});
