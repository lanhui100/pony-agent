import { afterEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, h, nextTick } from "vue";
import { TooltipProvider } from "reka-ui";
import HomeWorkspace from "@/components/HomeWorkspace.vue";
import { useRuntimeStore } from "@/stores/runtime";
import { hydrateMessagesFromHistory } from "@/lib/runtime/messages";
import type {
  ChatMessage,
  ToolActivity,
  TraceTimelineEntry,
  TurnHistoryMessage,
  TurnTraceRecord
} from "@/types/runtime";

vi.mock("@/lib/tauri", () => ({
  safeInvoke: vi.fn(),
  safeListen: vi.fn(),
  isTauriAvailable: vi.fn(() => false)
}));

const SHOW_REASONING_STORAGE_KEY = "pony-agent.ui.show-reasoning-content";

const ScrollAreaStub = defineComponent({
  setup(_props, { slots, expose }) {
    expose({
      viewportEl: document.createElement("div"),
      scrollToBottom: vi.fn()
    });

    return () => h("div", { class: "scroll-area-stub" }, slots.default ? slots.default() : []);
  }
});

function makeTraceEntry(
  id: string,
  kind: TraceTimelineEntry["kind"],
  sequence: number,
  extra: Partial<TraceTimelineEntry> = {}
): TraceTimelineEntry {
  return {
    id,
    kind,
    label: id,
    state: "completed",
    sequence,
    ...extra
  };
}

function makeToolActivity(id: string, name: string): ToolActivity {
  return {
    id,
    name,
    canonicalToolName: name,
    displayNameZh: name,
    status: "done",
    description: name,
    resultText: `${name} 执行完成`
  };
}

/** 一轮纯历史会话：user + assistant（带 reasoning）+ 一条 trace 内工具调用。 */
function historyTurnFixture() {
  const history: TurnHistoryMessage[] = [
    {
      role: "user",
      content: "检查系统状态"
    },
    {
      role: "assistant",
      content: "系统运行正常，一切就绪。",
      reasoningContent: "需要先检查系统各种运行指标..."
    }
  ];
  return history;
}

const mountedWrappers: Array<{ unmount: () => void }> = [];

afterEach(() => {
  while (mountedWrappers.length) {
    mountedWrappers.pop()?.unmount();
  }
  window.localStorage.clear();
});

async function mountWorkspace(messages: ChatMessage[], turnTraceHistory: TurnTraceRecord[]) {
  window.localStorage.setItem(SHOW_REASONING_STORAGE_KEY, "true");
  const pinia = createPinia();
  setActivePinia(pinia);

  const runtimeStore = useRuntimeStore();
  runtimeStore.$patch({
    sessionOperation: null,
    phase: "ready",
    error: null,
    messages,
    turnTraceHistory
  });

  const wrapper = mount({
    render() {
      return h(TooltipProvider, null, {
        default: () => h(HomeWorkspace)
      });
    }
  }, {
    global: {
      directives: {
        motion: () => {}
      },
      stubs: {
        ScrollArea: ScrollAreaStub,
        MarkdownRenderer: {
          props: ["content"],
          template: `<div class="assistant-plain-text">{{ content }}</div>`
        },
        Transition: true,
        TransitionGroup: true
      }
    }
  });
  mountedWrappers.push(wrapper);

  await nextTick();
  await nextTick();

  return wrapper;
}

/** 取 agent shell 内直接子元素的展示时序标签（reasoning/tools/content），忽略其他装饰节点。 */
function agentShellEventOrder(wrapper: ReturnType<typeof mount>) {
  const agentShell = wrapper.get(".conversation-agent-shell");
  return Array.from(agentShell.element.children)
    .map((element) => {
      if (element.classList.contains("conversation-reasoning-panel")) return "reasoning";
      if (element.classList.contains("conversation-tool-panel")) return "tools";
      if (element.classList.contains("assistant-response-panel")) return "content";
      return null;
    })
    .filter((value): value is "reasoning" | "tools" | "content" => value !== null);
}

describe("Stage History Message Order Acceptance", () => {
  it("F1: hydrateMessagesFromHistory keeps the live-consistent array contract [user, assistant, tool]", () => {
    // 契约说明（Lead 裁定 wave-24）：store.messages 数组顺序必须与实时回合装配保持一致，
    // 即 [user, assistant, tool...]（实时装配见 src/stores/runtime.ts:2960-3064：
    // ensureAssistantMessage 先 push assistant，syncToolMessages 再把 tool 追加到数组尾部）。
    // 历史水合不得把 tool 消息插到 assistant 之前 —— 历史会话与实时会话必须是同一数组契约，
    // 展示层的 Reasoning -> Tools -> Content 视觉时序由 HomeWorkspace.agentTurnEvents 负责，
    // 不应通过污染 store 数组顺序来实现。
    const history = historyTurnFixture();

    const traceHistory: TurnTraceRecord[] = [
      {
        turnId: "history-turn-1",
        runId: "run-1",
        phase: "completed",
        updatedAt: 1000,
        toolActivities: [makeToolActivity("act-1", "read_file")]
      }
    ];

    // Act: 执行历史记录水合
    const messages = hydrateMessagesFromHistory(history, null, traceHistory);

    // Assert: 数组契约锁 —— 与实时装配一致，assistant 在前，tool 追加在后
    expect(messages.map((m) => m.role)).toEqual(["user", "assistant", "tool"]);

    const assistantIndex = messages.findIndex((m) => m.role === "assistant");
    const toolIndex = messages.findIndex((m) => m.role === "tool");
    expect(assistantIndex).toBeGreaterThan(-1);
    expect(toolIndex).toBeGreaterThan(-1);
    expect(assistantIndex).toBeLessThan(toolIndex);
  });

  it("F2: HomeWorkspace renders purely hydrated historical turn events in strict order (Reasoning -> Tools -> Content)", async () => {
    // Arrange: 纯历史会话，trace 无 traceTimeline（只有 toolActivities），
    // 展示层必须依靠 agentTurnEvents 的 fallback 排序权重保证 Reasoning -> Tools -> Content。
    const history = historyTurnFixture();

    const traceHistory: TurnTraceRecord[] = [
      {
        turnId: "history-turn-1",
        runId: "run-1",
        phase: "completed",
        updatedAt: 1000,
        toolActivities: [
          {
            id: "act-1",
            name: "check_health",
            canonicalToolName: "check_health",
            displayNameZh: "健康检查",
            status: "done",
            description: "健康检查",
            resultText: "ok"
          }
        ]
      }
    ];

    const hydratedMessages = hydrateMessagesFromHistory(history, null, traceHistory);

    // Act: 挂载 HomeWorkspace 并等待渲染完成
    const wrapper = await mountWorkspace(hydratedMessages, traceHistory);

    // Assert: 思考过程 -> 工具调用 -> 正文
    const reasoningPanel = wrapper.find(".conversation-reasoning-panel");
    const toolPanel = wrapper.find(".conversation-tool-panel");
    const contentPanel = wrapper.find(".assistant-response-panel");

    expect(reasoningPanel.exists()).toBe(true);
    expect(toolPanel.exists()).toBe(true);
    expect(contentPanel.exists()).toBe(true);

    expect(agentShellEventOrder(wrapper)).toEqual(["reasoning", "tools", "content"]);
  });

  it("F3: traceTimeline with only call_model (no call_tool) must still render Reasoning -> Tools -> Content", async () => {
    // Arrange: 历史 trace 的 traceTimeline 只记录了一个 call_model hop（无 call_tool 条目），
    // 工具信息来源是该 trace 的 toolActivities（旧数据/折叠重建后的常见形态）。
    // 展示层不得因为“工具没有 call_tool trace 链接”就把它排到内容之后。
    const history = historyTurnFixture();

    const traceHistory: TurnTraceRecord[] = [
      {
        turnId: "history-turn-1",
        runId: "run-1",
        phase: "completed",
        updatedAt: 1000,
        traceTimeline: [
          makeTraceEntry("trace-model-1", "call_model", 1, {
            reasoningContent: "先确认环境再决定是否调用工具",
            text: "已检查完毕，结论如下。"
          })
        ],
        toolActivities: [makeToolActivity("act-1", "check_health")]
      }
    ];

    const hydratedMessages = hydrateMessagesFromHistory(history, null, traceHistory);

    // Act
    const wrapper = await mountWorkspace(hydratedMessages, traceHistory);

    // Assert: 即使 traceTimeline 里没有 call_tool，展示顺序仍必须是 Reasoning -> Tools -> Content
    const reasoningPanel = wrapper.find(".conversation-reasoning-panel");
    const toolPanel = wrapper.find(".conversation-tool-panel");
    const contentPanel = wrapper.find(".assistant-response-panel");

    expect(reasoningPanel.exists()).toBe(true);
    expect(toolPanel.exists()).toBe(true);
    expect(contentPanel.exists()).toBe(true);

    const agentShell = wrapper.get(".conversation-agent-shell");
    const children = Array.from(agentShell.element.children);
    const reasoningDomIndex = children.indexOf(reasoningPanel.element);
    const toolDomIndex = children.indexOf(toolPanel.element);
    const contentDomIndex = children.indexOf(contentPanel.element);

    expect(reasoningDomIndex).toBeGreaterThan(-1);
    expect(toolDomIndex).toBeGreaterThan(-1);
    expect(contentDomIndex).toBeGreaterThan(-1);
    expect(reasoningDomIndex).toBeLessThan(contentDomIndex);
    // 核心红相断言：工具必须落在正文之上（当前实现 toolOrder = modelEntry.sequence + 0.1 会落到内容之下）
    expect(toolDomIndex).toBeLessThan(contentDomIndex);
  });

  it("F4: multi-hop interleaving with a linked call_tool keeps the tool between the first and last model content", async () => {
    // Arrange: 多 hop 交错回归锁 —— call_model(seq 1) -> call_tool(seq 2) -> call_model(seq 3)，
    // 工具消息 id 后缀与该 call_tool 条目 toolActivities[].id 匹配，展示层必须通过 trace 链接
    // 把工具排到第一个 model 内容与最后一个 model 内容之间，而不是塌缩到某一端。
    const history = historyTurnFixture();

    const traceHistory: TurnTraceRecord[] = [
      {
        turnId: "history-turn-1",
        runId: "run-1",
        phase: "completed",
        updatedAt: 1000,
        traceTimeline: [
          makeTraceEntry("trace-model-1", "call_model", 1, {
            reasoningContent: "第一跳：决定调用工具",
            text: "第一跳内容：准备调用工具。"
          }),
          makeTraceEntry("trace-tool-1", "call_tool", 2, {
            toolActivities: [makeToolActivity("act-hop-1", "read_file")]
          }),
          makeTraceEntry("trace-model-3", "call_model", 3, {
            reasoningContent: "第三跳：汇总工具结果",
            text: "第三跳内容：工具结果已汇总。"
          })
        ],
        toolActivities: []
      }
    ];

    const hydratedMessages = hydrateMessagesFromHistory(history, null, traceHistory);

    // Act
    const wrapper = await mountWorkspace(hydratedMessages, traceHistory);

    // Assert: 工具 DOM 位置必须夹在第一个 model 内容与最后一个 model 内容之间
    const agentShell = wrapper.get(".conversation-agent-shell");
    const children = Array.from(agentShell.element.children);
    const toolPanel = wrapper.find(".conversation-tool-panel");
    const contentPanels = wrapper.findAll(".assistant-response-panel");

    expect(toolPanel.exists()).toBe(true);
    expect(contentPanels.length).toBeGreaterThanOrEqual(2);

    const toolDomIndex = children.indexOf(toolPanel.element);
    const firstContentDomIndex = children.indexOf(contentPanels[0]!.element);
    const lastContentDomIndex = children.indexOf(contentPanels[contentPanels.length - 1]!.element);

    expect(toolDomIndex).toBeGreaterThan(-1);
    expect(firstContentDomIndex).toBeGreaterThan(-1);
    expect(lastContentDomIndex).toBeGreaterThan(-1);
    expect(firstContentDomIndex).toBeLessThan(toolDomIndex);
    expect(toolDomIndex).toBeLessThan(lastContentDomIndex);

    // 完整交错序列锁：reasoning -> content -> tools -> reasoning -> content
    expect(agentShellEventOrder(wrapper)).toEqual([
      "reasoning",
      "content",
      "tools",
      "reasoning",
      "content"
    ]);
  });

  it("F5: turn without reasoning content and without trace must still render Tools -> Content (canonical turn order)", async () => {
    // Arrange: 无 reasoningContent、无 traceTimeline 的纯快照回合（user + assistant + 一个工具）。
    // Lead 裁定二（.dev-team/wave24-adjudication.md §6）：Reasoning < Tools < 正文 必须无条件成立，
    // 不得依赖 reasoning 面板是否渲染 —— 该回合没有任何思考内容，不变量退化为 Tools < 正文。
    // 数组下标不是时间线：turn 内工具调用时序落盘即丢（docs/decisions/0008-event-sourcing-evolution.md:18），
    // 重载会话不存在权威时序来源，展示层必须使用规范化顺序。
    const turnId = "canonical-order-turn";
    const messages: ChatMessage[] = [
      {
        id: "user-canonical-order",
        turnId,
        role: "user",
        content: "verify canonical turn order",
        status: "done"
      },
      {
        id: "assistant-canonical-order",
        turnId,
        role: "assistant",
        content: "canonical answer without reasoning",
        // 关键：无 reasoningContent，思考面板必然不渲染
        reasoningContent: null,
        status: "done",
        modelName: "OpenAI/GPT-5"
      },
      {
        id: "tool-canonical-order",
        turnId,
        role: "tool",
        content: "canonical tool output",
        toolName: "Read",
        canonicalToolName: "Read",
        detail: "read before the answer",
        status: "done",
        durationSeconds: 0.4
      }
    ];

    // Act: turnTraceHistory 为空（该回合无 trace）。mountWorkspace 内已开启 show-reasoning-content，
    // 但本回合无 reasoningContent，故思考面板不会渲染 —— 下面的断言不得依赖思考面板存在。
    const wrapper = await mountWorkspace(messages, []);

    // Assert: 无思考面板，且工具仍必须落在正文之上（当前条件化实现为红）
    expect(wrapper.find(".conversation-reasoning-panel").exists()).toBe(false);
    expect(wrapper.find(".conversation-tool-panel").exists()).toBe(true);
    expect(wrapper.find(".assistant-response-panel").exists()).toBe(true);
    expect(wrapper.get(".assistant-response-panel").text()).toContain(
      "canonical answer without reasoning"
    );
    expect(wrapper.get(".conversation-tool-panel").text()).toContain("read before the answer");

    expect(agentShellEventOrder(wrapper)).toEqual(["tools", "content"]);
  });
});
