import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HomeSessionSidebar from "@/components/HomeSessionSidebar.vue";
import { useRuntimeStore } from "@/stores/runtime";
import type { ChatMessage, SessionOverview } from "@/types/runtime";

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

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

function createMessage(partial: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: partial.id ?? "msg-1",
    turnId: partial.turnId ?? "turn-1",
    role: partial.role ?? "user",
    content: partial.content ?? "hello",
    status: partial.status ?? "done",
    tokenCount: partial.tokenCount ?? null,
    reasoningContent: partial.reasoningContent ?? null,
    modelName: partial.modelName ?? null,
    toolName: partial.toolName ?? null,
    detail: partial.detail ?? null,
    durationSeconds: partial.durationSeconds ?? null
  };
}

function createSession(partial: Partial<SessionOverview> = {}): SessionOverview {
  return {
    conversationId: partial.conversationId ?? "session-1",
    title: partial.title ?? "会话 1",
    summary: partial.summary ?? "摘要",
    turnCount: partial.turnCount ?? 1,
    lastReferencedFile: partial.lastReferencedFile ?? null,
    updatedAtMs: partial.updatedAtMs ?? 1000
  };
}

function mountSidebar() {
  return mount(HomeSessionSidebar, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub
      }
    }
  });
}

describe("HomeSessionSidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("对空白临时会话禁用新建和删除操作", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-transient",
      sessionList: [],
      sessionOperation: null,
      isSubmitting: false,
      messages: []
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(wrapper.text()).toContain("当前为空对话，发送首条消息后才会保存");
    expect(wrapper.text()).toContain("未保存");
    expect(
      wrapper.get('button[title="当前已经是空白新对话，发送首条消息后才会保存到历史。"]').attributes("disabled")
    ).toBeDefined();
    expect(
      wrapper.get('button[title="空白新对话会在切换后自动丢弃，无需单独删除。"]').attributes("disabled")
    ).toBeDefined();
  });

  it("在会话操作期间禁用新建、切换和删除按钮", async () => {
    const runtimeStore = useRuntimeStore();
    runtimeStore.$patch({
      sessionId: "session-current",
      sessionOperation: "deleting",
      isSubmitting: false,
      messages: [createMessage({ content: "已有内容" })],
      sessionList: [
        createSession({
          conversationId: "session-current",
          title: "已保存对话",
          summary: "当前摘要"
        }),
        createSession({
          conversationId: "session-other",
          title: "其他对话",
          summary: "其他摘要",
          updatedAtMs: 2000
        })
      ]
    });

    const wrapper = mountSidebar();
    await nextTick();

    expect(
      wrapper.get('button[title="新建一个空白对话，会保留当前历史会话。"]').attributes("disabled")
    ).toBeDefined();

    const sessionButtons = wrapper
      .findAll("button")
      .filter((button) => button.text().includes("已保存对话") || button.text().includes("其他对话"));
    expect(sessionButtons).toHaveLength(2);
    expect(sessionButtons.every((button) => button.attributes("disabled") !== undefined)).toBe(true);

    const deleteButtons = wrapper.findAll('button[title="删除对话"]');
    expect(deleteButtons).toHaveLength(2);
    expect(deleteButtons.every((button) => button.attributes("disabled") !== undefined)).toBe(true);
  });
});
