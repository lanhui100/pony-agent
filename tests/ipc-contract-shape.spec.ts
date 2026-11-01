/**
 * W56-T④ IPC 契约漂移检测（前端侧）。
 *
 * 目标：前端经 `safeInvoke(command, args)` 下发的**命令名与参数键形状**必须与 Rust 侧
 * `#[tauri::command]` 的形参名 / `#[serde(rename_all = "camelCase")]` 的字段名一致。
 * 这类漂移（前端 snake_case、Rust camelCase，或字段改名一侧忘改）在运行时只表现为
 * "参数反序列化失败/字段丢失"，任何 crate 单测都发现不了 —— 本文件是唯一自动发现点。
 *
 * 断言口径（机器可判定）：
 *   1. 命令名精确匹配；
 *   2. 参数对象的**每一层键**都是 camelCase（不得出现 `_` 分隔的 snake_case 键）；
 *   3. 关键字段确实存在（漏字段 = 漂移）。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useRuntimeStore } from "@/stores/runtime";
import { useProviderStore } from "@/stores/providers";

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

/** 递归收集所有非 camelCase（含下划线）的键路径；空数组 = 形状合规。 */
function snakeCaseKeys(value: unknown, path = "$"): string[] {
  if (value === null || typeof value !== "object") {
    return [];
  }
  if (Array.isArray(value)) {
    return value.flatMap((item, index) => snakeCaseKeys(item, `${path}[${index}]`));
  }
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) => {
    const here = /_/.test(key) ? [`${path}.${key}`] : [];
    return [...here, ...snakeCaseKeys(child, `${path}.${key}`)];
  });
}

function callsOf(command: string) {
  return tauriMocks.mockSafeInvoke.mock.calls.filter(([name]) => name === command);
}

describe("IPC 参数契约（camelCase ↔ serde rename_all）", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeListen.mockResolvedValue(vi.fn());
    tauriMocks.mockSafeInvoke.mockReset();
    setActivePinia(createPinia());
  });

  it("list_sessions：命令名正确且不携带非法参数形状", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "list_sessions") {
        return [];
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const store = useRuntimeStore();
    await store.loadSessionCatalog();

    const calls = callsOf("list_sessions");
    expect(calls.length, "list_sessions 必须被调用一次").toBe(1);
    const args = calls[0]?.[1];
    expect(
      snakeCaseKeys(args),
      `list_sessions 不该携带 snake_case 参数键（Rust 侧无参数），实际 ${JSON.stringify(args)}`
    ).toEqual([]);
  });

  it("load_session_runtime_view：sessionId/nodeId 必须是 camelCase", async () => {
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "load_session_runtime_view") {
        return {
          session: { conversationId: "ipc-session-1" },
          messages: [],
          history: { nodes: [], current: null },
          turnTraceHistory: []
        };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const store = useRuntimeStore();
    await store.loadSessionRuntimeViewState("ipc-session-1");

    const calls = callsOf("load_session_runtime_view");
    expect(calls.length, "load_session_runtime_view 必须被调用").toBe(1);
    const args = calls[0]?.[1] as Record<string, unknown>;
    expect(args, "必须使用 sessionId（camelCase），而不是 session_id").toHaveProperty("sessionId");
    expect(args.sessionId).toBe("ipc-session-1");
    expect(
      snakeCaseKeys(args),
      `load_session_runtime_view 参数必须全 camelCase，实际 ${JSON.stringify(args)}`
    ).toEqual([]);
  });

  it("start_graph_run_stream：turnId/runId/goal/input 全 camelCase，input 内层亦然", async () => {
    const runtimeStore = useRuntimeStore();
    const providerStore = useProviderStore();
    providerStore.$patch({
      providers: [
        {
          id: "provider-ipc",
          name: "OpenAI",
          protocol: "openai-completions",
          baseUrl: "https://api.openai.com/v1",
          apiKeyEnvVar: "OPENAI_API_KEY",
          apiKeyValue: "",
          apiKeyPresent: false,
          selectedModelId: "model-ipc",
          models: [
            {
              id: "model-ipc",
              name: "GPT-IPC",
              model: "gpt-ipc",
              temperature: 0,
              maxOutputTokens: 4096,
              reasoningEffort: "medium",
              reasoningBudgetTokens: null,
              capabilityPreset: "openai-standard",
              capabilities: {
                contextWindowTokens: 128000,
                supportsTools: true,
                supportsStreaming: true,
                supportsImageInput: false,
                supportsReasoning: false
              }
            }
          ]
        }
      ],
      selectedReasoningEffort: "medium"
    });
    providerStore.selectModel("provider-ipc", "model-ipc");

    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) => {
      if (command === "resolve_graph_run_submission_plan") {
        return { command: "start_graph_run_stream", runId: null };
      }
      if (command === "start_graph_run_stream") {
        return { run: { id: "run-ipc" }, turnId: "9" };
      }
      throw new Error(`unexpected command: ${command}`);
    });

    runtimeStore.$patch({
      sessionId: "ipc-session-2",
      draftMessage: "ipc contract probe",
      phase: "ready",
      messages: []
    });

    await runtimeStore.submitTurn();

    const calls = callsOf("start_graph_run_stream");
    expect(calls.length, "start_graph_run_stream 必须被调用一次").toBe(1);
    const args = calls[0]?.[1] as Record<string, unknown>;

    expect(args, "必须携带 turnId（camelCase）").toHaveProperty("turnId");
    expect(args, "必须携带 runId（camelCase）").toHaveProperty("runId");
    expect(args, "必须携带 goal").toHaveProperty("goal");
    expect(args, "必须携带 input").toHaveProperty("input");
    expect(
      snakeCaseKeys(args),
      `start_graph_run_stream 参数（含 input 内层 TurnInput）必须全 camelCase，实际 ${JSON.stringify(args)}`
    ).toEqual([]);
    // TurnInput 的会话字段必须与 Rust `#[serde(rename_all = "camelCase")]` 对齐。
    const input = args.input as Record<string, unknown>;
    expect(input, "TurnInput 必须带 sessionId（camelCase）").toHaveProperty("sessionId");
  });

  it("契约漂移探针自检：snake_case 键必须被识别（防止本文件断言恒真）", () => {
    expect(snakeCaseKeys({ session_id: "x", nested: { turn_id: "y" } })).toEqual([
      "$.session_id",
      "$.nested.turn_id"
    ]);
    expect(snakeCaseKeys({ sessionId: "x", nested: { turnId: "y" } })).toEqual([]);
  });
});
