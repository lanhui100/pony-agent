/**
 * Wave-55 缺陷独立验收（L2-AT）— deferred persist teardown 确定性红相
 *
 * 缺陷现象（Lead 侦察基线，本文件独立复核行号）：
 *   `npx vitest run` 全量偶发 `ReferenceError: window is not defined`，
 *   栈指向 `src/stores/runtime.ts:904`（`scheduleDeferredPersist` 裸 `window.setTimeout`），
 *   该调用来自 `src/lib/runtime/utils.ts:167-193` `runLowPriorityTurnWork` 的 800ms 宏任务
 *   （`utils.ts:176-179`：回调到点时若 `resolveBrowserWindow()` 返回 null，**仍然执行
 *   `callback()` 而不是中止**），而 `src/stores/runtime.ts:4342-4363`（turn:completed
 *   STAGE 3）正是把 `this.scheduleDeferredPersist()` 交给该低优先级回调。
 *
 * 验收契约（红相必须失败，绿相必须通过）：
 *   T1 机制复现：用 fake window 捕获 800ms 低优先级回调 → 模拟 jsdom teardown
 *      （`destroyBrowserWindow()`：window 全局绑定消失，`typeof window === "undefined"`）
 *      → 手动执行该回调：**不得抛异常**，且**回调体不得被执行**。
 *      （修复前：抛 `ReferenceError: window is not defined` @ runtime.ts:904，栈含 utils.ts:177）
 *   T2 store 层环境安全：window 已销毁时 `scheduleDeferredPersist()` 不得抛异常、不得写入
 *      `deferredPersistTimerId`（保持 null）；`cancelDeferredPersist()` 在 `deferredPersistTimerId`
 *      非 null 且 window 已销毁时不得抛异常、不得残留该 id。
 *   T3 真实语义不退化：窗口存活时 `scheduleDeferredPersist(0)` 仍必须真实调度，并在（fake window
 *      捕获的）定时器回调中真正落一次持久化（`persistHistory` + localStorage 写入）。
 *
 * 纪律（test-expert / 零假测试）：
 *   - 全部确定性：无 sleep、无概率重试、无 skip/todo、无 `--silent`、不改任何配置；
 *   - 断言均验证状态突变或异常逃逸，禁止同义反复；
 *   - 只读契约：本文件由 Test Agent（test-agent-wave55）独占写域，红相收据落盘后 chmod a-w 冻结，
 *     实施者（Executor）不得修改；不得为变绿而改 src/、vitest.config.ts 或既有 spec。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { runLowPriorityTurnWork } from "@/lib/runtime/utils";
import { RUNTIME_STORAGE_KEY } from "@/lib/runtime/constants";
import { useRuntimeStore } from "@/stores/runtime";
import type { ChatMessage } from "@/types/runtime";

// 与 `src/lib/runtime/utils.ts:164` `LOW_PRIORITY_TURN_WORK_DELAY_MS` 对齐（契约常量，不是魔法数）。
const LOW_PRIORITY_TURN_WORK_DELAY_MS = 800;
// 与 `src/stores/runtime.ts:902` `scheduleDeferredPersist(delay = 140)` 默认延迟对齐。
const DEFAULT_DEFERRED_PERSIST_DELAY_MS = 140;

type CapturedTimer = {
  readonly id: number;
  readonly delay: number;
  readonly callback: () => void;
};

type FakeBrowserWindow = {
  readonly location: { search: string };
  readonly localStorage: Storage;
  readonly requestAnimationFrame: ReturnType<typeof vi.fn>;
  readonly setTimeout: ReturnType<typeof vi.fn>;
  readonly clearTimeout: ReturnType<typeof vi.fn>;
};

/**
 * 构造「捕获式」fake window：`setTimeout` 只登记回调不真正计时，`clearTimeout` 反登记。
 * 这样 800ms / 0ms / 140ms 三个阶段的宏任务都在测试进程内被确定性捕获与手动驱动。
 * `localStorage` 复用 jsdom 真实 Storage，保证 T3 验证的是真实持久化写入而非桩替身。
 */
function createFakeBrowserWindow(): { fakeWindow: FakeBrowserWindow; timers: CapturedTimer[] } {
  const timers: CapturedTimer[] = [];
  const storage = window.localStorage;
  let nextTimerId = 1;

  const fakeWindow: FakeBrowserWindow = {
    // `debugLog` → `isDebugLoggingEnabled`（utils.ts:29）会读 `window.location.search`。
    location: { search: "" },
    localStorage: storage,
    // `runLowPriorityTurnWork` 仅用 `typeof requestAnimationFrame === "function"` 作为
    // 「浏览器窗口存活」判据（utils.ts:169），因此这里只需提供一个可调用函数。
    requestAnimationFrame: vi.fn((handler: FrameRequestCallback) => {
      void handler;
      return nextTimerId++;
    }),
    setTimeout: vi.fn((callback: () => void, delay?: number) => {
      const timer: CapturedTimer = {
        id: nextTimerId++,
        delay: typeof delay === "number" ? delay : 0,
        callback
      };
      timers.push(timer);
      return timer.id;
    }),
    clearTimeout: vi.fn((timerId?: number) => {
      const index = timers.findIndex((timer) => timer.id === timerId);
      if (index >= 0) {
        timers.splice(index, 1);
      }
    })
  };

  return { fakeWindow, timers };
}

/**
 * 确定性模拟 vitest/jsdom 的 environment teardown。
 *
 * 真实 teardown 不是把 `window` 赋成 undefined，而是**移除全局绑定**，因此缺陷现场报的是
 * `ReferenceError: window is not defined`（而不是 `TypeError: Cannot read properties of undefined`）。
 * 这里两步叠加，逐字复刻现场：
 *   1. `vi.stubGlobal("window", undefined)`：`typeof window === "undefined"`（
 *      `resolveBrowserWindow()` @ utils.ts:136-138 与 store 内所有 `typeof window` 判据的触发条件）；
 *   2. `delete globalThis.window`：裸标识符 `window` 的绑定消失，
 *      `window.setTimeout` / `window.clearTimeout` 解引用即抛
 *      `ReferenceError: window is not defined`（runtime.ts:904 / runtime.ts:741 现场原文）。
 * `afterEach` 的 `vi.unstubAllGlobals()` 负责把原始 jsdom window 还原。
 */
function destroyBrowserWindow(): void {
  vi.stubGlobal("window", undefined);
  delete (globalThis as { window?: unknown }).window;
}

/**
 * 执行 act 并把「逃逸异常」转成带原始 name/message/stack 的断言失败。
 * 这样红相收据里保留的是缺陷原文（`ReferenceError: window is not defined`）与真实栈帧，
 * 而不是被 expect 包装掉的摘要。
 */
function actAndRequireNoThrow(label: string, act: () => void): void {
  try {
    act();
  } catch (error) {
    const detail =
      error instanceof Error
        ? `${error.name}: ${error.message}\n${error.stack ?? "<no stack>"}`
        : String(error);
    throw new Error(`[wave55] ${label}\n原始异常：${detail}`);
  }
}

function buildPersistableMessage(): ChatMessage {
  return {
    id: "wave55-user-message",
    turnId: "wave55-turn",
    role: "user",
    content: "wave55 deferred persist probe",
    status: "done"
  };
}

beforeEach(() => {
  vi.unstubAllGlobals();
  setActivePinia(createPinia());
  window.localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("wave55 deferred persist teardown（确定性红相）", () => {
  it("T1 机制复现：环境销毁后，800ms 低优先级回调不得执行 store.scheduleDeferredPersist", () => {
    // ── Arrange ────────────────────────────────────────────────────────────
    const store = useRuntimeStore();
    const { fakeWindow, timers } = createFakeBrowserWindow();
    // 与 src/stores/runtime.ts:4344（STAGE 3）一致的最短链路：
    // 低优先级回调体第一件事就是 scheduleDeferredPersist()。
    const onWork = vi.fn(() => {
      store.scheduleDeferredPersist();
    });

    vi.stubGlobal("window", fakeWindow);
    runLowPriorityTurnWork(onWork);
    expect(timers).toHaveLength(1);
    expect(timers[0].delay).toBe(LOW_PRIORITY_TURN_WORK_DELAY_MS);
    expect(onWork).not.toHaveBeenCalled();

    // 模拟 jsdom teardown：window 从全局消失（`typeof window === "undefined"`）。
    destroyBrowserWindow();

    // ── Act ────────────────────────────────────────────────────────────────
    // 把 800ms 定时器「到点」这一物理时刻确定性推进，不 sleep。
    actAndRequireNoThrow("环境销毁后 800ms 低优先级回调不得抛异常", () => {
      timers[0].callback();
    });

    // ── Assert ─────────────────────────────────────────────────────────────
    // 修复后：utils.runLowPriorityTurnWork 在回调到点时若窗口已消失必须中止，不得执行 callback。
    expect(onWork).not.toHaveBeenCalled();
    expect(store.deferredPersistTimerId).toBeNull();
  });

  it("T2a 环境销毁后 scheduleDeferredPersist 不得解引用 window，也不得写入 deferredPersistTimerId", () => {
    // ── Arrange ────────────────────────────────────────────────────────────
    const store = useRuntimeStore();
    expect(store.deferredPersistTimerId).toBeNull();
    destroyBrowserWindow();

    // ── Act ────────────────────────────────────────────────────────────────
    actAndRequireNoThrow("window 已销毁时 scheduleDeferredPersist 不得抛异常", () => {
      store.scheduleDeferredPersist();
    });

    // ── Assert ─────────────────────────────────────────────────────────────
    // 不得残留一个「指向不存在的 window 定时器」的 id（跨 teardown 存活会让后续
    // cancelDeferredPersist 再次解引用 window）。
    expect(store.deferredPersistTimerId).toBeNull();
  });

  it("T2b 环境销毁后 cancelDeferredPersist 不得抛异常，且不得残留 deferredPersistTimerId", () => {
    // ── Arrange ────────────────────────────────────────────────────────────
    const store = useRuntimeStore();
    // 模拟「跨 teardown 存活」的定时器 id（例如旧 jsdom 窗口登记的宏任务 id）。
    store.deferredPersistTimerId = 4711;
    destroyBrowserWindow();

    // ── Act ────────────────────────────────────────────────────────────────
    actAndRequireNoThrow("window 已销毁时 cancelDeferredPersist 不得抛异常", () => {
      store.cancelDeferredPersist();
    });

    // ── Assert ─────────────────────────────────────────────────────────────
    expect(store.deferredPersistTimerId).toBeNull();
  });

  it("T2c 对照：窗口存活时同一条 800ms 回调可正常执行（证明捕获手法真实、非空转）", () => {
    // ── Arrange ────────────────────────────────────────────────────────────
    const store = useRuntimeStore();
    const { fakeWindow, timers } = createFakeBrowserWindow();
    const onWork = vi.fn(() => {
      store.scheduleDeferredPersist();
    });

    vi.stubGlobal("window", fakeWindow);
    runLowPriorityTurnWork(onWork);
    expect(timers).toHaveLength(1);
    expect(timers[0].delay).toBe(LOW_PRIORITY_TURN_WORK_DELAY_MS);

    // ── Act ────────────────────────────────────────────────────────────────
    // 800ms 阶段：窗口存活 → 无 requestIdleCallback 时再调度 0ms 宏任务（utils.ts:192）。
    timers[0].callback();
    if (onWork.mock.calls.length === 0) {
      const followUp = timers[timers.length - 1];
      expect(followUp.delay).toBe(0);
      followUp.callback();
    }

    // ── Assert ─────────────────────────────────────────────────────────────
    expect(onWork).toHaveBeenCalledTimes(1);
    // onWork 内 scheduleDeferredPersist() 必须真实向 window 登记 140ms 持久化定时器。
    expect(store.deferredPersistTimerId).not.toBeNull();
    expect(fakeWindow.setTimeout).toHaveBeenLastCalledWith(expect.any(Function), DEFAULT_DEFERRED_PERSIST_DELAY_MS);
  });

  it("T3 窗口存活时 scheduleDeferredPersist(0) 仍真实调度，并在定时器回调中落一次持久化", () => {
    // ── Arrange ────────────────────────────────────────────────────────────
    const store = useRuntimeStore();
    store.messages = [buildPersistableMessage()];
    const persistSpy = vi.spyOn(store, "persistHistory");
    const { fakeWindow, timers } = createFakeBrowserWindow();

    vi.stubGlobal("window", fakeWindow);

    // ── Act ────────────────────────────────────────────────────────────────
    store.scheduleDeferredPersist(0);

    // ── Assert（调度语义）────────────────────────────────────────────────────
    expect(fakeWindow.setTimeout).toHaveBeenCalledTimes(1);
    expect(timers).toHaveLength(1);
    expect(timers[0].delay).toBe(0);
    expect(store.deferredPersistTimerId).toBe(timers[0].id);
    expect(persistSpy).not.toHaveBeenCalled();

    // ── Act（推进定时器到点）─────────────────────────────────────────────────
    timers[0].callback();

    // ── Assert（真实持久化语义，防止修复退化为「永远 no-op」）─────────────────
    expect(persistSpy).toHaveBeenCalledTimes(1);
    expect(store.deferredPersistTimerId).toBeNull();

    const raw = fakeWindow.localStorage.getItem(RUNTIME_STORAGE_KEY);
    expect(raw, `localStorage[${RUNTIME_STORAGE_KEY}] 必须被真实写入`).toBeTruthy();
    const cache = JSON.parse(raw as string) as {
      sessions: Record<string, { messages?: Array<{ content?: string }> }>;
    };
    expect(Object.keys(cache.sessions)).toContain(store.sessionId);
    expect(cache.sessions[store.sessionId]?.messages?.[0]?.content).toBe("wave55 deferred persist probe");
  });
});
