import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke, safeListen } from "@/lib/tauri";
import { useProviderStore } from "@/stores/providers";
import type {
  AvailableTool,
  ChatMessage,
  HealthPayload,
  RuntimePhase,
  SessionOverview,
  SessionSnapshot,
  ToolActivity,
  TraceStep,
  TurnHistoryMessage,
  TurnInput,
  TurnStreamEvent,
  TurnTraceRecord
} from "../types/runtime";

type RuntimeState = {
  sessionId: string;
  sessionList: SessionOverview[];
  phase: RuntimePhase;
  health: HealthPayload | null;
  error: string | null;
  draftMessage: string;
  sessionSummary: string;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
  isSubmitting: boolean;
  messages: ChatMessage[];
  availableTools: AvailableTool[];
  toolActivities: ToolActivity[];
  traceSteps: TraceStep[];
  turnTraceHistory: TurnTraceRecord[];
  activeTurnId: string | null;
  eventsReady: boolean;
};

type PersistedRuntimeState = {
  messages: ChatMessage[];
  turnTraceHistory: TurnTraceRecord[];
  sessionSummary: string;
  providerRequestedName: string;
  providerName: string;
  providerProtocol: string;
  providerModel: string;
  providerSource: string;
  providerMode: string;
  fallbackReason: string | null;
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  firstTokenLatencyMs: number | null;
};

const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
const DEFAULT_SESSION_ID = "local-dev-session";

type PersistedRuntimeCache = {
  sessions: Record<string, PersistedRuntimeState>;
};

function debugLog(event: string, payload?: Record<string, unknown>) {
  const message = {
    event,
    payload: payload ?? {},
    ts: new Date().toISOString()
  };
  console.info("[pony-agent][runtime]", message);
}

function toolStatusToMessageStatus(status: ToolActivity["status"]): ChatMessage["status"] {
  switch (status) {
    case "done":
      return "done";
    case "error":
      return "error";
    default:
      return "pending";
  }
}

function cloneTraceSteps(traceSteps?: TraceStep[] | null) {
  return (traceSteps ?? []).map((step) => ({ ...step }));
}

function cloneToolActivities(toolActivities?: ToolActivity[] | null) {
  return (toolActivities ?? []).map((tool) => ({ ...tool }));
}

function buildToolMessageDetail(tool: ToolActivity) {
  const blocks = [tool.summary.trim()];

  if (tool.argumentsText?.trim()) {
    blocks.push(`参数\n${tool.argumentsText.trim()}`);
  }

  if (tool.resultText?.trim()) {
    blocks.push(`结果\n${tool.resultText.trim()}`);
  }

  return blocks.filter(Boolean).join("\n");
}

function buildTurnTitle(message: string) {
  const compact = message.replace(/\s+/g, " ").trim();
  if (!compact) {
    return "空白输入";
  }

  return compact.length > 44 ? `${compact.slice(0, 44)}…` : compact;
}

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

void defaultToolActivities;

const defaultTraceSteps: TraceStep[] = [
  { id: "step-plan", label: "接收输入", state: "completed" },
  { id: "step-context", label: "组织上下文", state: "active" },
  { id: "step-call-model", label: "调用模型", state: "pending" },
  { id: "step-call-tool", label: "调用工具", state: "pending" },
  { id: "step-return", label: "返回结果", state: "pending" }
];

function createDefaultTraceSteps() {
  return cloneTraceSteps(defaultTraceSteps);
}

function wait(ms: number) {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function waitForNextPaint() {
  return new Promise<void>((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
}

function buildAssistantModelLabel(providerName?: string | null, modelName?: string | null) {
  const provider = providerName?.trim();
  const model = modelName?.trim();

  if (provider && model) {
    return `${provider}/${model}`;
  }

  return model || provider || null;
}

const defaultAvailableTools: AvailableTool[] = [
  {
    name: "time.now",
    description: "返回当前本机 UNIX 时间戳，适合最小时间查询与运行时校验。",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false
    }
  },
  {
    name: "echo.input",
    description: "把传入的 text 原样返回，适合验证工具调用链路与参数透传。",
    inputSchema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "需要原样回显的文本"
        }
      },
      required: ["text"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.read_file",
    description: "读取当前工作区内的文本文件全文预览，需要传入相对路径。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.read_file_segment",
    description: "按行读取当前工作区文件片段，更适合大文件局部排查与定点观察。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对文件路径"
        },
        startLine: {
          type: "integer",
          description: "从第几行开始读取，最小值为 1"
        },
        lineCount: {
          type: "integer",
          description: "读取多少行，默认 40"
        }
      },
      required: ["path"],
      additionalProperties: false
    }
  },
  {
    name: "workspace.list_files",
    description: "列出当前工作区目录中的文件与子目录，可指定相对路径和返回条数。",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "当前工作区内的相对目录路径，默认 ."
        },
        limit: {
          type: "integer",
          description: "最多返回多少个条目，默认 40"
        }
      },
      additionalProperties: false
    }
  }
];

function createAvailableTools() {
  return defaultAvailableTools.map((tool) => ({
    ...tool,
    inputSchema: {
      ...tool.inputSchema,
      properties: tool.inputSchema.properties ? { ...tool.inputSchema.properties } : {}
    }
  }));
}

function buildTurnHistory(messages: ChatMessage[]): TurnHistoryMessage[] {
  return messages
    .filter(
      (message) =>
        (message.role === "user" || message.role === "assistant") &&
        message.status !== "pending" &&
        message.content.trim().length > 0
    )
    .slice(-8)
    .map((message) => ({
      role: message.role === "user" ? "user" : "assistant",
      content: message.content
    }));
}

function loadPersistedRuntimeCache(): PersistedRuntimeCache {
  if (typeof window === "undefined") {
    return { sessions: {} };
  }

  try {
    const raw = window.localStorage.getItem(RUNTIME_STORAGE_KEY);
    if (!raw) {
      debugLog("restore:empty");
      return { sessions: {} };
    }

    const parsed = JSON.parse(raw) as PersistedRuntimeCache;
    debugLog("restore:ok", {
      sessions: Object.keys(parsed.sessions ?? {}).length
    });
    return {
      sessions: parsed.sessions ?? {}
    };
  } catch {
    debugLog("restore:error");
    return { sessions: {} };
  }
}

function loadPersistedRuntimeState(sessionId: string): PersistedRuntimeState | null {
  return loadPersistedRuntimeCache().sessions[sessionId] ?? null;
}

function persistSessionState(sessionId: string, payload: PersistedRuntimeState) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  cache.sessions[sessionId] = payload;
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function removePersistedSessionState(sessionId: string) {
  if (typeof window === "undefined") {
    return;
  }

  const cache = loadPersistedRuntimeCache();
  delete cache.sessions[sessionId];
  window.localStorage.setItem(RUNTIME_STORAGE_KEY, JSON.stringify(cache));
}

function createHistoryTurnId(index: number) {
  return `history-turn-${index + 1}`;
}

function hydrateMessagesFromHistory(history: TurnHistoryMessage[]): ChatMessage[] {
  const messages: ChatMessage[] = [];
  let currentTurnId: string | null = null;
  let turnIndex = 0;

  for (const item of history) {
    if (item.role === "user") {
      currentTurnId = createHistoryTurnId(turnIndex);
      turnIndex += 1;
      messages.push({
        id: `history-user-${turnIndex}`,
        turnId: currentTurnId,
        role: "user",
        content: item.content,
        status: "done",
        tokenCount: null
      });
      continue;
    }

    if (!currentTurnId) {
      currentTurnId = createHistoryTurnId(turnIndex);
      turnIndex += 1;
    }

    messages.push({
      id: `history-assistant-${turnIndex}`,
      turnId: currentTurnId,
      role: "assistant",
      content: item.content,
      status: "done",
      tokenCount: null
    });
    currentTurnId = null;
  }

  return messages;
}

export const useRuntimeStore = defineStore("runtime", {
  state: (): RuntimeState => {
    const persisted = loadPersistedRuntimeState(DEFAULT_SESSION_ID);

    return {
      sessionId: DEFAULT_SESSION_ID,
      sessionList: [],
      phase: persisted?.messages?.length ? "ready" : "idle",
      health: null,
      error: null,
      draftMessage: "",
      sessionSummary: persisted?.sessionSummary ?? "",
      providerRequestedName: persisted?.providerRequestedName ?? "",
      providerName: persisted?.providerName ?? "",
      providerProtocol: persisted?.providerProtocol ?? "",
      providerModel: persisted?.providerModel ?? "",
      providerSource: persisted?.providerSource ?? "",
      providerMode: persisted?.providerMode ?? "",
      fallbackReason: persisted?.fallbackReason ?? null,
      inputTokens: persisted?.inputTokens ?? null,
      outputTokens: persisted?.outputTokens ?? null,
      totalTokens: persisted?.totalTokens ?? null,
      firstTokenLatencyMs: persisted?.firstTokenLatencyMs ?? null,
      isSubmitting: false,
      activeTurnId: null,
      eventsReady: false,
      messages: persisted?.messages ?? [],
      availableTools: createAvailableTools(),
      toolActivities: [],
      traceSteps: createDefaultTraceSteps(),
      turnTraceHistory: persisted?.turnTraceHistory ?? []
    };
  },
  getters: {
    phaseLabel(state): string {
      const labels: Record<RuntimePhase, string> = {
        idle: "空闲",
        connecting: "连接中",
        ready: "已就绪",
        completed: "本轮完成",
        calling_model: "模型处理中",
        calling_tool: "工具处理中",
        failed: "失败"
      };

      return labels[state.phase];
    }
  },
  actions: {
    resetSessionRuntimeState() {
      this.phase = "idle";
      this.error = null;
      this.sessionSummary = "";
      this.providerRequestedName = "";
      this.providerName = "";
      this.providerProtocol = "";
      this.providerModel = "";
      this.providerSource = "";
      this.providerMode = "";
      this.fallbackReason = null;
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.messages = [];
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      this.turnTraceHistory = [];
    },
    persistHistory() {
      const payload: PersistedRuntimeState = {
        messages: this.messages,
        turnTraceHistory: this.turnTraceHistory,
        sessionSummary: this.sessionSummary,
        providerRequestedName: this.providerRequestedName,
        providerName: this.providerName,
        providerProtocol: this.providerProtocol,
        providerModel: this.providerModel,
        providerSource: this.providerSource,
        providerMode: this.providerMode,
        fallbackReason: this.fallbackReason,
        inputTokens: this.inputTokens,
        outputTokens: this.outputTokens,
        totalTokens: this.totalTokens,
        firstTokenLatencyMs: this.firstTokenLatencyMs
      };

      try {
        persistSessionState(this.sessionId, payload);
        debugLog("persist", {
          sessionId: this.sessionId,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length,
          phase: this.phase
        });
      } catch {
        // Ignore storage failures and keep runtime in memory.
        debugLog("persist:error");
      }
    },
    async loadSessionCatalog() {
      if (isTauriAvailable()) {
        this.sessionList = await safeInvoke<SessionOverview[]>("list_sessions");
        return;
      }

      const cache = loadPersistedRuntimeCache();
      this.sessionList = Object.entries(cache.sessions)
        .map(([conversationId, state]) => ({
          conversationId,
          summary: state.sessionSummary || "浏览器预览会话",
          turnCount: state.messages.filter((message) => message.role === "user").length,
          lastReferencedFile: null,
          updatedAtMs:
            state.turnTraceHistory.length > 0
              ? state.turnTraceHistory[state.turnTraceHistory.length - 1].updatedAt
              : 0
        }))
        .sort((left, right) => right.updatedAtMs - left.updatedAtMs);
    },
    applySessionSnapshot(sessionId: string, snapshot: SessionSnapshot) {
      const persisted = loadPersistedRuntimeState(sessionId);

      this.sessionId = sessionId;
      this.error = null;
      this.isSubmitting = false;
      this.activeTurnId = null;
      this.sessionSummary = snapshot.summary;
      this.messages = persisted?.messages?.length ? persisted.messages : hydrateMessagesFromHistory(snapshot.history);
      this.turnTraceHistory = persisted?.turnTraceHistory ?? [];
      this.providerRequestedName = persisted?.providerRequestedName ?? "";
      this.providerName = persisted?.providerName ?? "";
      this.providerProtocol = persisted?.providerProtocol ?? "";
      this.providerModel = persisted?.providerModel ?? "";
      this.providerSource = persisted?.providerSource ?? "";
      this.providerMode = persisted?.providerMode ?? "";
      this.fallbackReason = persisted?.fallbackReason ?? null;
      this.inputTokens = persisted?.inputTokens ?? null;
      this.outputTokens = persisted?.outputTokens ?? null;
      this.totalTokens = persisted?.totalTokens ?? null;
      this.firstTokenLatencyMs = persisted?.firstTokenLatencyMs ?? null;
      this.toolActivities = [];
      this.traceSteps = createDefaultTraceSteps();
      this.phase = this.messages.length ? "ready" : "idle";
      this.persistHistory();
    },
    async switchSession(nextSessionId: string) {
      if (this.isSubmitting) {
        return;
      }

      if (isTauriAvailable()) {
        const snapshot = await safeInvoke<SessionSnapshot>("load_session_snapshot", {
          sessionId: nextSessionId
        });
        this.applySessionSnapshot(nextSessionId, snapshot);
        await this.loadSessionCatalog();
        return;
      }

      const persisted = loadPersistedRuntimeState(nextSessionId);
      this.applySessionSnapshot(nextSessionId, {
        conversationId: nextSessionId,
        summary: persisted?.sessionSummary ?? "浏览器预览会话",
        history: buildTurnHistory(persisted?.messages ?? []),
        turnCount: persisted?.messages?.filter((message) => message.role === "user").length ?? 0,
        lastReferencedFile: null,
        updatedAtMs:
          persisted && persisted.turnTraceHistory.length > 0
            ? persisted.turnTraceHistory[persisted.turnTraceHistory.length - 1].updatedAt
            : Date.now()
      });
      await this.loadSessionCatalog();
    },
    async createSession() {
      const nextSessionId = `session-${Date.now()}`;
      await this.switchSession(nextSessionId);
    },
    async deleteSession(targetSessionId: string) {
      if (this.isSubmitting) {
        return;
      }

      removePersistedSessionState(targetSessionId);

      if (isTauriAvailable()) {
        this.sessionList = await safeInvoke<SessionOverview[]>("delete_session", {
          sessionId: targetSessionId
        });
      } else {
        await this.loadSessionCatalog();
      }

      const fallbackSessionId =
        this.sessionList.find((session) => session.conversationId !== targetSessionId)?.conversationId ??
        this.sessionList[0]?.conversationId ??
        DEFAULT_SESSION_ID;

      if (this.sessionId === targetSessionId || !this.sessionList.some((session) => session.conversationId === this.sessionId)) {
        await this.switchSession(fallbackSessionId);
        return;
      }

      await this.loadSessionCatalog();
    },
    async initializeSessions() {
      await this.loadSessionCatalog();
      const preferredSessionId = this.sessionList[0]?.conversationId ?? this.sessionId;
      await this.switchSession(preferredSessionId);
    },
    upsertTurnTrace(
      turnId: string,
      patch: Partial<Omit<TurnTraceRecord, "turnId" | "updatedAt">> & { updatedAt?: number }
    ) {
      const existing = this.turnTraceHistory.find((item) => item.turnId === turnId);
      const updatedAt = patch.updatedAt ?? Date.now();

      if (existing) {
        Object.assign(existing, patch, { updatedAt });
        this.persistHistory();
        return;
      }

      this.turnTraceHistory.push({
        turnId,
        title: patch.title ?? "未命名轮次",
        phase: patch.phase ?? this.phase,
        traceSteps: cloneTraceSteps(patch.traceSteps),
        toolActivities: cloneToolActivities(patch.toolActivities),
        providerRequestedName: patch.providerRequestedName ?? null,
        providerName: patch.providerName ?? null,
        providerProtocol: patch.providerProtocol ?? null,
        providerModel: patch.providerModel ?? null,
        providerSource: patch.providerSource ?? null,
        providerMode: patch.providerMode ?? null,
        sessionSummary: patch.sessionSummary ?? "",
        fallbackReason: patch.fallbackReason ?? null,
        error: patch.error ?? null,
        inputTokens: patch.inputTokens ?? null,
        outputTokens: patch.outputTokens ?? null,
        totalTokens: patch.totalTokens ?? null,
        firstTokenLatencyMs: patch.firstTokenLatencyMs ?? null,
        updatedAt
      });
      this.persistHistory();
    },
    applyTurnTokenStats(turnId: string, inputTokens?: number | null, outputTokens?: number | null) {
      const userMessage = this.messages.find((item) => item.turnId === turnId && item.role === "user");
      const assistantMessage = this.messages.find((item) => item.turnId === turnId && item.role === "assistant");

      if (userMessage && inputTokens != null) {
        userMessage.tokenCount = inputTokens;
      }

      if (assistantMessage && outputTokens != null) {
        assistantMessage.tokenCount = outputTokens;
      }

      this.persistHistory();
    },
    ensureAssistantMessage(turnId: string, modelName?: string | null) {
      const messageId = `assistant-${turnId}`;
      const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "assistant");

      if (existingMessage) {
        if (modelName?.trim()) {
          existingMessage.modelName = modelName;
        }
        return existingMessage;
      }

      const assistantMessage: ChatMessage = {
        id: messageId,
        turnId,
        role: "assistant",
        content: "正在思考...",
        status: "pending",
        tokenCount: null,
        modelName: modelName?.trim() || undefined
      };

      this.messages.push(assistantMessage);
      this.persistHistory();
      return assistantMessage;
    },
    syncToolMessages(turnId: string, toolActivities?: ToolActivity[] | null) {
      if (!toolActivities) {
        return;
      }

      const activeTools = toolActivities.filter((tool) => tool.status !== "planned");
      for (const tool of activeTools) {
        const messageId = `tool-${turnId}-${tool.id}`;
        const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "tool");
        const nextContent = tool.resultText ?? "";
        const nextDetail = buildToolMessageDetail(tool);

        if (existingMessage) {
          existingMessage.content = nextContent;
          existingMessage.status = toolStatusToMessageStatus(tool.status);
          existingMessage.toolName = tool.name;
          existingMessage.detail = nextDetail;
          existingMessage.durationSeconds = tool.durationSeconds ?? null;
          this.persistHistory();
          continue;
        }

        this.messages.push({
          id: messageId,
          turnId,
          role: "tool",
          content: nextContent,
          status: toolStatusToMessageStatus(tool.status),
          toolName: tool.name,
          detail: nextDetail,
          durationSeconds: tool.durationSeconds ?? null
        });
        this.persistHistory();
      }
    },
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
        this.phase = "completed";
        debugLog("health:ok", {
          runtime: payload.runtime,
          graph: payload.graphEngine
        });
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
        debugLog("health:error", {
          error: String(error)
        });
      }
    },
    async fetchAvailableTools() {
      if (this.availableTools.length > 0 && isTauriAvailable()) {
        const hasDefaultOnly = this.availableTools.every((tool, index) => tool.name === defaultAvailableTools[index]?.name);
        if (!hasDefaultOnly) {
          return;
        }
      }

      try {
        this.availableTools = isTauriAvailable()
          ? await safeInvoke<AvailableTool[]>("list_available_tools")
          : createAvailableTools();
        debugLog("tools:ok", {
          count: this.availableTools.length
        });
      } catch (error) {
        this.availableTools = createAvailableTools();
        debugLog("tools:error", {
          error: String(error)
        });
      }
    },
    async initializeTurnEvents() {
      if (this.eventsReady) {
        return;
      }

      if (!isTauriAvailable()) {
        this.eventsReady = true;
        debugLog("events:browser-preview");
        return;
      }

      debugLog("events:init");

      const startedUnlisten = await safeListen<TurnStreamEvent>("turn:started", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );
        this.phase = "calling_model";
        debugLog("event:started", {
          turnId: payload.turnId
        });
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.upsertTurnTrace(payload.turnId, {
          phase: this.phase,
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs
        });
      });

      const deltaUnlisten = await safeListen<TurnStreamEvent>("turn:delta", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(this.providerName, this.providerModel)
        );

        const delta = payload.text ?? "";
        if (assistantMessage.status === "pending" && assistantMessage.content === "正在思考...") {
          assistantMessage.content = delta;
        } else {
          assistantMessage.content += delta;
        }

        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        this.persistHistory();
        debugLog("event:delta", {
          turnId: payload.turnId,
          deltaLength: delta.length
        });
        this.upsertTurnTrace(payload.turnId, {
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs
        });
      });

      const traceUnlisten = await safeListen<TurnStreamEvent>("turn:trace", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        debugLog("event:trace", {
          turnId: payload.turnId,
          steps: this.traceSteps.length
        });
        this.upsertTurnTrace(payload.turnId, {
          phase: (payload.phase as RuntimePhase | null) ?? this.phase,
          traceSteps: payload.traceSteps ?? this.traceSteps
        });
      });

      const toolUnlisten = await safeListen<TurnStreamEvent>("turn:tool", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        this.phase = "calling_tool";
        debugLog("event:tool", {
          turnId: payload.turnId,
          tools: (payload.toolActivities ?? []).length
        });
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.upsertTurnTrace(payload.turnId, {
          phase: "calling_tool",
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities
        });
      });

      const completedUnlisten = await safeListen<TurnStreamEvent>("turn:completed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        const finalText = payload.text?.trim();
        if (finalText) {
          assistantMessage.content = payload.text ?? assistantMessage.content;
        }
        assistantMessage.status = "done";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = "ready";
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.sessionSummary = payload.sessionSummary ?? this.sessionSummary;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? null;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens);
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.upsertTurnTrace(payload.turnId, {
          phase: "completed",
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          sessionSummary: payload.sessionSummary ?? this.sessionSummary,
          fallbackReason: payload.fallbackReason ?? null,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          error: null
        });
        this.persistHistory();
        void this.loadSessionCatalog();
        debugLog("event:completed", {
          turnId: payload.turnId,
          finalTextLength: payload.text?.length ?? 0,
          messages: this.messages.length,
          traces: this.turnTraceHistory.length
        });
        this.isSubmitting = false;
        this.activeTurnId = null;
      });

      const failedUnlisten = await safeListen<TurnStreamEvent>("turn:failed", ({ payload }) => {
        if (this.activeTurnId !== payload.turnId) {
          return;
        }

        const assistantMessage = this.ensureAssistantMessage(
          payload.turnId,
          buildAssistantModelLabel(payload.providerName, payload.providerModel)
        );

        assistantMessage.content = payload.text ?? "本轮执行失败，请查看右侧 trace。";
        assistantMessage.status = "error";
        assistantMessage.modelName = buildAssistantModelLabel(payload.providerName, payload.providerModel);

        this.phase = "failed";
        this.error = payload.error ?? "本轮执行失败。";
        this.traceSteps = payload.traceSteps ?? this.traceSteps;
        this.toolActivities = payload.toolActivities ?? this.toolActivities;
        this.providerRequestedName = payload.providerRequestedName ?? this.providerRequestedName;
        this.providerName = payload.providerName ?? this.providerName;
        this.providerProtocol = payload.providerProtocol ?? this.providerProtocol;
        this.providerModel = payload.providerModel ?? this.providerModel;
        this.providerSource = payload.providerSource ?? this.providerSource;
        this.providerMode = payload.providerMode ?? this.providerMode;
        this.fallbackReason = payload.fallbackReason ?? this.fallbackReason;
        this.inputTokens = payload.inputTokens ?? this.inputTokens;
        this.outputTokens = payload.outputTokens ?? this.outputTokens;
        this.totalTokens = payload.totalTokens ?? this.totalTokens;
        this.firstTokenLatencyMs = payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs;
        this.applyTurnTokenStats(payload.turnId, payload.inputTokens, payload.outputTokens);
        this.syncToolMessages(payload.turnId, payload.toolActivities);
        this.upsertTurnTrace(payload.turnId, {
          phase: "failed",
          traceSteps: payload.traceSteps ?? this.traceSteps,
          toolActivities: payload.toolActivities ?? this.toolActivities,
          providerRequestedName: payload.providerRequestedName ?? this.providerRequestedName,
          providerName: payload.providerName ?? this.providerName,
          providerProtocol: payload.providerProtocol ?? this.providerProtocol,
          providerModel: payload.providerModel ?? this.providerModel,
          providerSource: payload.providerSource ?? this.providerSource,
          providerMode: payload.providerMode ?? this.providerMode,
          fallbackReason: payload.fallbackReason ?? this.fallbackReason,
          inputTokens: payload.inputTokens ?? this.inputTokens,
          outputTokens: payload.outputTokens ?? this.outputTokens,
          totalTokens: payload.totalTokens ?? this.totalTokens,
          firstTokenLatencyMs: payload.firstTokenLatencyMs ?? this.firstTokenLatencyMs,
          error: payload.error ?? "本轮执行失败。"
        });
        this.persistHistory();
        debugLog("event:failed", {
          turnId: payload.turnId,
          error: this.error
        });
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
      this.providerRequestedName = provider?.name ?? "browser-preview";
      debugLog("browser-preview:start", {
        turnId: requestId
      });
      this.providerName = provider?.name ?? "browser-preview";
      this.providerProtocol = provider?.protocol ?? "openai";
      this.providerModel = model?.model ?? model?.name ?? "mock-stream";
      this.providerSource = "browser_preview";
      this.providerMode = "browser_preview";
      this.inputTokens = null;
      this.outputTokens = null;
      this.totalTokens = null;
      this.firstTokenLatencyMs = null;
      this.fallbackReason = "当前通过 npm run dev 打开的是浏览器预览，不是 Tauri 桌面窗口，因此不会接入 Rust 后端。";

      await wait(120);
      const assistantMessage = this.ensureAssistantMessage(
        requestId,
        buildAssistantModelLabel(provider?.name ?? "browser-preview", model?.model ?? model?.name ?? "mock-stream")
      );
      assistantMessage.content = "";

      const chunks = [
        "当前看到的不是前端资源没加载，而是页面运行在普通浏览器里。\n\n",
        "此时 `@tauri-apps/api` 不会注入原生桥接能力，所以直接调用 `invoke/listen` 会失败。\n\n",
        "现在已切换到浏览器预览兜底模式：\n",
        "- 可以继续预览 UI 和输入交互\n",
        "- 不会连接 Rust agent core\n",
        "- 真正联调需要运行 `tauri dev`\n"
      ];

      for (const chunk of chunks) {
        await wait(80);
        assistantMessage.content += chunk;
      }

      assistantMessage.status = "done";
      assistantMessage.modelName = buildAssistantModelLabel(
        provider?.name ?? "browser-preview",
        model?.model ?? model?.name ?? "mock-stream"
      );
      assistantMessage.tokenCount = null;

      this.phase = "completed";
      this.sessionSummary = "浏览器预览模式已启用，当前轮次未连接 Rust 后端。";
      this.traceSteps = [
        { id: "step-plan", label: "接收输入", state: "completed" },
        { id: "step-context", label: "识别运行环境", state: "completed" },
        { id: "step-call-model", label: "浏览器预览回放", state: "completed" },
        { id: "step-call-tool", label: "调用工具", state: "completed" },
        { id: "step-return", label: "返回结果", state: "completed" }
      ];
      this.toolActivities = [];
      this.traceSteps = this.traceSteps.map((step) =>
        step.id === "step-call-tool" ? { ...step, state: "pending" } : step
      );
      this.upsertTurnTrace(requestId, {
        phase: "completed",
        traceSteps: this.traceSteps,
        toolActivities: [],
        sessionSummary: this.sessionSummary,
        fallbackReason: this.fallbackReason,
        title: this.turnTraceHistory.find((item) => item.turnId === requestId)?.title ?? "浏览器预览",
        error: null
      });
      this.persistHistory();
      void this.loadSessionCatalog();
      debugLog("browser-preview:completed", {
        turnId: requestId
      });
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
        sessionId: this.sessionId,
        history: buildTurnHistory(this.messages)
      };

      if (!payload.message) {
        return;
      }

      const requestId = String(Date.now());
      const userMessageId = `user-${requestId}`;

      this.messages.push({
        id: userMessageId,
        turnId: requestId,
        role: "user",
        content: message,
        status: "done",
        tokenCount: null
      });
      this.persistHistory();
      debugLog("submit", {
        turnId: requestId,
        messageLength: message.length,
        messages: this.messages.length
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
          summary: "当前回合尚未触发分段读取工具。"
        },
        {
          id: "tool-workspace-list-files",
          name: "workspace.list_files",
          status: "planned",
          summary: "当前回合尚未触发目录枚举工具。"
        }
      ];
      this.toolActivities = [];
      this.upsertTurnTrace(requestId, {
        title: buildTurnTitle(message),
        phase: "calling_model",
        traceSteps: this.traceSteps,
        toolActivities: this.toolActivities,
        providerRequestedName: providerStore.currentProvider?.name ?? null,
        providerName: providerStore.currentProvider?.name ?? null,
        providerProtocol: providerStore.currentProvider?.protocol ?? null,
        providerModel: providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null,
        providerSource: isTauriAvailable() ? null : "browser_preview",
        providerMode: isTauriAvailable() ? null : "browser_preview",
        sessionSummary: "",
        fallbackReason: null,
        error: null,
        inputTokens: null,
        outputTokens: null,
        totalTokens: null,
        firstTokenLatencyMs: null
      });

      try {
        await waitForNextPaint();
        debugLog("submit:user-painted", {
          turnId: requestId,
          messages: this.messages.length
        });

        if (!isTauriAvailable()) {
          await this.runBrowserPreviewTurn(requestId);
          return;
        }

        await safeInvoke("start_turn_stream", { turnId: requestId, input: payload });
      } catch (error) {
        const assistantMessage = this.ensureAssistantMessage(
          requestId,
          buildAssistantModelLabel(
            providerStore.currentProvider?.name ?? null,
            providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
          )
        );
        assistantMessage.content = "本轮执行失败，请查看右侧 trace。";
        assistantMessage.status = "error";
        assistantMessage.modelName = buildAssistantModelLabel(
          providerStore.currentProvider?.name ?? null,
          providerStore.currentModel?.model ?? providerStore.currentModel?.name ?? null
        );
        this.error = `本轮执行失败：${String(error)}`;
        this.phase = "failed";
        this.activeTurnId = null;
        this.traceSteps = [
          { id: "step-plan", label: "接收输入", state: "completed" },
          { id: "step-context", label: "组织上下文", state: "completed" },
          { id: "step-call-model", label: "调用模型", state: "completed" },
          { id: "step-call-tool", label: "调用工具", state: "completed" },
          { id: "step-return", label: "返回结果", state: "error" }
        ];
        this.traceSteps = this.traceSteps.map((step) => {
          if (step.id === "step-call-model") {
            return { ...step, state: "error" };
          }

          if (step.id === "step-call-tool" || step.id === "step-return") {
            return { ...step, state: "pending" };
          }

          return step;
        });
        this.upsertTurnTrace(requestId, {
          phase: "failed",
          traceSteps: this.traceSteps,
          toolActivities: this.toolActivities,
          error: this.error
        });
        this.persistHistory();
      } finally {
        if (this.phase === "failed") {
          this.isSubmitting = false;
        }
      }
    }
  }
});
