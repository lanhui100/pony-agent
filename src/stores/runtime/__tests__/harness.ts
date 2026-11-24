/**
 * runtime store 行为等价测试 · 共享夹具（Wave-1 Test Agent）
 *
 * 目标：把 `src/stores/runtime.ts` 的**可观测行为**冻结为 golden fixtures，作为拆解后的等价判据。
 * 本文件只提供测试脚手架，不含任何场景断言：
 *
 * 1. `FakeBackend`：`@/lib/tauri` 的 `safeInvoke` / `safeListen` 的可脚本化假后端
 *    （按命令路由 + 事件句柄登记/卸载计数），全链路零真实 IPC。
 * 2. `setupRuntimeHarness()`：每个用例的确定性环境（假定时器 + 固定时钟 + 去随机 + localStorage 清空）。
 * 3. `runtimeSnapshot()`：把 store 的 84 个状态键按 8 域投影成可序列化快照
 *    （含 getter 求值与落盘 payload），供 fixtures 逐字比对。
 * 4. `expectGolden()`：golden-master 比对（`UPDATE_RUNTIME_FIXTURES=1` 时由当前实现重新冻结）。
 *
 * 确定性纪律（防 Flaky）：
 * - 禁止任何真实 sleep：一律 `vi.useFakeTimers()` + `advanceTimers()`（固定次数/固定毫秒）。
 * - 时钟冻结：`BASE_TIME`；随机源冻结：`Math.random` 递增序列。
 * - 计时器句柄不进入 fixtures（真值随 sinon 内部计数漂移），统一投影为 `null | "pending"`。
 */
import { afterEach, beforeEach, expect, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { RUNTIME_STORAGE_KEY } from "@/lib/runtime/constants";
import { useRuntimeStore } from "@/stores/runtime";

/** 冻结时钟（假定时器安装时刻）。 */
export const BASE_TIME = 1_760_000_000_000;

/** 主测试会话 id（多数用例的落点）。 */
export const SESSION_ID = "rt-session-main";

export type TauriMocks = {
  mockSafeInvoke: ReturnType<typeof vi.fn>;
  mockSafeListen: ReturnType<typeof vi.fn>;
  mockIsTauriAvailable: ReturnType<typeof vi.fn>;
};

type EventHandler = (event: { payload: unknown }) => void;

export type Invocation = { command: string; args: unknown };

/**
 * 可脚本化假后端：
 * - `route(command, handler)` 注册命令实现（缺省返回 `undefined`，等价于「命令无返回」）；
 * - `emit(eventName, payload)` 把事件下发到所有已登记的监听器；
 * - `listenerCount()` / `unlistenCount` 用于验证订阅生命周期（14 个监听器、dispose 全注销）。
 */
export class FakeBackend {
  private readonly routes = new Map<string, (args: unknown) => unknown>();
  private readonly listeners = new Map<string, Set<EventHandler>>();
  readonly invocations: Invocation[] = [];
  unlistenCount = 0;
  /** 让某条 `safeListen` 挂起（模拟「disposeTurnEvents 在初始化中途被调用」）。 */
  private listenGate: { eventName: string; promise: Promise<void> } | null = null;

  constructor(mocks: TauriMocks, options: { tauriAvailable?: boolean } = {}) {
    const tauriAvailable = options.tauriAvailable ?? true;
    mocks.mockIsTauriAvailable.mockImplementation(() => tauriAvailable);
    mocks.mockSafeInvoke.mockImplementation(async (command: string, args?: unknown) => {
      this.invocations.push({ command, args });
      const route = this.routes.get(command);
      if (!route) {
        return undefined;
      }
      return await route(args);
    });
    mocks.mockSafeListen.mockImplementation(async (eventName: string, handler: EventHandler) => {
      const gate = this.listenGate;
      if (gate && gate.eventName === eventName) {
        await gate.promise;
        this.listenGate = null;
      }
      let handlers = this.listeners.get(eventName);
      if (!handlers) {
        handlers = new Set<EventHandler>();
        this.listeners.set(eventName, handlers);
      }
      handlers.add(handler);
      return () => {
        handlers!.delete(handler);
        this.unlistenCount += 1;
      };
    });
  }

  route(command: string, handler: (args: unknown) => unknown): this {
    this.routes.set(command, handler);
    return this;
  }

  /** 命中一次即可重复使用的静态返回值。 */
  routeValue(command: string, value: unknown): this {
    return this.route(command, () => value);
  }

  emit(eventName: string, payload: unknown): void {
    const handlers = [...(this.listeners.get(eventName) ?? [])];
    for (const handler of handlers) {
      handler({ payload });
    }
  }

  listenerCount(eventName?: string): number {
    if (eventName) {
      return this.listeners.get(eventName)?.size ?? 0;
    }
    let total = 0;
    for (const handlers of this.listeners.values()) {
      total += handlers.size;
    }
    return total;
  }

  listenerNames(): string[] {
    return [...this.listeners.entries()]
      .filter(([, handlers]) => handlers.size > 0)
      .map(([eventName]) => eventName)
      .sort();
  }

  commands(): string[] {
    return this.invocations.map((invocation) => invocation.command);
  }

  callsFor(command: string): Invocation[] {
    return this.invocations.filter((invocation) => invocation.command === command);
  }

  /** 挂起指定事件的监听登记，直到返回的 release() 被调用（确定性「初始化中途」）。 */
  gateNextListen(eventName: string): () => void {
    let release = () => {};
    const promise = new Promise<void>((resolve) => {
      release = resolve;
    });
    this.listenGate = { eventName, promise };
    return release;
  }
}

const nodeEnv = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env ?? {};
const CAPTURE_FIXTURES = nodeEnv.UPDATE_RUNTIME_FIXTURES === "1";

type NodeFs = {
  readFileSync: (path: string, encoding: string) => string;
  writeFileSync: (path: string, data: string) => void;
  existsSync: (path: string) => boolean;
  mkdirSync: (path: string, options?: { recursive?: boolean }) => void;
  readdirSync: (path: string) => string[];
};

type NodeUrl = { fileURLToPath: (url: string | URL) => string };
type NodePath = {
  dirname: (p: string) => string;
  join: (...parts: string[]) => string;
  sep: string;
};

/**
 * 运行时加载 node 内建（本仓 tsconfig 未引入 @types/node，用非字面量说明符规避类型依赖）。
 * 说明符由数组拼接而非字面量，避免 TS 静态解析 `node:*` 类型。
 */
async function loadNodeFs(): Promise<NodeFs> {
  const specifier = ["node", "fs"].join(":");
  return (await import(/* @vite-ignore */ specifier)) as NodeFs;
}

async function loadNodeUrl(): Promise<NodeUrl> {
  const specifier = ["node", "url"].join(":");
  return (await import(/* @vite-ignore */ specifier)) as NodeUrl;
}

let cachedPathModule: NodePath | null = null;
async function loadNodePath(): Promise<NodePath> {
  if (!cachedPathModule) {
    const specifier = ["node", "path"].join(":");
    cachedPathModule = (await import(/* @vite-ignore */ specifier)) as NodePath;
  }
  return cachedPathModule;
}

const FIXTURES_RELATIVE_TO_REPO = ["src", "stores", "runtime", "fixtures"] as const;

let cachedFixturesDir: string | null = null;

/**
 * 解析 fixtures 的**绝对目录**（跨平台，Windows CI 必过）。
 *
 * 候选序：
 *   1. `import.meta.url` → `node:url` 的 `fileURLToPath` → 去掉 `__tests__` → `fixtures`
 *      （Windows `file:///D:/a/…/harness.ts` 会被正确换算为 `D:\a\…`，不再出现 `/D:/…` 前导斜杠）；
 *   2. cwd 兜底 `<process.cwd()>/src/stores/runtime/fixtures`（CI 与本地 vitest 的 cwd 均为仓库根）。
 *
 * 选择规则：优先「存在且含 *.json」的候选；其次「存在」的候选（`UPDATE_RUNTIME_FIXTURES=1`
 * 首次捕获时目录可能为空）；都不存在时返回第一个候选（由调用方报错并打印全部候选）。
 */
async function resolveFixturesDir(): Promise<string> {
  if (cachedFixturesDir) {
    return cachedFixturesDir;
  }
  const fs = await loadNodeFs();
  const path = await loadNodePath();
  const candidates: string[] = [];

  try {
    const { fileURLToPath } = await loadNodeUrl();
    const moduleDir = path.dirname(fileURLToPath(import.meta.url));
    const packageDir = moduleDir.replace(new RegExp(`[\\\\/]__tests__$`), "");
    candidates.push(path.join(packageDir, "fixtures"));
  } catch {
    // import.meta.url 不可解析（非 file: URL 宿主）：直接走 cwd 兜底。
  }

  const cwd = (globalThis as { process?: { cwd?: () => string } }).process?.cwd?.();
  if (cwd) {
    candidates.push(path.join(cwd, ...FIXTURES_RELATIVE_TO_REPO));
  }

  const exists = (dir: string) => {
    try {
      return fs.existsSync(dir);
    } catch {
      return false;
    }
  };
  const hasJson = (dir: string) => {
    try {
      return exists(dir) && fs.readdirSync(dir).some((entry) => entry.endsWith(".json"));
    } catch {
      return false;
    }
  };

  cachedFixturesDir =
    candidates.find((dir) => hasJson(dir)) ??
    candidates.find((dir) => exists(dir)) ??
    candidates[0]!;
  return cachedFixturesDir;
}

/** 暴露解析结果供失败文案/诊断复用（不改变断言强度）。 */
export async function resolveFixturesDirForDiagnostics(): Promise<{
  dir: string;
  exists: boolean;
  candidates: string[];
}> {
  const fs = await loadNodeFs();
  const path = await loadNodePath();
  const dir = await resolveFixturesDir();
  const cwd = (globalThis as { process?: { cwd?: () => string } }).process?.cwd?.();
  const candidates: string[] = [];
  try {
    const { fileURLToPath } = await loadNodeUrl();
    const moduleDir = path.dirname(fileURLToPath(import.meta.url));
    candidates.push(path.join(moduleDir.replace(new RegExp(`[\\\\/]__tests__$`), ""), "fixtures"));
  } catch {
    candidates.push("<import.meta.url 不可解析>");
  }
  if (cwd) {
    candidates.push(path.join(cwd, ...FIXTURES_RELATIVE_TO_REPO));
  }
  let exists = false;
  try {
    exists = fs.existsSync(dir);
  } catch {
    exists = false;
  }
  return { dir, exists, candidates };
}

/**
 * golden-master 比对：fixtures 由**当前实现**跑出并冻结（`UPDATE_RUNTIME_FIXTURES=1`），
 * 正常运行时做双重比对 —— 结构 `toEqual`（可读 diff）+ 逐字 `toBe`（字节锁）。
 */
export async function expectGolden(name: string, actual: unknown): Promise<void> {
  const fs = await loadNodeFs();
  const path = await loadNodePath();
  const dir = await resolveFixturesDir();
  const file = path.join(dir, `${name}.json`);
  const text = `${JSON.stringify(actual, null, 2)}\n`;

  if (CAPTURE_FIXTURES) {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(file, text);
  }

  const diagnostics = await resolveFixturesDirForDiagnostics();
  const dirExists = fs.existsSync(dir);
  const fileExists = fs.existsSync(file);
  expect(
    fileExists,
    [
      `缺少 golden fixture：${name}.json（先以 UPDATE_RUNTIME_FIXTURES=1 冻结）`,
      `  解析到的 fixtures 目录：${dir}`,
      `  该目录是否存在：${dirExists}`,
      `  目标文件：${file}`,
      `  候选目录：${JSON.stringify(diagnostics.candidates)}`
    ].join("\n")
  ).toBe(true);
  const expectedText = fs.readFileSync(file, "utf8");
  expect(actual).toEqual(JSON.parse(expectedText));
  expect(text).toBe(expectedText);
}

/**
 * 每个用例的确定性环境。返回的对象在整个 describe 内复用（`getStore` / `backend` 在用例中可读）。
 */
export function setupRuntimeHarness(tauriMocks: TauriMocks, options: { tauriAvailable?: boolean } = {}) {
  const env: {
    backend: FakeBackend;
    store: ReturnType<typeof useRuntimeStore> | null;
    randomSeed: { value: number };
  } = {
    backend: null as unknown as FakeBackend,
    store: null,
    randomSeed: { value: 0 }
  };

  const { beforeEach: registerBeforeEach, afterEach: registerAfterEach } = { beforeEach, afterEach };

  registerBeforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    vi.useFakeTimers({ now: BASE_TIME });
    // 去随机：Math.random 供 queued-* id 使用；固定递增序列保证 id 可复现。
    env.randomSeed.value = 0;
    vi.spyOn(Math, "random").mockImplementation(() => {
      env.randomSeed.value = (env.randomSeed.value + 1) % 1_000_000;
      return env.randomSeed.value / 1_000_000;
    });
    env.backend = new FakeBackend(tauriMocks, options);
    env.store = useRuntimeStore();
  });

  registerAfterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  return {
    backend: (): FakeBackend => env.backend,
    getStore: () => {
      if (!env.store) {
        throw new Error("store 尚未在 beforeEach 中初始化");
      }
      return env.store;
    }
  };
}

/** 推进假定时器（唯一允许的「等待」方式；禁止真实 sleep）。 */
export async function advance(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
}

/** 排空微任务队列（覆盖 fire-and-forget 的 `void this.loadSessionState(...)` 等）。 */
export async function drainMicrotasks(rounds = 12): Promise<void> {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
  }
}

/** 读取持久化容器（`pony-agent.runtime-history.v1`）。 */
export function readPersistedCache(): Record<string, unknown> | null {
  const raw = window.localStorage.getItem(RUNTIME_STORAGE_KEY);
  return raw ? (JSON.parse(raw) as Record<string, unknown>) : null;
}

/**
 * 夹具构造专用：把种子字段批量写入 store。
 *
 * 种子字段集合由 golden fixtures 冻结（含跨域混合字段），逐字段类型收窄只会制造噪音，
 * 故用函数式 `$patch` + `Object.assign` 写入；**仅用于 Arrange 阶段造场景**，
 * 断言侧仍然逐字段读取真实 store 状态，不削弱任何断言。
 */
export function seedStore(store: ReturnType<typeof useRuntimeStore>, seed: Record<string, unknown>): void {
  store.$patch((state) => {
    Object.assign(state as Record<string, unknown>, seed);
  });
}

/** 落盘面快照：指定会话 payload + 运行态映射（`persistHistory` 的完整产物）。 */
export function persistedSnapshot(sessionId: string) {
  const cache = readPersistedCache();
  const sessions = (cache?.sessions ?? {}) as Record<string, unknown>;
  return {
    sessionKeys: Object.keys(sessions).sort(),
    payload: sessions[sessionId] ?? null,
    runningSessionMap: cache?.runningSessionMap ?? null,
    completedSet: cache?.completedSet ?? null,
    failedSet: cache?.failedSet ?? null
  };
}

function timerState(value: unknown): null | "pending" {
  return value == null ? null : "pending";
}

type RuntimeStore = ReturnType<typeof useRuntimeStore> & Record<string, any>;

/**
 * store 全量可观测快照：84 个状态键按 8 域投影 + 全部关键 getter 求值 + 落盘 payload。
 * 只剔除进程内计时器句柄（改为 `null | "pending"`），其余一律逐字冻结。
 */
export function runtimeSnapshot(rawStore: ReturnType<typeof useRuntimeStore>) {
  const store = rawStore as RuntimeStore;
  const snapshot = {
    d1_session: {
      sessionId: store.sessionId,
      sessionList: store.sessionList,
      sessionOperation: store.sessionOperation,
      sessionHydrating: store.sessionHydrating,
      deletingSessionSet: store.deletingSessionSet,
      sessionSwitchToken: store.sessionSwitchToken,
      sessionError: store.sessionError,
      sessionNotice: store.sessionNotice,
      runningSessionMap: store.runningSessionMap,
      completedSessionSet: store.completedSessionSet,
      failedSessionSet: store.failedSessionSet
    },
    d3_turn: {
      phase: store.phase,
      phaseLabel: store.phaseLabel,
      isSubmitting: store.isSubmitting,
      activeTurnId: store.activeTurnId,
      activeTurnStartedAtMs: store.activeTurnStartedAtMs,
      activeRunId: store.activeRunId,
      activeStreamKind: store.activeStreamKind,
      retryProgress: store.retryProgress,
      browserPreviewRunToken: store.browserPreviewRunToken,
      streamBufferTurnId: store.streamBufferTurnId,
      streamBufferText: store.streamBufferText,
      streamBufferReasoning: store.streamBufferReasoning,
      eventsReady: store.eventsReady,
      streamDebugDeltaCount: store.streamDebugDeltaCount,
      streamDebugFlushCount: store.streamDebugFlushCount,
      streamDebugTextCharsReceived: store.streamDebugTextCharsReceived,
      streamDebugTextCharsFlushed: store.streamDebugTextCharsFlushed,
      timers: {
        deferredPersistTimerId: timerState(store.deferredPersistTimerId),
        streamFlushTimerId: timerState(store.streamFlushTimerId),
        traceTimelineThrottleTimerId: timerState(store.traceTimelineThrottleTimerId),
        submissionWatchdogTimerId: timerState(store.submissionWatchdogTimerId)
      }
    },
    d4_messages: {
      messages: store.messages,
      draftMessage: store.draftMessage,
      pendingAttachments: store.pendingAttachments,
      attachmentAssets: store.attachmentAssets,
      sessionSummary: store.sessionSummary,
      error: store.error,
      toolActivities: store.toolActivities,
      traceSteps: store.traceSteps,
      messageRevision: store.messageRevision,
      retrievedContext: store.retrievedContext,
      recallLastUserMessage: store.recallLastUserMessage(),
      agentRuntimeStatus: store.agentRuntimeStatus,
      failedTurnRetry: store.failedTurnRetry,
      canSubmitPrompt: store.canSubmitPrompt,
      isInputDisabled: store.isInputDisabled
    },
    d5_telemetry: {
      traceTimeline: store.traceTimeline,
      turnTraceHistory: store.turnTraceHistory,
      eventCursorByTurnId: store.eventCursorByTurnId,
      liveMetrics: store.liveMetrics,
      activeProviderCallRecords: store.activeProviderCallRecords,
      providerRequestedName: store.providerRequestedName,
      providerName: store.providerName,
      providerProtocol: store.providerProtocol,
      providerModel: store.providerModel,
      providerSource: store.providerSource,
      providerMode: store.providerMode,
      fallbackReason: store.fallbackReason,
      inputTokens: store.inputTokens,
      outputTokens: store.outputTokens,
      totalTokens: store.totalTokens,
      firstTokenLatencyMs: store.firstTokenLatencyMs,
      activeTraceId: store.activeTraceId,
      sessionTotalCostUsd: store.sessionTotalCostUsd,
      hasUnpricedTurns: store.hasUnpricedTurns,
      formattedSessionCost: store.formattedSessionCost,
      formattedGovernanceCost: store.formattedGovernanceCost,
      usageGovernanceSummary: store.usageGovernanceSummary
    },
    d6_history: {
      visibleNodeId: store.visibleNodeId,
      branchHeadNodeId: store.branchHeadNodeId,
      activeBranchId: store.activeBranchId,
      cursorVersion: store.cursorVersion,
      historyCursorMode: store.historyCursorMode,
      historyNodes: store.historyNodes,
      historyBranches: store.historyBranches,
      initialRollbackActive: store.initialRollbackActive,
      latestExecutionCheckpoint: store.latestExecutionCheckpoint,
      latestGraphRunSubmissionPlan: store.latestGraphRunSubmissionPlan,
      latestGraphRunControlBoundaryEvidence: store.latestGraphRunControlBoundaryEvidence,
      latestRunControlAuditSummary: store.latestRunControlAuditSummary,
      latestHistoryStateAuditSummary: store.latestHistoryStateAuditSummary,
      isHistoricalMode: store.isHistoricalMode,
      conversationCheckpointEntries: store.conversationCheckpointEntries
    },
    d7_queue: {
      pendingQueuedMessages: store.pendingQueuedMessages,
      queueDeliveryMode: store.queueDeliveryMode,
      toolAuthorizationMode: store.toolAuthorizationMode
    },
    d8_catalog: {
      health: store.health,
      availableToolNames: store.availableTools.map((tool: { name: string }) => tool.name),
      capabilitySourceIds: store.capabilitySources.map((source: { sourceId: string }) => source.sourceId),
      capabilityIds: store.capabilities.map((capability: { capabilityId: string }) => capability.capabilityId)
    },
    persistence: persistedSnapshot(store.sessionId)
  };

  return JSON.parse(JSON.stringify(snapshot)) as Record<string, unknown>;
}
