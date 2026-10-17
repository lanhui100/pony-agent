# Wave-24 Lead 裁定书：历史会话加载时序缺陷的根因与契约边界

## 1. 触发事件（物理凭据）
Lead 全量回归门禁（`npx vitest run`）结果：`Test Files 1 failed | 89 passed`，`Tests 2 failed | 960 passed | 10 skipped`。
失败用例（`tests/runtime-store.spec.ts`，Exit Code 1，日志见 `.dev-team/evidence/wave24-full-regression-failures.log`）：
1. `hydrates assistant history from snapshot content instead of stale persisted markdown text`（:1662）
2. `hydrates reasoning, model label, and tool history from trace when persisted cache is unavailable`（:1749）

两者均断言 `store.messages` 数组顺序为 `[user, assistant, tool]` 并按位置索引断言（`messages[1]`=assistant、`messages[2]`=tool）。
执行者此前仅跑定向子集（HomeWorkspace / multihop）即申报全绿，属**假放行**，由 Lead 全量门禁拦下。

## 2. 事实核对（Lead 只读侦察）
- `src/stores/runtime.ts:2960-2975`：assistant 消息在首个 model 事件时 `push` 入队。
- `src/stores/runtime.ts:2978-3064`：tool 消息在 `syncToolMessages` 中 append 到数组尾部。
  ⇒ **实时回合的数组顺序本就是 `[user, assistant, tool...]`**。
- `src/lib/runtime/messages.ts:333-357`（`buildTurnHistory`）：仅保留 user/assistant（过滤 tool），故 LLM 侧历史顺序不受 tool/assistant 相对位置影响。
  ⇒ 数组顺序不承载模型上下文语义，仅承载展示侧的 fallback 排序信号。

## 3. 裁定
- **不改动 `store.messages` 数组顺序**（`src/lib/runtime/messages.ts` 的 push 顺序回退）。理由：① 与实时装配路径保持一致，避免「实时 vs 重载后」数组顺序不一致导致 `latestMessageRole`/`latestTurnSignature` 等派生信号翻转；② 不破坏既有 store 契约与 2 条回归用例；③ 最小闭包原则。
- **缺陷根因位于展示层** `src/components/HomeWorkspace.vue::agentTurnEvents` 的 fallback 排序权重。当前实现仍有两处用户可见缺陷：
  - (i) `modelEntries.length === 0`（快照式纯历史）：content 取 `assistantOrder`，tool 取 `messageIndexInTurn(tool)`（更大）⇒ **Content 渲染在 Tools 之前**；
  - (ii) `modelEntries.length > 0` 但工具无匹配 `call_tool` 条目（timeline 仅含 `call_model`）：`toolOrder = modelEntries[0].sequence + 0.1` ⇒ **Tool 落到 Content 之下**。
- 正确不变量：**Reasoning < Tools < 最终 Assistant 正文**；且**具备 `call_tool` trace 链接的工具必须保持多 hop 交错顺序不变**（防止修 fallback 时破坏既有正确行为）。

## 4. 契约演进（契约演进门禁）
- 原验收用例 F1 的数组顺序断言与既有 store 契约冲突，由 Lead 一票裁定后交 Test Agent 修订（task-4）：F1 改为锁定 live-consistent 数组顺序；新增 F3（timeline 仅含 call_model 时的展示顺序，当前必红）、F4（多 hop 交错回归锁）。
- 实施分离：task-5 由 Executor 执行；验收测试只读，Executor 不得修改 `tests/acceptance/`。

## 5. 熔断计数
- 波次 wave-24 / 功能流（L2-T）失败计数：attempts = 1（上限 3）。定向子集假放行属流程缺陷，已通过「全量套件为唯一放行门禁」纠正。

## 6. 裁定二（展示顺序不变量与 tests/HomeWorkspace.spec.ts:3138 契约冲突）

### 6.1 事实
实施者 executor-agent-25 报告规格偏差：若把「无 trace 链接工具」无条件排在正文之前，冻结用例 `tests/HomeWorkspace.spec.ts:3138`（"orders assistant content and tool calls by their turn timeline instead of pinning tools above content"）必红。
该用例夹具：`messages=[user, assistant("first answer", done, 无 reasoningContent), tool("read after answer")]`、无 `traceTimeline`，断言 `toolIndex > contentIndex`（正文在工具之上）。
实施者因此把规则收敛为「仅当该回合实际渲染出 reasoning 面板时才上抬工具」，其余场景保留数组时间线顺序。实测：无条件版本 = `1 failed | 963 passed`（唯一失败即 :3138）。

### 6.2 证据链
- `git log -S "instead of pinning tools above content"` → 该断言由 `80bf840 fix: stabilize agent streaming rendering` 引入，其 design.md 只讨论流式 markdown 呈现，**未对「思考/工具/正文」的排序语义作出任何产品决策**；该断言是当时实现的伴生锁，而非产品契约。
- `docs/decisions/0008-event-sourcing-evolution.md:18`：turn 内工具调用时序只存在于流事件、**落盘后丢失**。⇒ 重载会话不存在权威时序来源，必须使用**规范化顺序**，不能依赖数组下标（数组下标在实时装配中同样是「assistant 先入队、tool 后追加」，并非时间线）。
- 用户指令（本波次需求来源）：每一轮的 Agent 正文必须位于思考过程与工具调用之后。
- 数组下标排序在「无 reasoning 面板」配置下会让正文重新跑到工具之上，即：同一逻辑回合的展示顺序取决于「思考过程开关」这一无关配置 ⇒ 配置依赖型排序，不可接受。

### 6.3 裁定
- 采用**规范化不变量**：`Reasoning < Tools < 正文`，对同一回合**无条件成立**（不依赖 reasoning 面板是否渲染）。
- 契约演进：`tests/HomeWorkspace.spec.ts:3138` 的期望由「正文在工具之上」修订为「工具在正文之上」，标题同步改为反映规范化顺序，并在注释中引用本裁定；该修订由独立 Test Agent 执行（task-6），实施者无测试写权。
- 新增验收用例 F5：无 reasoningContent、无 trace、含工具的回合，DOM 必须为 `Tools -> Content`（无条件规则的判别用例，当前必红）。
- 实施者（task-7）移除 `fallbackReasoningShown` 条件，令 fallback 工具权重无条件为 `assistantOrder - 0.1 + toolIdx*0.001`（无 modelEntries 时）/ `lastModelSequence - 0.1 + toolIdx*0.001`（有 modelEntries 时）；`traceSequence != null` 保持原样。

### 6.4 熔断计数（更新）
- 波次 wave-24 / 功能流（L2-T）attempts = 2（上限 3）：第 1 次为定向子集假放行；第 2 次为规格未覆盖「无 reasoning 面板」场景导致的契约冲突（由实施者主动报备，属诚实上报，非缺陷隐瞒）。
