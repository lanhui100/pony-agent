import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import { useProviderStore } from "@/stores/providers";
import type {
  ChatMessage,
  HealthPayload,
  RuntimePhase,
  ToolActivity,
  TraceStep,
  TurnInput,
  TurnStreamEvent
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
  activeTurnId: string | null;
  eventsReady: boolean;
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

function createDefaultToolActivities(): ToolActivity[] {
  return defaultToolActivities.map((tool) => ({ ...tool }));
}

const defaultTraceSteps: TraceStep[] = [
  { id: "step-1", label: "Plan", state: "completed" },
  { id: "step-2", label: "Reason", state: "active" },
  { id: "step-3", label: "CallTool", state: "pending" },
  { id: "step-4", label: "Observe", state: "pending" },
  { id: "step-5", label: "Done", state: "pending" }
];

function createDefaultTraceSteps(): TraceStep[] {
  return defaultTraceSteps.map((step) => ({ ...step }));
}

function wait(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function buildAssistantModelLabel(providerName?: string | null, modelName?: string | null) {
  const provider = providerName?.trim();
  const model = modelName?.trim();

  if (provider && model) {
    return `${provider}/${model}`;
  }

  return model || provider || null;
}

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
    activeTurnId: null,
    eventsReady: false,
    messages: [
      {
        id: "msg-user-seed",
        role: "user",
        content: "帮我搭一个适合调试 Rust agent runtime 的工作台。",
        status: "done"
      },
      {
        id: "msg-agent-seed",
        role: "assistant",
        content: "当前界面已经接上 Vue + Pinia，下一步开始把真实模型调用和诊断信息接进来。",
        status: "done"
      }
    ],
    toolActivities: createDefaultToolActivities(),
    traceSteps: createDefaultTraceSteps()
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
      if (this.health) {
        return;
      }

      this.phase = "connecting";
      this.error = null;

      try {
        const payload: HealthPayload = isTauriAvailable()
          ? await safeInvoke<HealthPayload>("health_check")
          : {
              appName: "Pony Agent",
              appVersion: "dev-preview",
              runtime: "browser-preview",
              graphEngine: "mock-stream"
            };
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
    async initializeTurnEvents() {
      if (this.eventsReady) {
        return;
      }

      if (!isTauriAvailable()) {
        this.eventsReady = true;
        return;
      }

      const startedUnlisten = await safeListen<TurnStreamEvent>("turn:started", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.phase = "calling_model";
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
      });

      const deltaUnlisten = await safeListen<TurnStreamEvent>("turn:delta", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.messages.find(
          (item) => item.id === `assistant-${payload.turnId}` && item.role === "assistant"
        );

        if (!assistantMessage) {
          return;
        }

        const delta = payload.text ?? "";
        if (assistantMessage.status === "pending" && assistantMessage.content === "正在思考...") {
          assistantMessage.content = delta;
        } else {
          assistantMessage.content += delta;
        }
      });

      const traceUnlisten = await safeListen<TurnStreamEvent>("turn:trace", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.traceSteps = payload.traceSteps ?? this.traceSteps;
      });

      const toolUnlisten = await safeListen<TurnStreamEvent>("turn:tool", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.toolActivities = payload.toolActivities ?? this.toolActivities;
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.messages.find(
          (item) => item.id === `assistant-${payload.turnId}` && item.role === "assistant"
        );

        if (assistantMessage) {
          assistantMessage.content = payload.text ?? assistantMessage.content;
          assistantMessage.status = "done";
          assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);
        }

        this.phase = "ready";
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.sessionSummary = payload.sessionSummary ?? this.sessionSummary;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? null;
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.messages.find(
          (item) => item.id === `assistant-${payload.turnId}` && item.role === "assistant"
        );

        if (assistantMessage) {
          assistantMessage.content = payload.text ?? "本轮执行失败，请查看右侧状态信息。";
          assistantMessage.status = "error";
          assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);
        }

        this.phase = "failed";
        this.error = payload.error ?? "本轮执行失败。";
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      void startedUnlisten;
      void deltaUnlisten;
      void traceUnlisten;
      void toolUnlisten;
      void completedUnlisten;
      void failedUnlisten;
      this.eventsReady = true;
    },
    async runBrowserPreviewTurn(requestId: string) {
      const providerStore = useProviderStore();
      const provider = providerStore.currentProvider;
      const model = providerStore.currentModel;
      const assistantMessage = this.messages.find(
        (item) => item.id === `assistant-${requestId}` && item.role === "assistant"
      );

      this.providerRequestedName = provider?.name ?? "browser-preview";
      this.providerName = provider?.name ?? "browser-preview";
      this.providerProtocol = provider?.protocol ?? "openai";
      this.providerModel = model?.model ?? model?.name ?? "mock-stream";
      this.providerMode = "browser_preview";
      this.fallbackReason = "当前通过 npm run dev 打开的是浏览器预览，不是 Tauri 窗口，因此不会连接 Rust 后端。";

      await wait(120);
      if (assistantMessage) {
        assistantMessage.content = "";
      }

      const chunks = [
        "当前看到的不是前端资源没加载，而是 dev 页面运行在普通浏览器里。",
        "这时 @tauri-apps/api 没有注入原生桥接能力，所以直接调用 invoke/listen 会失败。\n\n",
        "现在已经切到浏览器预览兜底模式：\n",
        "- 可以继续预览 UI 和输入交互\n",
        "- 不会连到 Rust agent core\n",
        "- 真正联调用 tauri dev\n\n",
        "如果你愿意，我下一步可以继续帮你把 tauri dev 的实际启动链路也一起验通。"
      ];

      for (const chunk of chunks) {
        await wait(80);
        if (assistantMessage) {
          assistantMessage.content += chunk;
        }
      }

      if (assistantMessage) {
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(
          provider?.name ?? "browser-preview",
          model?.model ?? model?.name ?? "mock-stream"
        );
      }

      this.phase = "ready";
      this.sessionSummary = "浏览器预览模式已启用，当前轮次未连接 Rust 后端。";
      this.traceSteps = [
        { id: "step-plan", label: "接收输入", state: "completed" },
        { id: "step-context", label: "识别运行环境", state: "completed" },
        { id: "step-call-model", label: "浏览器预览回放", state: "completed" },
        { id: "step-return", label: "返回结果", state: "completed" }
      ];
      this.toolActivities = [
        {
          id: "tool-browser-preview",
          name: "browser.preview",
          status: "done",
          summary: "当前轮次使用浏览器预览回放，没有触发 Rust 后端和真实 provider 调用。"
        }
      ];
      this.isSubmitting = false;
      this.activeTurnId = null;
    },
    async submitTurn() {
      await this.initializeTurnEvents();
      const providerStore = useProviderStore();
      const message = this.draftMessage.trim();
      const payload: TurnInput = {
        message,
        providerId: providerStore.currentProvider?.id ?? null,
        modelId: providerStore.currentModel?.id ?? null
      };

      if (!payload.message) {
        return;
      }

      const requestId = String(Date.now());
      const userMessageId = `user-${requestId}`;
      const assistantMessageId = `assistant-${requestId}`;

      this.messages.push({
        id: userMessageId,
        role: "user",
        content: message,
        status: "done"
      });
      this.messages.push({
        id: assistantMessageId,
        role: "assistant",
        content: "正在思考...",
        status: "pending",
        modelName: buildAssistantModelLabel(
          providerStore.currentProvider?.name ?? null,
          providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
        )
      });

      this.isSubmitting = true;
      this.error = null;
      this.phase = "calling_model";
      this.activeTurnId = requestId;
      this.draftMessage = "";
      this.traceSteps = [
        { id: "step-plan", label: "接收输入", state: "completed" },
        { id: "step-context", label: "组织上下文", state: "completed" },
        { id: "step-call-model", label: "调用模型", state: "active" },
        { id: "step-return", label: "返回结果", state: "pending" }
      ];
      this.toolActivities = [
        {
          id: "tool-terminal",
          name: "terminal.exec",
          status: "planned",
          summary: "当前回合尚未触发本地工具调用。"
        },
        {
          id: "tool-mcp",
          name: "mcp.call",
          status: "planned",
          summary: "当前回合正在等待模型响应，后续会在这里承接真实工具链事件。"
        }
      ];

      try {
        if (!isTauriAvailable()) {
          await this.runBrowserPreviewTurn(requestId);
          return;
        }

        await safeInvoke("start_turn_stream", { turnId: requestId, input: payload });
      } catch (error) {
        const assistantMessage = this.messages.find((item) => item.id === assistantMessageId);
        if (assistantMessage) {
          assistantMessage.content = "本轮执行失败，请查看右侧状态信息。";
          assistantMessage.status = "error";
          assistantMessage.modelName = buildAssistantModelLabel(
            providerStore.currentProvider?.name ?? null,
            providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
          );
        }
        this.error = `本轮执行失败：${String(error)}`;
        this.phase = "failed";
        this.activeTurnId = null;
        this.traceSteps = [
          { id: "step-plan", label: "接收输入", state: "completed" },
          { id: "step-context", label: "组织上下文", state: "completed" },
          { id: "step-call-model", label: "调用模型", state: "completed" },
          { id: "step-return", label: "返回结果", state: "completed" }
        ];
      } finally {
        if (this.phase === "failed") {
          this.isSubmitting = false;
        }
      }
    }
  }
});
