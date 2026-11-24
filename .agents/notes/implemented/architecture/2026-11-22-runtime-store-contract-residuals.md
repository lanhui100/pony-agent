# Agent Note: runtime store 残余裂缝登记与 Pinia 内建成员契约

Status: implemented

## Problem

runtime store 拆解（B271）把契约面压成三张冻结清单：state 键、getters、actions（`types.ts` 的
`FrozenStateKeys` 等 + 两个契约测试逐项对账）。但这三张清单**没有覆盖 Pinia 内建成员**，也没有覆盖
「拆解暴露但未修」的既有死路径。具体两个缺口：

1. **内建成员无裁决**：`turn-events.ts` 的监听器聚合体有 6 处 `this.$patch(...)` 是拆解前既有实现，
   由于 `$patch` 不在 `RuntimeStoreThis` 里，代码用局部 `TurnEventsThis` 别名绕过类型面；
   `$reset` / `$subscribe` 则完全没有说法。契约清单看起来完整，实际留了一个类型后门。
2. **残余裂缝无登记**：拆解过程中发现多处死路径与双实现（如 `streamFlushFrameId` 死键、
   `restoreSessionRuntimeSnapshot` 与 `applySessionSnapshot` 双恢复家），但没有一份可执行清单，
   导致后续 Wave 2 无法判断「哪些能删、哪些是既有语义不许动、哪些要先推翻 golden fixtures」。

## Decision

1. `types.ts` 新增 `RuntimeStoreBuiltins` 契约段，逐项裁决并写明允许/禁止面：
   - `$patch`：纳入契约，仅允许 turn-events 监听器聚合体的批内原子写与测试 Arrange；
   - `$reset`：明令禁止，重置一律走 `resetSessionRuntimeState()`（它带清空面/保留面契约：
     队列、工作区树不清）；
   - `$subscribe`：生产面明令禁止，观测走 action/getter。
   `turn-events.ts` 的局部别名改为 `Pick<RuntimeStoreBuiltins, "$patch">`，类型面回到单一真源。
2. 新增机械守卫 `src/stores/runtime/__tests__/runtime-builtins-contract.spec.ts`：扫描 `src/**`
   （排除 `__tests__` 与 `*.spec.ts`），断言 `. $reset(` / `. $subscribe(` 生产面零命中，
   `. $patch(` 仅白名单 `turn-events.ts`，且白名单不腐化（该文件仍实际持有调用）。
3. 新增 `.dev-team/runtime-split/contract-residuals.md` 作为残余登记表，分四类：死路径（R1–R2）、
   归属裂缝（R3–R8）、已登记不改（R9–R10）、待 B277 收口的测试缺口；每条附证据与状态。
   其中 R1/R2/R7 随本 PR 修复，R8 明确标「待修，需另立决策」。
4. 修订口径：`fact-map.md` 是拆解前历史测绘快照，不改写正文，只在顶部追加修订注记；
   `contract-surface.spec.ts` / `concurrency-lifecycle.spec.ts` / `FrozenStateKeys` 三者是键集冻结面，
   任何键增删必须同改，禁止先改夹具后改代码。

## Alternatives considered

1. **明令禁止 `$patch`（生产 + 测试全禁）**：测试侧已有 10+ 处用它造场景（harness `seedStore`），
   禁用会逼出 `Object.assign(store.$state, …)` 这类更隐蔽的绕过；且 6 处生产调用是拆解前既有实现，
   一次性重写会把「契约收口」变成「行为重构」，超出本 PR 的变更闭包。
2. **把 `$patch` 包成 store action（如 `patchRuntimeState`）**：多一层封装但不增加原子性保证，
   反而让「谁改了什么」更难追；真正的收益是裁决与机械守卫，不是再加一个 API。
3. **只写文档清单、不加扫描测试**：宪法常载命约第 2 条要求「机械可查的承诺配一条非零退出命令」，
   只写文档等于承诺不可验；扫描测试让 `$reset` / `$subscribe` 的新增使用当场变红。
4. **清单放进 ADR 正文**：清单会随 Wave 2 持续增删，ADR 应冻结决策而非承载活账本；
   拆成 `.dev-team/runtime-split/contract-residuals.md` + ADR 引用，各自一个家。

## Consequences

- Pinia 内建成员从「隐性后门」变成「显式契约 + 可机器验收的禁止面」；
  新增 `$reset` / `$subscribe` 使用会在 `npm test` 阶段失败，不依赖 review 记忆。
- 残余裂缝有了可执行清单，B277 的等价测试可以先按 R8/R10 的边界设计，不必重新测绘。
- 静态扫描测试白名单是显式维护点：新增 `$patch` 合法用途时须同步改 `types.ts` 契约段、
  测试白名单与该文件，三处不一致即红。
- 成本：源码扫描测试读取整个 `src/**`，本机约 30ms，可接受；`.vue` 文件也在扫描面内。
