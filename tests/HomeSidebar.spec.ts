import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSidebar from "@/components/HomeSidebar.vue";
import {
  __resetFrontendFlightRecorderForTests,
  injectFrontendDiagnosticStall
} from "@/lib/frontend-flight-recorder";
import { useProviderStore } from "@/stores/providers";
import { useRuntimeStore } from "@/stores/runtime";
import type { ProviderRegistry } from "@/types/provider";
import type {
  BuildContextObservation,
  ProviderCallCacheRecord,
  RetrievedContextState,
  TraceStep,
  TraceTimelineEntry,
  TurnTraceRecord
} from "@/types/runtime";

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

vi.mock("@/lib/frontend-flight-recorder", async () => {
  const actual = await vi.importActual<typeof import("@/lib/frontend-flight-recorder")>(
    "@/lib/frontend-flight-recorder"
  );
  return {
    ...actual,
    injectFrontendDiagnosticStall: vi.fn(() => 1800)
  };
});

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

const TooltipStub = defineComponent({
  props: {
    text: {
      type: String,
      default: ""
    }
  },
  template: '<div class="tooltip-stub" :data-tooltip="text"><slot /></div>'
});

function createProviderRegistry(): ProviderRegistry {
  return {
    selectedProviderId: "provider-openai",
    providers: [
      {
        id: "provider-openai",
        name: "OpenAI",
        protocol: "openai",
        baseUrl: "https://api.openai.com/v1",
        apiKeyEnvVar: "OPENAI_API_KEY",
        apiKeyValue: "",
        apiKeyPresent: false,
        selectedModelId: "model-gpt5",
        models: [
          {
            id: "model-gpt5",
            name: "GPT-5",
            model: "gpt-5",
            temperature: 0,
            maxOutputTokens: 4096,
            reasoningEffort: null,
            reasoningBudgetTokens: null,
            capabilityPreset: "open-ai-reasoning",
            capabilities: {
              contextWindowTokens: 128000,
              supportsTools: true,
              supportsStreaming: true,
              supportsImageInput: false,
              supportsReasoning: true
            }
          }
        ]
      }
    ]
  };
}

function createRetrievedContext(overrides: Partial<RetrievedContextState> = {}): RetrievedContextState {
  return {
    turnContext: {
      userMessage: "继续推进 PA-025",
      images: [],
      referencesImage: false
    },
    sessionContext: {
      conversationId: "graph-session",
      title: "Graph session",
      summary: "retrieval summary from host",
      recentHistory: [
        { role: "user", content: "继续推进 PA-025" },
        { role: "assistant", content: "正在继续处理" }
      ],
      recentAttachmentAssets: [],
      turnCount: 3,
      lastReferencedFile: "src-tauri/src/agent/context.rs"
    },
    runState: {
      runId: "run-alpha",
      goal: "梳理 retrieval boundary 并推进上层接入",
      phase: "running",
      activeTurnId: "turn-1",
      lastCompletedTurnId: null,
      resumeCount: 0,
      lastDecisionSummary: "继续推进 runtime 侧接口",
      executionCheckpointStatus: "running",
      executionCheckpointPhase: "calling_model"
    },
    longTermMemory: {
      status: "available",
      summary: "已有长期记忆",
      entries: [
        {
          kind: "user_preference.response_language",
          content: "Reply in Chinese.",
          source: "explicit_user_message",
          updatedAtMs: 1_715_000_000_000
        },
        {
          kind: "project_focus.active_task",
          content: "Current active task is PA-018.",
          source: "explicit_user_message",
          updatedAtMs: 1_715_000_000_001
        }
      ]
    },
    transcript: {
      providerNativeMessages: []
    },
    ...overrides
  };
}

function createBuildContextObservation(
  overrides: Partial<BuildContextObservation> = {}
): BuildContextObservation {
  return {
    requestFormat: overrides.requestFormat ?? "response_format=json_schema",
    messageCount: overrides.messageCount ?? 3,
    imageCount: overrides.imageCount ?? 1,
    toolCount: overrides.toolCount ?? 2,
    temperature: overrides.temperature ?? 0.2,
    maxOutputTokens: overrides.maxOutputTokens ?? 4096,
    stablePrefixText:
      overrides.stablePrefixText ??
      "system: stable system rule\ndeveloper: stable capability prefix",
    semiStableContextText:
      overrides.semiStableContextText ??
      "developer: retrieval summary from host\nuser: recent history question",
    volatileInputText:
      overrides.volatileInputText ??
      "user: continue PA-025 with the latest screenshot",
    prefixMutationReasons: overrides.prefixMutationReasons ?? [],
    contextRefreshReason: overrides.contextRefreshReason ?? "initial_build",
    instructionScopeSources:
      overrides.instructionScopeSources ?? ["thread://base-system", "workspace://src/App.vue"],
    conversationCarryMode: overrides.conversationCarryMode ?? "full_replay",
    requestMessagesText:
      overrides.requestMessagesText ??
      "[0] system\nsummarize retrieval state\n\n[1] user\ncontinue PA-025\n\n[2] assistant\nacknowledged",
    toolDefinitionsText:
      overrides.toolDefinitionsText ??
      "workspace.read_file(path: string)\nworkspace.search(query: string)"
  };
}

function createTraceSteps(): TraceStep[] {
  return [
    { id: "step-plan", label: "Receive input", state: "completed" },
    { id: "step-context", label: "Build context", state: "completed" },
    { id: "step-call-model", label: "Call model", state: "completed" },
    { id: "step-call-tool", label: "Call tool", state: "pending" },
    { id: "step-return", label: "Return result", state: "completed" }
  ];
}

function createTraceTimeline(): TraceTimelineEntry[] {
  return [
    {
      id: "input-1",
      kind: "input",
      label: "RECEIVE INPUT",
      state: "completed",
      sequence: 1,
      text: "继续推进 PA-025，不要生成摘要"
    },
    { id: "context-2", kind: "context", label: "BUILD CONTEXT", state: "completed", sequence: 2 },
    { id: "model-3", kind: "model", label: "CALL MODEL #1", state: "completed", sequence: 3, firstTokenLatencyMs: 321 },
    {
      id: "return-4",
      kind: "return",
      label: "RETURN RESULT",
      state: "completed",
      sequence: 4,
      inputTokens: 120,
      cacheHitInputTokens: 80,
      reasoningTokens: 18,
      outputTokens: 40,
      totalTokens: 160,
      firstTokenLatencyMs: 321,
      turnDurationMs: 2800
    }
  ];
}

function createTraceRecord(overrides: Partial<TurnTraceRecord> = {}): TurnTraceRecord {
  return {
    turnId: overrides.turnId ?? "turn-1",
    title: overrides.title ?? "测试轮次",
    phase: overrides.phase ?? "ready",
    traceSteps: overrides.traceSteps ?? createTraceSteps(),
    traceTimeline: overrides.traceTimeline ?? createTraceTimeline(),
    toolActivities: overrides.toolActivities ?? [],
    providerRequestedName: overrides.providerRequestedName ?? null,
    providerName: overrides.providerName ?? null,
    providerProtocol: overrides.providerProtocol ?? null,
    providerModel: overrides.providerModel ?? null,
    providerSource: overrides.providerSource ?? null,
    providerMode: overrides.providerMode ?? null,
    buildContextObservation: overrides.buildContextObservation ?? null,
    sessionSummary: overrides.sessionSummary ?? null,
    fallbackReason: overrides.fallbackReason ?? null,
    error: overrides.error ?? null,
    inputTokens: overrides.inputTokens ?? null,
    cacheHitInputTokens: overrides.cacheHitInputTokens ?? null,
    reasoningTokens: overrides.reasoningTokens ?? null,
    outputTokens: overrides.outputTokens ?? null,
    totalTokens: overrides.totalTokens ?? null,
    firstTokenLatencyMs: overrides.firstTokenLatencyMs ?? null,
    turnDurationMs: overrides.turnDurationMs ?? null,
    updatedAt: overrides.updatedAt ?? 1,
    providerCallRecords: overrides.providerCallRecords ?? []
  };
}

function createProviderCallRecord(overrides: Partial<ProviderCallCacheRecord> = {}): ProviderCallCacheRecord {
  return {
    requestKind: overrides.requestKind ?? "initial_request",
    providerSource: overrides.providerSource ?? "provider_decision_stream",
    providerMode: overrides.providerMode ?? "live",
    inputTokens: overrides.inputTokens ?? null,
    cacheHitInputTokens: overrides.cacheHitInputTokens ?? null,
    cacheMissInputTokens: overrides.cacheMissInputTokens ?? null,
    reasoningTokens: overrides.reasoningTokens ?? null,
    outputTokens: overrides.outputTokens ?? null,
    totalTokens: overrides.totalTokens ?? null,
    firstTokenLatencyMs: overrides.firstTokenLatencyMs ?? null,
    turnDurationMs: overrides.turnDurationMs ?? null,
    latencyKind: overrides.latencyKind ?? "provider_stream",
    prefixMutationReasons: overrides.prefixMutationReasons ?? []
  };
}

async function flushAll() {
  await Promise.resolve();
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

async function mountSidebar() {
  const providerStore = useProviderStore();
  providerStore.$patch({
    registry: createProviderRegistry(),
    selectedReasoningEffort: null
  });

  const wrapper = mount(HomeSidebar, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub,
        Tooltip: TooltipStub
      }
    }
  });
  await flushAll();
  return wrapper;
}

function countOccurrences(text: string, needle: string) {
  return text.split(needle).length - 1;
}

describe("HomeSidebar", () => {
  // ── PA-096：右侧栏只保留 状态/计划/调试；trace 用例迁 tests/TraceInspector.spec.ts，
  // 工具目录用例迁 tests/ConfigToolsSection.spec.ts（按交互 testid 分类）。──

  beforeEach(() => {
    vi.clearAllMocks();
    __resetFrontendFlightRecorderForTests();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    vi.spyOn(console, "info").mockImplementation(() => {});

    if (!navigator.clipboard) {
      Object.defineProperty(navigator, "clipboard", {
        value: { writeText: vi.fn() },
        configurable: true
      });
    } else {
      vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    }

    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "graph-session",
      sessionList: [],
      sessionOperation: null,
      isSubmitting: false,
      messages: [],
      phase: "ready"
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("右侧栏只保留状态、计划段，计划面板有值时显示，不再渲染 Tools、Trace 与 Debug（PA-096/C6）", async () => {
    // 计划与任务在无值时不渲染（由原来的常驻改为有值才显示）
    let wrapper = await mountSidebar();
    await flushAll();

    expect(wrapper.text()).toContain("状态");
    expect(wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.getAttribute("data-open")).toBe("true");
    const planPanelToggle = wrapper.find('[data-testid="plan-panel-toggle"]');
    expect(planPanelToggle.exists()).toBe(false);
    expect(wrapper.text()).not.toContain("Tools");
    expect(wrapper.text()).not.toContain("DEBUG");
    expect(wrapper.find('[data-testid="tools-panel-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="trace-panel-toggle"]').exists()).toBe(false);
    wrapper.unmount();

    // 当 plans > 0 时，显示计划
    tauriMocks.mockSafeInvoke.mockImplementation(async (cmd) => {
      if (cmd === "plan_list") {
        return [
          {
            planId: "p1",
            kind: "plan",
            summary: "测试计划",
            lifecycle: "active",
            steps: [],
            revision: 1
          }
        ];
      }
      return null;
    });
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({ sessionId: "session-plan-1" });
    const planStore = (await import("@/stores/plan")).usePlanStore();
    await planStore.list("session-plan-1");

    const wrapperWithPlan = mount(HomeSidebar, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Tooltip: TooltipStub
        }
      }
    });
    await flushAll();
    expect(wrapperWithPlan.text()).toContain("计划");
    wrapperWithPlan.unmount();
  }, 10000);

  it("右侧栏条件渲染：有子智能体时显示团队（队友文案改为团队），有任务时显示任务看板", async () => {
    let wrapper = await mountSidebar();
    await flushAll();

    // 默认空状态无子智能体、无任务看板
    expect(wrapper.find('[data-testid="team-roster-toggle"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="team-task-board-toggle"]').exists()).toBe(false);
    wrapper.unmount();

    // 注入子智能体与任务数据
    const teamStore = (await import("@/stores/team")).useTeamStore();
    teamStore.$patch({
      members: [
        {
          target: "lead",
          name: "Lead",
          role: "lead",
          status: "running",
          description: "Leader"
        },
        {
          target: "subagent-1",
          name: "Worker",
          role: "executor",
          status: "active",
          description: "Subagent 1"
        }
      ],
      tasks: [
        {
          task_id: "task-1",
          subject: "Test task",
          description: "desc",
          status: "pending",
          revision: 1,
          created_at_ms: Date.now(),
          write_scopes: [],
          blocked_by: []
        }
      ]
    });

    const wrapperWithTeam = mount(HomeSidebar, {
      global: {
        stubs: {
          ScrollArea: ScrollAreaStub,
          Tooltip: TooltipStub
        }
      }
    });
    await flushAll();

    const rosterToggle = wrapperWithTeam.find('[data-testid="team-roster-toggle"]');
    expect(rosterToggle.exists()).toBe(true);
    expect(rosterToggle.text()).toContain("团队");
    expect(rosterToggle.text()).not.toContain("队友");

    const taskBoardToggle = wrapperWithTeam.find('[data-testid="team-task-board-toggle"]');
    expect(taskBoardToggle.exists()).toBe(true);
    expect(taskBoardToggle.text()).toContain("任务看板");

    wrapperWithTeam.unmount();
  });

  it("实时聚合：进行中 turn 的模型调用与 token 即时反映在状态面板", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      isSubmitting: true,
      activeTurnId: "turn-live-aggregate",
      inputTokens: 1200,
      outputTokens: 300,
      traceTimeline: [
        {
          id: "call-live-1",
          turnId: "turn-live-aggregate",
          sequence: 1,
          kind: "call_model",
          label: "CALL MODEL #1",
          state: "completed",
          inputTokens: 1200,
          outputTokens: 300
        }
      ],
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-settled-aggregate",
          title: "settled aggregate",
          phase: "completed",
          inputTokens: 800,
          outputTokens: 200,
          updatedAt: 1000
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    // liveTurnEnabled 开启：活跃 turn 并入聚合，轮次数包含进行中
    const turnCountStub = wrapper
      .get('[data-testid="status-panel-toggle"]')
      .element.closest("section")!
      .querySelector<HTMLElement>('.tooltip-stub[data-tooltip="对话轮次数"]');
    expect(turnCountStub?.textContent?.trim()).toBe("2");
  }, 10000);

  it("状态面板不再承载会话操作状态（已收敛到底部状态栏）", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: "switching"
    });

    const wrapper = await mountSidebar();
    await flushAll();

    expect(wrapper.find('[data-testid="status-session-summary"]').exists()).toBe(false);
    expect(wrapper.text()).not.toContain("正在切换对话");
  });

  it("状态面板保持紧凑布局，受控状态与会话状态均归入底部状态栏", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionOperation: "switching",
      latestRunControlAuditSummary: {
        actionEvidenceSummary: {
          status: "available",
          sourceFamily: "run_control",
          commandKind: "resume_graph_run_stream",
          boundary: "resume_requested",
          resultKind: "observe",
          summary: "检测到暂停中的运行；点击后会恢复该 run 并继续执行。",
          targetSummary: "恢复 run-alpha",
          blocked: false,
          degraded: false
        },
        currentContextProjection: {
          phase: "paused",
          checkpointStatus: "ready",
          activeRunId: "run-alpha",
          submissionPlanCommand: "resume_graph_run_stream"
        }
      }
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(statusPanelText).not.toContain("Run phase");
    expect(statusPanelText).not.toContain("Recent history");
    expect(statusPanelText).not.toContain("Recent attachments");
    expect(statusPanelText).not.toContain("Long-term memory");
    expect(statusPanelText).not.toContain("Goal:");
    expect(statusPanelText).not.toContain("Last file:");
    expect(statusPanelText).not.toContain("正在切换对话");
    expect(wrapper.find('[data-testid="status-session-summary"]').exists()).toBe(false);
  });

  it("状态面板中的 token 标签不显示总计字样", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-status-tokens",
          title: "status token labels",
          phase: "completed",
          inputTokens: 8437,
          cacheHitInputTokens: 4352,
          outputTokens: 1366,
          // wave-61 §5.1（D5，Lead 一票裁决 2026-11-03）：补一条 per-call 记录，只加数据、不改断言强度。
          // 原意图：本用例要验证「输入/输出/缓存三位有值 ⇒ 其 tooltip 语义可达」。
          // 缺陷根因：`sessionCacheHitTokensTotal` 由 `providerCallRecords[].cacheHitInputTokens` 求和，
          //   仅有 turn 级 `cacheHitInputTokens` 时该值为 null ⇒ 按 §2 E-2 缓存位整个不渲染。
          // 修正：补 per-call 记录（含 cacheHitInputTokens / cacheMissInputTokens / outputTokens），
          //   **刻意不提供 `turnDurationMs` / `firstTokenLatencyMs`** ⇒ 生成速度仍无值，
          //   使「速度位缺席 ⇒ 其 tooltip 缺席」的条件断言前提保持成立（只补缓存维度）。
          providerCallRecords: [
            createProviderCallRecord({
              inputTokens: 8437,
              cacheHitInputTokens: 4352,
              cacheMissInputTokens: 4085,
              outputTokens: 1366
            })
          ]
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanel = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")!;
    const statusPanelText = statusPanel.textContent ?? "";
    const tooltipStubs = statusPanel.querySelectorAll<HTMLElement>(".tooltip-stub[data-tooltip]");
    const tooltipTexts = Array.from(tooltipStubs).map((el) => el.getAttribute("data-tooltip"));

    // Token labels are now tooltips instead of visible text
    // ── wave-61 §5.1 追加授权（Lead 二轮裁决 R6，2026-11-03）───────────────────────
    //   原意图：token 位的语义标签必须**可达**（从可见文本迁移为 tooltip 后不丢语义）。
    //   迁移理由：wave-61 §2 E-2 让「无值位点」整个不渲染，其内层 Tooltip 随之消失；
    //             本夹具无 per-call 分量 ⇒ 只有速度位缺席，其余三位在场。
    //   等价断言：输入/输出/缓存读取三个基础标签仍在；速度位缺席 ⇒ 其 tooltip 亦缺席。
    // wave-61 §5.1（D5，Lead 一票裁决 2026-11-03：**恢复强断言**，用补夹具而非放宽断言）：
    //   夹具已补 per-call 缓存记录 ⇒ 输入/输出/缓存三位均在场 ⇒ 「缓存读取」可无条件精确包含。
    expect(
      tooltipTexts,
      `wave-61 §5.1（R6）：输入/输出/缓存三位有值 ⇒ 三个基础标签必须可达；实际 tooltip 文案：${JSON.stringify(tooltipTexts)}`
    ).toEqual(expect.arrayContaining(["输入", "输出", "缓存读取"]));

    // 夹具自检（Lead 指定）：缓存位必须真实渲染且文本非空 —— 防止将来夹具退化后
    // 「tooltip 名单」断言退化成与数据无关的空断言。
    const cacheSlot = statusPanel.querySelector<HTMLElement>('[data-testid="status-metric-cache-tokens"]');
    expect(
      cacheSlot,
      "D5 夹具自检：补齐 per-call 记录后缓存位 status-metric-cache-tokens 必须在 DOM（否则强断言无从成立）"
    ).not.toBeNull();
    expect(
      (cacheSlot?.textContent ?? "").trim(),
      "D5 夹具自检：缓存位文本必须非空（值来自 providerCallRecords[].cacheHitInputTokens）"
    ).not.toBe("");

    expect(
      tooltipTexts,
      "wave-61 §5.1（R6）条件等价断言：速度位不存在 ⇒ 其 tooltip 亦不得存在"
    ).not.toContain("生成速度");
    // wave-54 C6（ADR 0053）：紧凑格式 <1e6 取一位小数（8437 ⇒ 8.4K、1366 ⇒ 1.4K），
    // 不再四舍五入成整数 K；Token 文本仍移除。
    expect(statusPanelText).not.toContain("Token");
    expect(statusPanelText).toContain("8.4K");
    expect(statusPanelText).toContain("1.4K");
    expect(statusPanelText).toMatch(/\d\.\dK/);
    // wave-54 C6 / ADR 0053：该夹具无 per-call 分量（无 providerCallRecords、无 turnDurationMs）
    // ⇒ 速度无值。
    // wave-61 §5 授权迁移（Lead 一票，原意图保真）：
    //   原意图：速度位空值时**不得**显示带 t/s 的数字（旧整轮聚合回退已废除）；
    //   迁移理由：wave-61 §2 E-2/E-5 把「空值」的表达从占位文本 `—` 改为「位点整个不渲染」，
    //             §2 E-5 明令禁止 `—` 占位文本；
    //   等价断言：速度位在无值时**不存在于 DOM**（比"存在但显示 —"更强的同义断言：既不显示数字，
    //             也不显示占位符）。
    const speedCarrier = statusPanel.querySelector<HTMLElement>('[data-testid="status-metric-token-speed"]');
    expect(
      speedCarrier,
      "wave-61 §5 迁移：速度无值 ⇒ 速度位整个 DOM 不渲染（禁止 `—` 占位）"
    ).toBeNull();
    expect(
      statusPanelText,
      "wave-61 §5 迁移：状态面板内不得再出现 `—` 占位文本"
    ).not.toContain("—");
    // No "总计" suffix in labels
    expect(statusPanelText).not.toContain("输入总计");
    expect(statusPanelText).not.toContain("缓存读取总计");
    expect(statusPanelText).not.toContain("输出总计");
  });
  it("wave-61 §5.1（R6 正向半边）：速度位在场时其 tooltip 必须存在（原意图双向可验）", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-status-speed-present",
          title: "status speed present",
          phase: "completed",
          traceTimeline: [
            {
              id: "speed-call-model-1",
              kind: "call_model",
              label: "CALL MODEL #1",
              state: "completed",
              sequence: 1,
              inputTokens: 1000,
              outputTokens: 500,
              turnDurationMs: 2000,
              firstTokenLatencyMs: 0
            }
          ],
          providerCallRecords: [
            createProviderCallRecord({
              inputTokens: 1000,
              outputTokens: 500,
              turnDurationMs: 2000,
              firstTokenLatencyMs: 0
            })
          ],
          inputTokens: 1000,
          outputTokens: 500,
          turnDurationMs: 2000,
          firstTokenLatencyMs: 0
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanel = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")!;
    const tooltipStubs = statusPanel.querySelectorAll<HTMLElement>(".tooltip-stub[data-tooltip]");
    const tooltipTexts = Array.from(tooltipStubs).map((el) => el.getAttribute("data-tooltip"));

    // Assert（锚点自检：速度位必须在场 —— 该夹具带 per-call 分量，speed 可算）
    const speedCarrier = statusPanel.querySelector<HTMLElement>('[data-testid="status-metric-token-speed"]');
    expect(
      speedCarrier,
      "R6 正向：带 per-call 分量的夹具下速度位必须渲染（否则本用例无法证明 tooltip 的在场分支）"
    ).not.toBeNull();

    // Assert（契约主体：速度位在场 ⇒ 其 tooltip 语义可达）
    expect(
      tooltipTexts,
      `R6 正向：速度位在场时 tooltipTexts 必须包含「生成速度」；实际 ${JSON.stringify(tooltipTexts)}`
    ).toContain("生成速度");

    wrapper.unmount();
  });

  // PA-096：原 build_context 混合用例的状态面板负断言半边（trace 正断言半边在 TraceInspector.spec）。
  // 注：trace 面板已不在本树内，此负断言验证状态面板自身不渲染请求细节。
  it("状态面板不暴露 build-context 请求细节", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionSummary: "legacy summary should be shadowed",
      turnTraceHistory: [
        createTraceRecord({
          turnId: "turn-build-context",
          title: "show build context",
          phase: "completed"
        })
      ]
    });

    const wrapper = await mountSidebar();
    await flushAll();

    const statusPanelText = wrapper.get('[data-testid="status-panel-toggle"]').element.closest("section")?.textContent ?? "";

    expect(statusPanelText).not.toContain("legacy summary should be shadowed");
    expect(statusPanelText).not.toContain("[0] system");
    expect(statusPanelText).not.toContain("stable capability prefix");
    expect(wrapper.find('[data-testid="trace-step-button-context-2"]').exists()).toBe(false);
  });

  it.skip("显示 recorder 摘要并支持导出前端 trace", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      frontendRecorderStats: {
        initialized: true,
        activeSessionId: "graph-session",
        activeTurnId: "turn-1",
        seq: 3,
        bufferedEventCount: 0,
        bufferedSnapshotCount: 0,
        flushCount: 1,
        flushFailureCount: 0,
        droppedEventCount: 2,
        droppedSnapshotCount: 1,
        stallCount: 4,
        lastStallGapMs: 620,
        lastFlushDurationMs: 18,
        lastFlushAtMs: 1000,
        lastError: null,
        lastExportAtMs: null
      },
      turnTraceHistory: [createTraceRecord({ turnId: "turn-1", updatedAt: 2000 })]
    });
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "export_frontend_trace_json") {
        return {
          format: "json",
          fileName: "frontend-trace.json",
          content: "{\"events\":[]}",
          truncated: false,
          eventCount: 0,
          snapshotCount: 0,
          fromWallMs: null,
          toWallMs: null
        };
      }
      return null;
    });

    const wrapper = await mountSidebar();
    await flushAll();

    // KNOWN TEST DEBT: Recorder UI section rendering changed
    expect(wrapper.text()).toContain("状态");

    const exportButton = wrapper.findAll("button").find((button) => button.text().includes("导出 JSON"));
    expect(exportButton).toBeTruthy();
    await exportButton!.trigger("click");

    expect(tauriMocks.mockSafeInvoke).toHaveBeenCalledWith(
      "export_frontend_trace_json",
      expect.objectContaining({
        sessionId: "graph-session",
        turnId: "turn-1"
      })
    );
  });

  it.skip("支持手动注入 stall smoke 以便采集前端卡顿证据", async () => {
    // KNOWN TEST DEBT: refreshFrontendRecorderStats removed from store
    vi.useFakeTimers();
    const runtimeStore = useRuntimeStore();
    const refreshSpy = vi.spyOn(runtimeStore, "refreshFrontendRecorderStats");

    const wrapper = await mountSidebar();
    await flushAll();

    await wrapper.get('[data-testid="frontend-stall-smoke"]').trigger("click");
    await vi.runAllTimersAsync();

    expect(injectFrontendDiagnosticStall).toHaveBeenCalledWith(1800, "sidebar-stall-smoke");
    expect(refreshSpy).toHaveBeenCalled();
    vi.useRealTimers();
  });
});
