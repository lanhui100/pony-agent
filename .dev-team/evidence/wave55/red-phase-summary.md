# Wave-55 独立验收（L2-AT）红相收据 — deferred persist teardown

- 测试文件（已冻结 444）：`tests/acceptance/wave55/deferred-persist-teardown.spec.ts`
  - sha256 `8027fe32a8c150d9b06bac9474de885074260712fdaeb104c0fa59583558b2da`（冻结前）
- 红相原始日志：`.dev-team/evidence/wave55/red-phase.log`（sha256 `dd556dd38ce3ecb697734219329778a999de669845bd1dfe3247b0da8cc520e0`）
- 绿相可行性模拟（隔离副本 `/tmp/wave55-green-sim`，**未改动仓库 src/**）：`.dev-team/evidence/wave55/green-phase-simulation.log`

## 1. 红相确定性（同一命令连跑 3 次）

命令：`npx vitest run tests/acceptance/wave55/deferred-persist-teardown.spec.ts`

| Run | 时间 | 结果 | EXIT |
| --- | --- | --- | --- |
| 1/3 | 2026-10-27T22:26:39+08:00 | 3 failed / 2 passed（5） | 1 |
| 2/3 | 2026-10-27T22:26:41+08:00 | 3 failed / 2 passed（5） | 1 |
| 3/3 | 2026-10-27T22:26:44+08:00 | 3 failed / 2 passed（5） | 1 |

固定失败集：`T1`（机制复现）、`T2a`（scheduleDeferredPersist）、`T2b`（cancelDeferredPersist）。
固定通过集（对照/防退化）：`T2c`（窗口存活时 800ms 回调可执行）、`T3`（真实调度 + localStorage 落盘）。

## 2. 失败断言原文（三次一致）

```
Error: [wave55] 环境销毁后 800ms 低优先级回调不得抛异常
原始异常：ReferenceError: window is not defined
    at Proxy.scheduleDeferredPersist (/home/dm/pony-agent/src/stores/runtime.ts:904:37)
    at Object.callback (/home/dm/pony-agent/src/lib/runtime/utils.ts:177:7)
```

```
Error: [wave55] window 已销毁时 scheduleDeferredPersist 不得抛异常
原始异常：ReferenceError: window is not defined
    at Proxy.scheduleDeferredPersist (/home/dm/pony-agent/src/stores/runtime.ts:904:37)
```

```
Error: [wave55] window 已销毁时 cancelDeferredPersist 不得抛异常
原始异常：ReferenceError: window is not defined
    at Proxy.cancelDeferredPersist (/home/dm/pony-agent/src/stores/runtime.ts:741:7)
```

## 3. 根因归类（独立复核，path:line）

1. **主因（jsdom 销毁后回调仍执行）**：`src/lib/runtime/utils.ts:174-179`
   `safeSetTimeout(..., 800)` 的回调到点时 `resolveBrowserWindow()` 返回 null（window 绑定已消失），
   当前实现走 `callback()`（`:177`）而不是中止。`typeof window === "undefined"` 对已删除的全局绑定
   是安全的，因此 utils 层不报错，把错误「下传」到 store。
2. **爆点（store 裸 window 解引用）**：`src/stores/runtime.ts:902-908`
   `scheduleDeferredPersist(delay=140)` 直接 `window.setTimeout`（`:904`），无环境守卫。
3. **次爆点（cancel 同样裸解引用）**：`src/stores/runtime.ts:736-743`
   `cancelDeferredPersist()` 直接 `window.clearTimeout`（`:741`）；且 `:903` 在 set 之前先 cancel，
   任一解引用抛错都会打断整条持久化链路。
4. **使能条件（store 定时器未清理 / teardown 顺序）**：`deferredPersistTimerId` 仅由
   `persistHistory()`（`:1046`）与 `cancelDeferredPersist()` 清理，store 无 dispose/teardown 钩子；
   真实触发链在生产是 `turn:completed` STAGE 3（`:4342-4363` → `:4344`）经 `runLowPriorityTurnWork`
   排出的 800ms 宏任务跨文件 teardown 存活；既有测试 `tests/runtime-store.spec.ts:462-474`
   `flushDeferredTurnWork()` 用真实 820ms 宏任务等待，未等待的用例把定时器留给下一个文件/已销毁的 jsdom。

定性：**「jsdom 销毁后回调仍执行」(utils.ts:177) 为主因**，「store 定时器无环境守卫/未清理」
(runtime.ts:904, :741) 为直接爆点，「teardown 顺序 + 缺少生命周期清理」为放大条件。三者需同时收口，
否则只改 utils 会让 T2a/T2b 依旧红。

## 4. 绿相可行性（非仓库改动）

在隔离副本 `/tmp/wave55-green-sim`（src 拷贝 + node_modules 软链）施加最小三处守卫
（utils.ts:176 `if (!idleWindow) return;`；runtime.ts schedule/cancel 各加
`if (typeof window === "undefined") { this.deferredPersistTimerId = null; return; }`）后，
同一测试文件 **5 passed / EXIT=0**，证明契约不是「永远红」的过约束；仓库 `src/` 未被修改。
