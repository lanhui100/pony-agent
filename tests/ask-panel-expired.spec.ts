import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AskPanel from "@/components/AskPanel.vue";
import { useAskStore } from "@/stores/ask";
import type { PendingAsk } from "@/types/ask-plan";

// Wave-15 red-phase acceptance (frozen contract `.dev-team/contracts/wave-15-ask-cas.md`):
// C2-4 — an Ask that is already expired must render every submission control disabled, so the
// user can never trigger the host CAS error that started this wave.

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

function createPendingAsk(partial: Partial<PendingAsk> = {}): PendingAsk {
  return {
    requestId: partial.requestId ?? "ask-1",
    requestKind: partial.requestKind ?? "interaction",
    sessionId: partial.sessionId ?? "session-1",
    runId: partial.runId ?? "run-1",
    turnId: partial.turnId ?? "turn-1",
    callId: partial.callId ?? "call-1",
    descriptorSnapshotId: partial.descriptorSnapshotId ?? "snapshot-1",
    descriptorId: partial.descriptorId ?? "builtin:ask_user",
    finalArgsDigest: partial.finalArgsDigest ?? "args-digest",
    policyDigest: partial.policyDigest ?? "policy-digest",
    nonce: partial.nonce ?? "nonce-1",
    version: partial.version ?? 1,
    expiresAtMs: partial.expiresAtMs ?? Date.now() + 60_000,
    state: partial.state ?? "pending",
    prompt: partial.prompt ?? "继续执行？",
    options: partial.options ?? ["是", "否"]
  };
}

async function flushAsync() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

async function mountWithAsk(ask: PendingAsk) {
  tauriMocks.mockSafeInvoke.mockResolvedValue([ask]);
  const wrapper = mount(AskPanel);
  await flushAsync();
  await vi.waitFor(() => {
    expect(useAskStore().pendingAsks).toHaveLength(1);
  });
  await wrapper.vm.$nextTick();
  return wrapper;
}

describe("AskPanel — expired ask guard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("C2-4: disables the option, answer, and cancel controls for a locally expired ask", async () => {
    // Arrange
    const expired = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() - 1_000,
      options: ["是", "否"]
    });

    // Act
    const wrapper = await mountWithAsk(expired);

    // Assert: every option button is disabled.
    const optionButtons = wrapper.findAll('[data-testid="ask-options"] button');
    expect(optionButtons).toHaveLength(2);
    for (const button of optionButtons) {
      expect(button.attributes("disabled")).toBeDefined();
    }

    // Assert: answer + cancel are disabled.
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="ask-cancel-submit"]').attributes("disabled")).toBeDefined();

    // Assert: the card explains why (an expiry hint naming 「过期」).
    expect(wrapper.text()).toContain("过期");

    wrapper.unmount();
  });

  it("C2-4 (control): a live ask keeps its controls enabled", async () => {
    // Arrange: without this control the disable assertion could pass for the wrong reason.
    const live = createPendingAsk({
      requestId: "ask-1",
      version: 2,
      expiresAtMs: Date.now() + 60_000,
      options: ["是", "否"]
    });

    // Act
    const wrapper = await mountWithAsk(live);

    // Assert
    const optionButtons = wrapper.findAll('[data-testid="ask-options"] button');
    expect(optionButtons).toHaveLength(2);
    for (const button of optionButtons) {
      expect(button.attributes("disabled")).toBeUndefined();
    }
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeUndefined();
    expect(wrapper.get('[data-testid="ask-cancel-submit"]').attributes("disabled")).toBeUndefined();

    wrapper.unmount();
  });

  it("C2-4: an expired ask with no options still disables answer/cancel", async () => {
    // Arrange: free-text Ask (no option chips) must be guarded too.
    const expired = createPendingAsk({
      requestId: "ask-1",
      expiresAtMs: Date.now() - 1,
      options: []
    });

    // Act
    const wrapper = await mountWithAsk(expired);

    // Assert
    expect(wrapper.find('[data-testid="ask-options"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="ask-answer-submit"]').attributes("disabled")).toBeDefined();
    expect(wrapper.get('[data-testid="ask-cancel-submit"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).toContain("过期");

    wrapper.unmount();
  });
});
