# Agent Note: goal 元工具权威结构化（TurnKind）与双预算语义显式化

Status: implemented

## Problem

`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` 的审核在 O1–O3/O5/O6 之外还留了两项
P1/P2 缺陷（PR #106 明确把它们排除在范围外并登记 backlog）：

**P1-1（O4）：goal 元工具的「谁有权」靠文案嗅探推导。**

`DispatchContext` 只有两个布尔：`is_goal_round: bool` 与 `is_direct_human: bool`，
由 `AgentRuntime::resolve_governed_turn_context` 用
`is_goal_round_prompt(&input.message)`（匹配 `<goal_round round="N" max="M">…</goal_round>`
模板）**推导**，`is_direct_human = !is_goal_round`。

问题出在「既不是模板也不是人类直发」的那一类轮次：**Ask / 审批恢复轮**。恢复轮的输入是
用户原文（或注入的答案回填），不是 `<goal_round ...>` 模板 ⇒ 被判成
`is_direct_human = true` ⇒ 模型在恢复轮里拿到 `create_goal` / `pause` / `edit` / `resume`
的权威（`crates/pony-tools/src/tools/misc.rs` 的四处 `goal_action_unauthorized` 闸门）。
这与「Ghost* 只应由人类直发轮触发」的设计意图直接冲突：一次 429 重试、一次 Ask 作答，
都可能让模型在没有人类明示的情况下创建/暂停/改写目标。

**P2-1（O7）：goal 有两个轮次预算，但界面只讲一个。**

- 每输入预算：`MAX_CONSECUTIVE_AUTO_CONTINUES = 8`（`planner.rs`），一次用户输入触发的
  自动续轮链上限；
- 目标预算：`GoalData::max_goal_rounds`（默认 50），跨多次输入累计的成功轮次上限。

`/goal show` 与桌面 `GoalBar` 都只展示后者（「轮次: N/50」）。于是用户看到「承诺 50 轮、
实跑 8 轮就停」，而界面上没有任何一句话解释这 8 轮的来源——观感是承诺没兑现。

## Decision

### O4：用 `TurnKind` 取代两个推导布尔

`crates/pony-tools/src/dispatcher.rs` 新增：

```rust
pub enum TurnKind { DirectHuman, GoalRound, AskResume, ApprovalResume, SubAgent }
```

`DispatchContext` 的 `is_goal_round` / `is_direct_human` 两个字段**删除**，替换为
`turn_kind: TurnKind`（`#[serde(default)]` ⇒ 缺省 `DirectHuman`，与旧
`is_direct_human: true` 默认值等价）。授权真值表收敛为两条方法：

- `TurnKind::is_direct_human()` ⇒ 仅 `DirectHuman`（ghost 类动作的唯一授权来源）；
- `TurnKind::is_goal_round()` ⇒ 仅 `GoalRound`（`complete` / `blocked` 的第二条路径）。

类别由**构造轮次的宿主**写入，不靠文案：

- `RunTurnFacts` 新增 `turn_kind: Option<TurnKind>`（`crates/pony-agent-core/src/agent/runtime/types.rs`）；
- `resolve_governed_turn_context` 的推导改为 `facts.turn_kind.unwrap_or_else(嗅探兜底)`
  —— 声明优先，嗅探只在「宿主没声明」时保持既有行为（向后兼容）；
- 各宿主显式声明：Ask 恢复轮 ⇒ `AskResume`，审批恢复轮 ⇒ `ApprovalResume`
  （`ask_resume_commands.rs` 的 `continue_bound_graph_run_stream` 加 `turn_kind` 参数，
  两个入口分别传入自己的类别）；goal 自动续轮 ⇒ `GoalRound`
  （`execute_graph_run_stream` 的续轮分支与 `advance_graph_run` 的循环）；
  子代理派发 ⇒ `SubAgent`（`child_dispatch.rs`）。

`PreparedGraphRunStream` 增加 `turn_kind: Option<TurnKind>` 作为该载荷，使「发起方声明」
能穿过 `prepare_* → execute_graph_run_stream_step` 的既有管线，而无需另造执行路径。

### O7：显式声明路线（不合并闸门）

两个预算回答的是两个不同问题——「一次输入能自治多久」vs「这个目标总共值多少轮」——
合并会让其中一个语义消失（合并成 50 则失去单次自治上限，合并成 8 则目标预算名存实亡）。
因此选择「显式声明 + 分别展示」：

- 数值单源：`pony_storage::goal::MAX_AUTO_CONTINUES_PER_INPUT = 8`；
  `planner::MAX_CONSECUTIVE_AUTO_CONTINUES` 改为引用它（core 依赖 storage 方向合法），
  禁止两处各写字面量；
- 文案单源：`goal::budget_summary_lines(&GoalData) -> (String, String)` 产出
  「自动续轮: 每次输入最多 8 轮（之后等你发话，目标保持激活）」与
  「目标预算: N/M 轮（跨多次输入累计；用满后目标转 blocked(round_limit)）」；
- 展示面：TUI `/goal show` 直接追加这两行（保留既有「轮次: N/M」行）；桌面端新增
  additive 命令 `goal_budget_lines`（`src-tauri/src/lib.rs`），`GoalBar` 取回后**逐字**渲染，
  前端不拼措辞。

不改 `goal://changed` 载荷：它的字段集合是 §7 冻结契约（`goal_events.rs` 的形状测试逐键断言），
而双预算文案是纯展示、与状态变更事件无关（目标没变也要能读）。

## Alternatives considered

**O4 备选：保留两个布尔，只补一个 `is_resume: bool`。** 落选：三个布尔能表达 8 种组合，
其中 5 种是非法的（`is_goal_round && is_resume`、三者皆 false…）。非法状态可构造就是缺陷温床
——审核报告 P1-1 正是「两个布尔的组合语义靠调用方自觉」的产物。枚举让非法状态不可表达。

**O4 备选：只改文案嗅探，让它也能识别恢复轮。** 落选：恢复轮的输入就是用户原文，
与「人类直发轮」在文本上**不可区分**——这是信息缺失，不是匹配规则不够聪明。任何
「更聪明的嗅探」都只能靠猜，且会把猜错的责任永久留在运行时。

**O4 备选：把 `turn_kind` 直接塞进 `TurnInput`（线缆字段）。** 落选：`TurnInput` 是前端可写的
入参，让前端能声明「我是 GoalRound」等于把授权判定交给不受信方。类别必须由**宿主在服务端侧**
写入，故落在 `RunTurnFacts`（进程内事实）而非 `TurnInput`。

**O7 备选：把 8 轮并入 `max_goal_rounds` 单闸门。** 落选：单闸门无法同时表达「单次自治上限」
与「目标总预算」。合并后要么把目标预算压到 8（长目标永远做不完），要么把单次自治放到 50
（一次输入失控跑 50 轮、期间人类插不上话）。

**O7 备选：把双预算文案塞进 `goal://changed` 载荷。** 落选：该载荷是 §7 冻结契约，
逐键断言；而文案是展示层事实，与状态变更无关（目标没变也要能读）。单开只读命令
`goal_budget_lines` 让「状态事件」与「展示文案」两个变化轴解耦。

## Consequences

- **契约面变更**：`DispatchContext` 删两字段加一枚举 —— 它是进程内结构（无 TS 声明、
  无 `from_value::<DispatchContext>` 反序列化点，已检索确认），故不影响线缆契约；
  `serde` 缺省 `DirectHuman` 保证旧序列化载荷仍可读。
- **授权收紧**：Ask/审批恢复轮、子代理派发现在都**失去** ghost 类 goal 元工具权威。
  这是修缺陷的意图本身；`complete` / `blocked` 在 `GoalRound` 下仍放行（续轮的正当动作）。
- **新增只读命令** `goal_budget_lines`（Tauri），纯展示、无副作用。
- **可见性**：`/goal show` 与 GoalBar 都能读到「8 轮 / 50 轮」两个口径，
  「承诺 50 实跑 8」的观感落差由界面自身解释掉。
- **残留**：文案嗅探作为「宿主未声明」的兜底仍然存在（`runtime/mod.rs`）。它现在只服务
  向后兼容路径；随着所有宿主补齐声明，可再立一条清理项。
- **验收**：`crates/pony-agent-core/tests/runtime_suite/goal_o4_turn_kind_authority_contract.rs`
  （8 例：变体集合、授权矩阵、恢复轮拒绝 create/pause/resume/edit、DirectHuman 放行、
  GoalRound 放行 complete 但拒绝 pause）与 `goal_o7_dual_budget_contract.rs`
  （7 例：常量冻结、单源一致、两预算不等、文案含双预算与进度、`/goal show` 双行共存）、
  前端 `tests/GoalBar-budget.spec.ts`（7 例：请求/逐字渲染/旧行共存/空载荷/畸形载荷/失败降级/
  会话切换刷新）。
