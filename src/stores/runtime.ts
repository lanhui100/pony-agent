import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import { useProviderStore } from "@/stores/providers";
import type {
  ChatMessage,
  HealthPayload,
  RuntimePhase,
  ToolActivity,
  TurnHistoryMessage,
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
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  messages: ChatMessage[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  activeTurnId: string | null;
  eventsReady: boolean;
};

const defaultToolActivities: ToolActivity[] = [
  {
    id: "tool-time-now",
    name: "time.now",
    status: "planned",
    summary: "返回当前本机 UNIX 时间戳。"
  },
  {
    id: "tool-echo-input",
    name: "echo.input",
    status: "planned",
    summary: "把传入 text 原样返回，用于验证 tool roundtrip。"
  },
  {
    id: "tool-workspace-read-file",
    name: "workspace.read_file",
    status: "planned",
    summary: "读取当前工作区内的文本文件预览。"
  },
  {
    id: "tool-workspace-read-file-segment",
    name: "workspace.read_file_segment",
    status: "planned",
    summary: "按行读取文件的一段内容，更适合大文件局部查看。"
  },
  {
    id: "tool-workspace-list-files",
    name: "workspace.list_files",
    status: "planned",
    summary: "列出当前工作区目录下的文件和子目录。"
  }
];

function createDefaultToolActivities(): ToolActivity[] {
  return defaultToolActivities.map((tool) => ({ ...tool }));
}

const defaultTraceSteps: TraceStep[] = [
  { id: "step-plan", label: "接收输入", state: "completed" },
  { id: "step-context", label: "组织上下文", state: "active" },
  { id: "step-call-model", label: "调用模型", state: "pending" },
  { id: "step-call-tool", label: "调用工具", state: "pending" },
  { id: "step-return", label: "返回结果", state: "pending" }
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

function buildTurnHistory(messages: ChatMessage[]): TurnHistoryMessage[] {
  return messages
    .filter((message) => message.status !== "pending" && message.content.trim().length > 0)
    .slice(-8)
    .map((message) => ({
      role: message.role,
      content: message.content
    }));
}

export const useRuntimeStore = defineStore("runtime", {
  state: (): RuntimeState => ({
    phase: "idle",
    health: null,
    error: null,
    draftMessage: "",
    sessionSummary: "",
    providerRequestedName: "",
    providerName: "",
    providerProtocol: "",
    providerModel: "",
    providerMode: "",
    fallbackReason: null,
    inputTokens: null,
    outputTokens: null,
    totalTokens: null,
    firstTokenLatencyMs: null,
    isSubmitting: false,
    activeTurnId: null,
    eventsReady: false,
    messages: [],
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
          step.id === "step-context"
            ? { ...step, state: "completed" }
            : step.id === "step-return"
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
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
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

        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
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
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
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
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
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
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
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
        { id: "step-call-tool", label: "调用工具", state: "completed" },
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
        modelId: providerStore.currentModel?.id ?? null,
        history: buildTurnHistory(this.messages)
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
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.traceSteps = [
        { id: "step-plan", label: "接收输入", state: "completed" },
        { id: "step-context", label: "组织上下文", state: "completed" },
        { id: "step-call-model", label: "调用模型", state: "active" },
        { id: "step-call-tool", label: "调用工具", state: "pending" },
        { id: "step-return", label: "返回结果", state: "pending" }
      ];
      this.toolActivities = [
        {
          id: "tool-time-now",
          name: "time.now",
          status: "planned",
          summary: "当前回合尚未触发时间工具。"
        },
        {
          id: "tool-echo-input",
          name: "echo.input",
          status: "planned",
          summary: "当前回合正在等待模型规划阶段。"
        },
        {
          id: "tool-workspace-read-file",
          name: "workspace.read_file",
          status: "planned",
          summary: "当前回合尚未触发文件读取工具。"
        },
        {
          id: "tool-workspace-read-file-segment",
          name: "workspace.read_file_segment",
          status: "planned",
          summary: "当前回合尚未触发文件分段读取工具。"
        },
        {
          id: "tool-workspace-list-files",
          name: "workspace.list_files",
          status: "planned",
          summary: "当前回合尚未触发目录列举工具。"
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
          { id: "step-call-tool", label: "调用工具", state: "completed" },
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
