import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ModelMonitorPage from "@/components/ModelMonitorPage.vue";
import type {
  CapabilitySourceView,
  CapabilityView,
  HookTraceRecord,
  ModelMonitorSessionDrilldownView,
  ModelMonitorSummaryView,
  SessionRuntimeView,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

function createTimelineEntry(partial: Partial<TraceTimelineEntry> = {}): TraceTimelineEntry {
  return {
    id: partial.id ?? "timeline-1",
    kind: partial.kind ?? "model",
    label: partial.label ?? "Call model",
    state: partial.state ?? "completed",
    sequence: partial.sequence ?? 1,
    providerRequestedName: partial.providerRequestedName ?? "test-openai",
    providerName: partial.providerName ?? "test-openai",
    providerProtocol: partial.providerProtocol ?? "openai",
    providerModel: partial.providerModel ?? "gpt-5.4",
    providerSource: partial.providerSource ?? "test",
    providerMode: partial.providerMode ?? "chat",
    buildContextObservation: partial.buildContextObservation ?? null,
    toolActivities: partial.toolActivities ?? [],
    text: partial.text ?? "模型返回正文",
    reasoningContent: partial.reasoningContent ?? null,
    fallbackReason: partial.fallbackReason ?? null,
    error: partial.error ?? null,
    inputTokens: partial.inputTokens ?? 11,
    cacheHitInputTokens: partial.cacheHitInputTokens ?? 3,
    reasoningTokens: partial.reasoningTokens ?? 0,
    outputTokens: partial.outputTokens ?? 7,
    totalTokens: partial.totalTokens ?? 18,
    firstTokenLatencyMs: partial.firstTokenLatencyMs ?? 220,
    turnDurationMs: partial.turnDurationMs ?? 850
  };
}

function createTrace(partial: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: partial.turnId ?? "turn-1",
    eventId: partial.eventId !== undefined ? partial.eventId : "turn-1:4",
    eventType: partial.eventType !== undefined ? partial.eventType : "turn.completed",
    eventVersion: partial.eventVersion !== undefined ? partial.eventVersion : "turn-event-v1",
    sequence: partial.sequence !== undefined ? partial.sequence : 4,
    emittedAtMs: partial.emittedAtMs !== undefined ? partial.emittedAtMs : 4004,
    title: partial.title ?? "首轮分析",
    phase: partial.phase ?? "completed",
    traceSteps: partial.traceSteps ?? [],
    traceTimeline: partial.traceTimeline ?? [createTimelineEntry()],
    toolActivities: partial.toolActivities ?? [],
    hookTraceRecords: partial.hookTraceRecords ?? [],
    providerRequestedName: partial.providerRequestedName ?? "test-openai",
    providerName: partial.providerName ?? "test-openai",
    providerProtocol: partial.providerProtocol ?? "openai",
    providerModel: partial.providerModel ?? "gpt-5.4",
    providerSource: partial.providerSource ?? "test",
    providerMode: partial.providerMode ?? "chat",
    buildContextObservation: partial.buildContextObservation ?? {
      requestFormat: "responses",
      messageCount: 4,
      imageCount: 0,
      toolCount: 2,
      temperature: 0.2,
      maxOutputTokens: 1024,
      stablePrefixText: "stable",
      semiStableContextText: "包含会话历史和摘要",
      volatileInputText: "当前问题",
      prefixMutationReasons: ["history_boundary_shifted"],
      requestMessagesText: "assembled request",
      toolDefinitionsText: "tool defs"
    },
    sessionSummary: partial.sessionSummary ?? "会话摘要",
    fallbackReason: partial.fallbackReason ?? null,
    error: partial.error ?? null,
    inputTokens: partial.inputTokens ?? 21,
    cacheHitInputTokens: partial.cacheHitInputTokens ?? 5,
    reasoningTokens: partial.reasoningTokens ?? 0,
    outputTokens: partial.outputTokens ?? 13,
    totalTokens: partial.totalTokens ?? 34,
    firstTokenLatencyMs: partial.firstTokenLatencyMs ?? 220,
    turnDurationMs: partial.turnDurationMs ?? 850
  };
}

function createHookTraceRecord(partial: Partial<HookTraceRecord> = {}): HookTraceRecord {
  return {
    hookName: partial.hookName ?? "audit.observe",
    hookClass: partial.hookClass ?? "observe",
    hookPoint: partial.hookPoint ?? "model_call_start",
    hookOrder: partial.hookOrder ?? 1,
    resultKind: partial.resultKind ?? "observe",
    structuredResult:
      partial.structuredResult ?? {
        resultKind: "observe",
        payload: {
          summary: "hook observed lifecycle boundary without mutation"
        }
      },
    blocked: partial.blocked ?? false,
    elapsedMs: partial.elapsedMs ?? 12,
    inputSummary: partial.inputSummary ?? "monitor",
    persistenceEvidenceRef: partial.persistenceEvidenceRef ?? null,
    summary: partial.summary ?? "observe hook summary"
  };
}

function createRuntimeView(traces: TurnTraceRecord[]): SessionRuntimeView {
  return {
    session: {
      conversationId: "session-alpha",
      title: "Alpha Session",
      summary: "监控摘要 alpha",
      history: [],
      attachmentAssets: [],
      providerNativeTranscript: [],
      turnTraceHistory: traces,
      longTermMemoryEntries: [],
      turnCount: traces.length,
      lastReferencedFile: null,
      updatedAtMs: 2000,
      historyNodes: [],
      historyBranches: [],
      historyCursor: {
        sessionId: "session-alpha",
        visibleNodeId: null,
        activeBranchId: "branch-main",
        branchHeadNodeId: null,
        workspaceNodeId: null,
        mode: "live"
      },
      resolvedNodeId: null,
      latestNodeId: null
    },
    retrieved: {
      turnContext: {
        userMessage: "当前问题",
        images: [],
        referencesImage: false
      },
      sessionContext: {
        conversationId: "session-alpha",
        title: "Alpha Session",
        summary: "监控摘要 alpha",
        recentHistory: [],
        recentAttachmentAssets: [],
        turnCount: traces.length,
        lastReferencedFile: null
      },
      runState: {
        runId: null,
        goal: null,
        phase: null,
        activeTurnId: null,
        lastCompletedTurnId: null,
        resumeCount: null,
        lastDecisionSummary: null,
        executionCheckpointStatus: null,
        executionCheckpointPhase: null
      },
      longTermMemory: {
        status: "empty",
        summary: "empty",
        entries: []
      },
      transcript: {
        providerNativeMessages: []
      }
    },
    checkpoint: null,
    historyNodes: [],
    historyBranches: [],
    historyCursor: {
      sessionId: "session-alpha",
      visibleNodeId: null,
      activeBranchId: "branch-main",
      branchHeadNodeId: null,
      workspaceNodeId: null,
      mode: "live"
    }
  };
}

function createSummaryView(): ModelMonitorSummaryView {
  return {
    overview: {
      sessionCount: 2,
      requestCount: 5,
      modelCallCount: 6,
      toolCallCount: 2,
      hookCallCount: 4,
      blockedHookCount: 1,
      failedRequestCount: 1,
      retrievalParticipationCount: 3,
      inputTokens: 111,
      cacheHitInputTokens: 24,
      outputTokens: 77,
      totalTokens: 188,
      avgFirstTokenLatencyMs: 240,
      avgTurnDurationMs: 980,
      avgHookDurationMs: 18,
      totalHookDurationMs: 72
    },
    providers: [
      {
        key: "test-openai",
        label: "test-openai",
        requestCount: 5,
        modelCallCount: 6,
        failedRequestCount: 1,
        retrievalParticipationCount: 3,
        inputTokens: 111,
        cacheHitInputTokens: 24,
        outputTokens: 77,
        totalTokens: 188,
        avgFirstTokenLatencyMs: 240,
        avgTurnDurationMs: 980
      }
    ],
    models: [
      {
        key: "gpt-5.4",
        label: "test-openai/gpt-5.4",
        requestCount: 5,
        modelCallCount: 6,
        failedRequestCount: 1,
        retrievalParticipationCount: 3,
        inputTokens: 111,
        cacheHitInputTokens: 24,
        outputTokens: 77,
        totalTokens: 188,
        avgFirstTokenLatencyMs: 240,
        avgTurnDurationMs: 980
      }
    ],
    tools: [
      {
        key: "search_docs",
        label: "search_docs",
        callCount: 2,
        failedCallCount: 0,
        avgDurationMs: 600,
        totalDurationMs: 1200
      }
    ],
    hookClasses: [
      {
        key: "observe",
        label: "observe",
        callCount: 3,
        blockedCallCount: 0,
        avgDurationMs: 11,
        totalDurationMs: 33
      },
      {
        key: "guard",
        label: "guard",
        callCount: 1,
        blockedCallCount: 1,
        avgDurationMs: 39,
        totalDurationMs: 39
      }
    ],
    hooks: [
      {
        key: "audit.observe",
        label: "audit.observe",
        callCount: 3,
        blockedCallCount: 0,
        avgDurationMs: 11,
        totalDurationMs: 33
      },
      {
        key: "guard.input",
        label: "guard.input",
        callCount: 1,
        blockedCallCount: 1,
        avgDurationMs: 39,
        totalDurationMs: 39
      }
    ],
    capabilitySources: [
      {
        key: "mcp-local",
        label: "mcp-local",
        callCount: 2,
        failedCallCount: 1,
        avgDurationMs: 250,
        totalDurationMs: 500
      }
    ],
    capabilityInvocationModes: [
      {
        key: "direct_tool_call",
        label: "direct_tool_call",
        callCount: 1,
        failedCallCount: 0,
        avgDurationMs: 200,
        totalDurationMs: 200
      },
      {
        key: "read_only_fetch",
        label: "read_only_fetch",
        callCount: 1,
        failedCallCount: 1,
        avgDurationMs: 300,
        totalDurationMs: 300
      }
    ],
    capabilityFailureClasses: [
      {
        key: "ok",
        label: "ok",
        callCount: 1,
        failedCallCount: 0,
        avgDurationMs: 200,
        totalDurationMs: 200
      },
      {
        key: "permission_denied",
        label: "permission_denied",
        callCount: 1,
        failedCallCount: 1,
        avgDurationMs: 300,
        totalDurationMs: 300
      }
    ],
    sessions: [
      {
        sessionId: "session-alpha",
        title: "Alpha Session",
        summary: "监控摘要 alpha",
        updatedAtMs: 2000,
        requestCount: 3,
        modelCallCount: 3,
        toolCallCount: 1,
        hookCallCount: 3,
        blockedHookCount: 1,
        failedRequestCount: 0,
        retrievalParticipationCount: 2,
        inputTokens: 66,
        cacheHitInputTokens: 12,
        outputTokens: 45,
        totalTokens: 111,
        avgFirstTokenLatencyMs: 220,
        avgTurnDurationMs: 900,
        avgHookDurationMs: 20,
        totalHookDurationMs: 60
      },
      {
        sessionId: "session-beta",
        title: "Beta Session",
        summary: "监控摘要 beta",
        updatedAtMs: 1000,
        requestCount: 2,
        modelCallCount: 3,
        toolCallCount: 1,
        hookCallCount: 1,
        blockedHookCount: 0,
        failedRequestCount: 1,
        retrievalParticipationCount: 1,
        inputTokens: 45,
        cacheHitInputTokens: 12,
        outputTokens: 32,
        totalTokens: 77,
        avgFirstTokenLatencyMs: 270,
        avgTurnDurationMs: 1060,
        avgHookDurationMs: 12,
        totalHookDurationMs: 12
      }
    ],
    generatedAtMs: 3000
  };
}

function createDrilldownView(sessionId = "session-alpha", title = "Alpha Session", summary = "监控摘要 alpha"): ModelMonitorSessionDrilldownView {
  const traces = [
    createTrace({
      turnId: "turn-2",
      title: "第二轮分析",
      totalTokens: 55,
      turnDurationMs: 1100,
      hookTraceRecords: [
        createHookTraceRecord(),
        createHookTraceRecord({
          hookName: "guard.input",
          hookClass: "guard",
          hookPoint: "context_build_start",
          hookOrder: 2,
          resultKind: "deny",
          structuredResult: {
            resultKind: "deny",
            payload: {
              reasonCode: "unsafe_input",
              message: "guard denied the input"
            }
          },
          blocked: true,
          elapsedMs: 39,
          inputSummary: "monitor-guard",
          summary: "guard hook blocked the turn"
        })
      ],
      traceTimeline: [
        createTimelineEntry({ id: "timeline-1", label: "Prepare retrieval", kind: "prepare_retrieval", text: null }),
        createTimelineEntry({ id: "timeline-2", label: "Return result", kind: "return_result" })
      ]
    }),
    createTrace({
      turnId: "turn-1",
      title: "第一轮分析"
    })
  ];

  return {
    sessionId,
    metrics: {
      sessionId,
      title,
      summary,
      updatedAtMs: 2000,
      requestCount: 2,
      modelCallCount: 2,
      toolCallCount: 1,
      hookCallCount: 3,
      blockedHookCount: 1,
      failedRequestCount: 0,
      retrievalParticipationCount: 2,
      inputTokens: 66,
      cacheHitInputTokens: 12,
      outputTokens: 45,
      totalTokens: 111,
      avgFirstTokenLatencyMs: 220,
      avgTurnDurationMs: 900,
      avgHookDurationMs: 20,
      totalHookDurationMs: 60
    },
    runtimeView: {
      ...createRuntimeView(traces),
      session: {
        ...createRuntimeView(traces).session,
        conversationId: sessionId,
        title,
        summary
      },
      retrieved: {
        ...createRuntimeView(traces).retrieved,
        sessionContext: {
          ...createRuntimeView(traces).retrieved.sessionContext,
          conversationId: sessionId,
          title,
          summary
        }
      },
      historyCursor: {
        ...createRuntimeView(traces).historyCursor!,
        sessionId
      }
    }
  };
}

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function createCapabilitySources(): CapabilitySourceView[] {
  return [
    {
      sourceId: "builtin-tools",
      sourceKind: "builtin",
      displayName: "Builtin Tools",
      transportKind: "in_process",
      serverIdentity: "pony-agent:builtin-tools",
      availability: "available",
      declaredCapabilities: ["tool"],
      permissionProfile: "host-mediated",
      updatedAtMs: 3000,
      lastIngressObservation: {
        boundary: "control_plane.apply_mcp_source_snapshot",
        summary: "builtin source ingress registered `builtin-tools` with 1 capability candidates",
        candidateIds: ["builtin:time_now"],
        observedAtMs: 3100
      }
    }
  ];
}

function createCapabilities(): CapabilityView[] {
  return [
    {
      capabilityId: "builtin:time_now",
      sourceId: "builtin-tools",
      sourceKind: "builtin",
      kind: "tool",
      label: "time_now",
      description: "返回当前本机 UNIX 时间戳",
      invocationMode: "direct_tool_call",
      inputSchemaSummary: "object",
      safetyClass: "host_tool",
      visibility: "default",
      observabilityTags: ["builtin", "tool"],
      requiresApproval: false,
      hostMediated: true,
      permissionScope: "workspace"
    }
  ];
}

function mockCapabilityCommand(command: string, args?: Record<string, unknown>) {
  if (command === "list_capability_sources") {
    return createCapabilitySources();
  }
  if (command === "list_capabilities") {
    expect(args).toEqual({
      sourceId: "builtin-tools",
      kind: null
    });
    return createCapabilities();
  }
  if (command === "inspect_capability") {
    return createCapabilities()[0];
  }
  return undefined;
}

describe("ModelMonitorPage", () => {
  beforeEach(() => {
    tauriMocks.mockSafeInvoke.mockReset();
    tauriMocks.mockIsTauriAvailable.mockReset();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("加载摘要并自动打开首个 session 下钻", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return capabilityPayload;
      }

      if (command === "load_model_monitor_summary") {
        return createSummaryView();
      }

      if (command === "load_model_monitor_session_drilldown") {
        return createDrilldownView();
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenNthCalledWith(1, "load_model_monitor_summary");
    expect(wrapper.get('[data-testid="model-monitor-overview"]').text()).toContain("请求总数");
    expect(wrapper.get('[data-testid="model-monitor-overview"]').text()).toContain("Hooks 活动");
    expect(wrapper.get('[data-testid="model-monitor-overview"]').text()).toContain("阻断 1");
    expect(wrapper.get('[data-testid="model-monitor-providers"]').text()).toContain("test-openai");
    expect(wrapper.get('[data-testid="model-monitor-tools"]').text()).toContain("search_docs");
    expect(wrapper.get('[data-testid="model-monitor-hook-classes-summary"]').text()).toContain("observe");
    expect(wrapper.get('[data-testid="model-monitor-hook-classes-summary"]').text()).toContain("guard");
    expect(wrapper.get('[data-testid="model-monitor-hooks-summary"]').text()).toContain("audit.observe");
    expect(wrapper.get('[data-testid="model-monitor-hooks-summary"]').text()).toContain("guard.input");
    expect(wrapper.get('[data-testid="model-monitor-capability-sources-summary"]').text()).toContain("mcp-local");
    expect(wrapper.get('[data-testid="model-monitor-capability-invocation-modes-summary"]').text()).toContain("direct_tool_call");
    expect(wrapper.get('[data-testid="model-monitor-capability-failure-classes-summary"]').text()).toContain("permission_denied");
    expect(wrapper.text()).toContain("Alpha Session");
    expect(wrapper.get('[data-testid="model-monitor-capability-source-detail"]').text()).toContain("Last Ingress");
    expect(wrapper.get('[data-testid="model-monitor-capability-source-detail"]').text()).toContain("builtin:time_now");
    expect(wrapper.get('[data-testid="model-monitor-sessions"]').text()).toContain("hooks 3 / blocked 1");
    expect(wrapper.get('[data-testid="model-monitor-drilldown-metrics"]').text()).toContain("Hooks 与阻断");
    expect(wrapper.get('[data-testid="model-monitor-drilldown-metrics"]').text()).toContain("3 calls");
    expect(wrapper.get('[data-testid="model-monitor-trace-timeline"]').text()).toContain("Return result");
    expect(wrapper.get('[data-testid="model-monitor-trace-timeline"]').text()).toContain("prepare_retrieval");
    expect(wrapper.get('[data-testid="model-monitor-hook-trace"]').text()).toContain("audit.observe");
    expect(wrapper.get('[data-testid="model-monitor-hook-trace"]').text()).toContain("guard.input");
    expect(wrapper.get('[data-testid="model-monitor-hook-trace"]').text()).toContain("blocked");
    wrapper.unmount();
  });

  it("点击 session 行后切换下钻内容", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return capabilityPayload;
      }

      if (command === "load_model_monitor_summary") {
        return createSummaryView();
      }

      if (command === "load_model_monitor_session_drilldown") {
        if (args?.sessionId === "session-beta") {
          return createDrilldownView("session-beta", "Beta Session", "监控摘要 beta");
        }

        return createDrilldownView();
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    await wrapper.get('[data-testid="model-monitor-session-session-beta"]').trigger("click");
    await flushPromises();

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith("load_model_monitor_session_drilldown", {
      sessionId: "session-beta"
    });
    expect(wrapper.text()).toContain("Beta Session");
    wrapper.unmount();
  });

  it("只接受最后一次 session 下钻响应，避免旧请求覆盖新选择", async () => {
    const alphaDeferred = createDeferred<ModelMonitorSessionDrilldownView>();
    const betaDeferred = createDeferred<ModelMonitorSessionDrilldownView>();

    tauriMocks.mockSafeInvoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return Promise.resolve(capabilityPayload);
      }

      if (command === "load_model_monitor_summary") {
        return Promise.resolve(createSummaryView());
      }
      if (command === "load_model_monitor_session_drilldown") {
        if (args?.sessionId === "session-alpha") {
          return alphaDeferred.promise;
        }
        if (args?.sessionId === "session-beta") {
          return betaDeferred.promise;
        }
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    await wrapper.get('[data-testid="model-monitor-session-session-beta"]').trigger("click");

    betaDeferred.resolve(createDrilldownView("session-beta", "Beta Session", "监控摘要 beta"));
    await flushPromises();
    expect(wrapper.text()).toContain("Beta Session");

    alphaDeferred.resolve(createDrilldownView("session-alpha", "Alpha Session", "监控摘要 alpha"));
    await flushPromises();

    expect(wrapper.get('[data-testid="model-monitor-drilldown-title"]').text()).toBe("Beta Session");
    wrapper.unmount();
  });

  it("展示后端错误态", async () => {
    tauriMocks.mockSafeInvoke.mockRejectedValue(new Error("summary failed"));

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    expect(wrapper.get('[data-testid="model-monitor-summary-error"]').text()).toContain("summary failed");
    wrapper.unmount();
  });

  it("在非 tauri 环境展示不可用提示", async () => {
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    expect(wrapper.get('[data-testid="model-monitor-summary-error"]').text()).toContain("Tauri 后端");
    wrapper.unmount();
  });

  it("展示 capability source、capability 列表与 inspect 详情", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return capabilityPayload;
      }

      if (command === "load_model_monitor_summary") {
        return createSummaryView();
      }

      if (command === "load_model_monitor_session_drilldown") {
        return createDrilldownView();
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    expect(wrapper.get('[data-testid="model-monitor-capability-sources"]').text()).toContain("Builtin Tools");
    expect(wrapper.get('[data-testid="model-monitor-capabilities"]').text()).toContain("time_now");
    expect(wrapper.get('[data-testid="model-monitor-capability-detail"]').text()).toContain("builtin:time_now");
    wrapper.unmount();
  });
  it("renders capability activity in trace drilldown", async () => {
    const drilldown = createDrilldownView();
    drilldown.runtimeView.session.turnTraceHistory = [
      createTrace({
        turnId: "turn-capability",
        title: "Capability Trace",
        toolActivities: [
          {
            id: "tool-1",
            name: "workspace_search",
            status: "done",
            summary: "capability execution completed",
            argumentsText: "{\"query\":\"abc\"}",
            resultText: "ok",
            durationSeconds: 0.2,
            capabilityInvocation: {
              toolName: "workspace_search",
              capabilityId: "mcp:tool:workspace_search",
              sourceId: "mcp-local",
              sourceKind: "mcp",
              capabilityKind: "tool",
              invocationMode: "direct_tool_call",
              failureKind: null,
              requiresApproval: false,
              hostMediated: true,
              permissionScope: "workspace.read"
            }
          }
        ]
      })
    ];

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return capabilityPayload;
      }

      if (command === "load_model_monitor_summary") {
        return createSummaryView();
      }

      if (command === "load_model_monitor_session_drilldown") {
        return drilldown;
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    const capabilityActivity = wrapper.get('[data-testid="model-monitor-capability-activity"]').text();
    expect(capabilityActivity).toContain("mcp:tool:workspace_search");
    expect(capabilityActivity).toContain("mcp-local");
    expect(capabilityActivity).toContain("direct_tool_call");
    wrapper.unmount();
  });

  it("marks traces without terminal envelope as raw evidence only", async () => {
    const drilldown = createDrilldownView();
    drilldown.runtimeView.session.turnTraceHistory = [
      createTrace({
        turnId: "turn-raw",
        eventId: null,
        eventType: null,
        eventVersion: null,
        sequence: null,
        emittedAtMs: null,
        title: "Raw Trace"
      })
    ];

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      const capabilityPayload = mockCapabilityCommand(command, args);
      if (capabilityPayload !== undefined) {
        return capabilityPayload;
      }

      if (command === "load_model_monitor_summary") {
        return createSummaryView();
      }

      if (command === "load_model_monitor_session_drilldown") {
        return drilldown;
      }

      throw new Error(`unexpected command: ${command}`);
    });

    const wrapper = mount(ModelMonitorPage);
    await flushPromises();

    expect(wrapper.get('[data-testid="model-monitor-raw-trace-warning"]').text()).toContain(
      "缺少 canonical terminal envelope"
    );
    wrapper.unmount();
  });
});
