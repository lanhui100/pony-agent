# Wave-16 Lead 审计与门禁收据（ask CAS 报错修复）

基线：`7e8bdb5`（红相锚定 commit：`fc15414`）· 契约：`.dev-team/contracts/wave-16-ask-cas.md`

## 1. 机械写域审计（Join Barrier 前置）

本波次声明写域与实际改动比对（`git diff --numstat`）：

| 文件 | 归属 | +/- | 越界 |
| :--- | :--- | :--- | :--- |
| `crates/pony-agent-core/src/agent/ask_control.rs` | executor-core（task-2） | +46/-5 | 否 |
| `src/stores/ask.ts` | executor-ui（task-3） | +48/-0 | 否 |
| `src/components/AskPanel.vue` | executor-ui（task-3） | +34/-6 | 否 |
| `crates/pony-agent-core/tests/ask_pending_contract.rs`、`ask_pending_adversarial.rs` | test-agent / L2-AT | 新增 | 否 |
| `tests/ask-pending-expiry.spec.ts`、`tests/ask-panel-expired.spec.ts` | test-agent | 新增 | 否 |

**结论：无越界写入。** 实施者未触碰 `dispatcher.rs` / `tool_runtime.rs` / 验收测试；
`can_consume` CAS 语义零改动。

## 2. 变动爆炸半径（机械触发）

- 本波次业务代码 union diff = **128 增 / 11 删（>100 行）** → 触发 ≥2 路 **互补** L3 对抗审查：
  - 视角 A（并发/调度/状态流转）：`.dev-team/review-wave16-a.json`
  - 视角 B（安全防御/协议一致性 + 边界等价性）：`.dev-team/review-wave16-b.json`
- 关键字黑名单（locks/thread/net/exec 等）：本波次变更未命中（仅列表谓词 + 前端 TTL 守卫）。

## 3. 并行施工隔离声明（重要）

本工作区存在**并行 wave-15（token 指标）会话**，其未提交改动落于
`crates/pony-agent-core/src/agent/runtime/tests.rs`、`src/components/HomeWorkspace.vue`、
`src/components/chat/WorkspaceComposer.vue`、`src/lib/useTimelineAutoScroll.ts`、
`src/lib/runtime/token-metrics.ts`、`tests/acceptance/status-flow-contract.spec.ts` 等，
与本波次写域**完全不相交**。本波次所有提交使用定向 `git add <paths>`，未使用 `git add .`，
未触碰上述文件，未执行任何 `git reset/stash`。

## 4. Lead 亲自执行的门禁（真实 EXIT 码，收口终态）

| 门禁 | 结果 |
| :--- | :--- |
| `cargo test --test ask_pending_contract --target-dir target-test` | **EXIT=0**，6 passed / 0 failed |
| `cargo test --test ask_pending_adversarial --target-dir target-test` | **EXIT=0**，4 passed / 0 failed |
| `cargo test --test ask_user_acceptance --target-dir target-test` | **EXIT=0**，9 passed / 0 failed |
| `cargo test --test dispatcher_matrix --target-dir target-test` | **EXIT=0**，27 passed / 0 failed |
| `cargo test --lib agent::ask_control` | **EXIT=0**，9 passed / 0 failed |
| `cargo test --lib agent::control_plane` | **EXIT=0**，65 passed / 0 failed |
| `cargo test --lib agent::graph::tests` | **EXIT=0**，23 passed / 0 failed |
| `npx vitest run tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts tests/ask-store.spec.ts tests/AskPlanPanel.spec.ts tests/workspace-composer-ask.spec.ts` | **EXIT=0**，5 files / 39 passed |
| `npm run typecheck`（vue-tsc --noEmit） | **EXIT=0**，0 error |

红相基线（commit `fc15414`）：同上 Rust 门禁 EXIT=101（6 failed）与 EXIT=101（4 failed）、
vitest EXIT=1（4 failed / 4 passed）。红→绿转化由上述同一门禁闭合。

## 5. 全量前端单测（Lead，最终 revision）

```bash
npm run test:unit
```
结果：**Test Files 1 failed | 81 passed (82)；Tests 1 failed | 865 passed | 10 skipped (876)**。

唯一失败 `tests/workspace-runtime-status-streaming.spec.ts:56`
（`statusSlot.attributes("data-visible")` 期望 `"false"` 实得 `"true"`）属**并行 wave-14/15
状态条可见性反转**的外部在飞变更（该波次 `tasks.task-11「test: 可见性断言改写 + 红相 — pending」`
未完成；`src/components/HomeStatusPanel.vue` / `HomeWorkspace.vue` / `useTimelineAutoScroll.ts`
均在本波次写域之外、且带并行会话未提交改动）。归属判定依据：本波次 diff 不含状态条相关文件
（见 §1 写域审计），失败断言与 ask 链路无因果关联。

本波次范围内门禁（5 个 ask 相关 spec）为 **EXIT=0 / 49 passed**，Rust 三目标
（`ask_pending_contract` 6 / `ask_pending_adversarial` 4 / `ask_user_acceptance` 9）**全部 EXIT=0**。

## 6. 真实浏览器链路门禁（分级降级声明）

- `npm run build`（`vue-tsc --noEmit && vite build`）→ **EXIT=0**（构建成功，含最终类型检查）。
- `npx playwright test tests/e2e/browser-preview.spec.ts` → **EXIT=1**，6 failed，全部失败原因为
  Playwright 自带浏览器二进制缺失（`Looks like Playwright was just installed or updated ... Please run
  pnpm exec playwright install`），非断言失败；本会话不下载浏览器（避免联网/审批）。
  → 标注 **`[E2E DEGRADED: PLAYWRIGHT BROWSER MISSING]`**。
- **ask 用户面旅程不可在浏览器态复现的结构性原因**：ask 面板/卡片的数据源是 Tauri IPC
  （`ask_list_pending` / `ask_answer` / `ask_cancel` / `ask_expire`），浏览器预览态下
  `isTauriAvailable()` 为 false（`src/lib/tauri.ts`），且生产 ask 卡片
  （`src/components/chat/WorkspaceComposer.vue`）携带并行会话未提交 WIP。
  → 标注 **`[E2E DEGRADED: TAURI HOST REQUIRED]`**；本次以最高可用保真度替代：
  真实 Rust dispatcher/控制平面集成测试（C1/C4，6+4 passed）+ 真实 pinia store 与真实挂载组件
  测试（49 passed，含 18 条 ask 契约用例）+ 生产构建 EXIT=0。

## 7. 冻结测试未被削弱（Lead 独立核验，机械判据）

```bash
git diff fc15414 -- tests/ask-pending-expiry.spec.ts tests/ask-panel-expired.spec.ts | grep "^-" | grep -v "^---"
```
删除行**仅**：`import { useAskStore }`（重复导入）、`// Arrange` 注释、`mockExpiredAskHost(expired)` +
`await store.refresh()`（夹具机制，改为直接注入快照）、`expiresAtMs: Date.now() + 1`（1ms 余量夹具改 60s）。
`grep -c "expect"` 计删除的 `expect` 行 = **0** → 断言零删除、零 skip/ignore，强度只增（test-agent 另新增 5 处前置/精确断言）。
