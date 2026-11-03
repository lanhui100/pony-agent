# Wave-61 契约：Agent Team 激活（工具面 × 宿主接线 × 策略提示 × 并发守恒）

- 契约编号：wave-61-team-activation
- 状态：`FROZEN v1.7（wave-61；v1.7 = 真机举证 BLOCKING 收口：C12 子代理工作线程运行时上下文 / recheck 链 fa·fp·fr·fv 登记 / e2e_probe 客观事实入账）`
- 作者：Protocol Agent（协议特派员）；v1.1/v1.2 由 Lead 裁决驱动演进
- 唯一写域：`.dev-team/`（本文件 + `.dev-team/nfr-baseline.json` + `.dev-team/nfr-baseline.schema.json` 的**增量**声明；schema 只含 wave-61 自己的字段）
- 禁止写域：任何 Rust 源码与测试（含 `crates/**`、`src-tauri/**`、`tests/**`）、`.dev-team/state.json`
- 分级：L3（协议契约 + 跨模块集成 + 宿主 bootstrap + 并发原语）

> **引用口径**：本契约对代码采用「符号名 + `path:line`」双轨引用。行号已于 v1.2 按 **HEAD `60e7cb8` + 工作树**重新实测；
> 实现位移后以**符号名**为准，行号仅作定位辅助。

## 0. 缺陷因果（Lead 已用 trace 证据确认，本契约复述为设计前提）

`build_context_observations` 中 session-4bdcc39a / turn-0c9aebe0：dev-team 技能正文已注入，但该 turn 的
`toolDefinitionsText` 只有 28 项且**完全不含** subagent / workflow / spawn_teammate / list_agents /
send_message / interrupt_agent / drain_inbox / team_task_*。更早的 turn-62edc32c 工具面里有这些工具。
→ 结论：不是模型"忘了用"，而是**工具面被收窄**，且即便可见也会 fail-closed。

三条独立真值源（逐条给出 `path:line`）：

| 编号 | 真值源 | 现状 | 因果 |
|---|---|---|---|
| S1 可见性 | `crates/pony-agent-core/src/agent/tools.rs:7413` `tool_exposure_for_name()` → `:7470` `_ => ToolExposure::Internal` | 10 项团队工具全部落 `Internal`（v1.2 目标：其中 **9 项**升 `ModelVisible`，`workflow` 按 C1.7 **保持** `Internal`） | `provider_contract_views()`（`tools.rs:622-631`）只保留 `ModelVisible` 或 `primitive_name == tool_search`；`TurnToolView::from_registry`（`tool_runtime.rs:161-176`）同一过滤；Dispatcher 对 `Internal` 直接拒（`dispatcher.rs:851-866` `origin_not_authorized`）→ 既不可见、也不可能被模型调用 |
| S2 可执行性 | `crates/pony-agent-core/src/agent/tools.rs:152` `subagent_runtime_wired()` = `subagent_engine().is_some()` | 注册点在 wave-61 已收敛进 `runtime/builder.rs`（C2）；此前生产从未调用 | 未接线时 `tools.rs` 10 处 fail-closed 门（`:1571-1573` 等）返回 `capability_unavailable` |
| S3 策略面 | `crates/pony-agent-core/src/agent/context.rs:15` `BASE_SYSTEM_PROMPT` | wave-61 已新增 `AGENT_TEAM_POLICY_PROMPT`（`:63-78`）并按 `agent_team_policy_enabled()`（`:76-78`）注入（`:451-453` 稳定层、`:547-552` native 面） | 参考实现 DSH `packages/experimental/tool-agent-team/src/index.ts:34-58` 注入 `Agent Teams is available in this session` 策略段（`:173-177` 注册） |

> 28 vs 29 的口径说明：wave-61 前 `builtin_turn_tool_contract_views()` 为 **29** 项（W42/PA-149 增 `skill_load`）。
> trace 观测到的 28 项与此差异归因于该 turn 早于 `skill_load` 落地。**[UNVERIFIED]**：未回放该 trace 的构建指纹。

---

## C1 工具面暴露契约（9 项 `ModelVisible`；`workflow` 保持 `Internal`）

> **v1.2 变更（Lead 裁决 / L3-P BLOCKING #2）**：`workflow` 从可见面**排除**（见 C1.7），
> 模型可见面由 wave-61 初稿的 39 项修正为 **38 项**。

### C1.1 目标状态

`tool_exposure_for_name()`（`crates/pony-agent-core/src/agent/tools.rs:7413`；团队工具复合臂现位于 `:7449-7460`）
必须为下列 **9 个** primitive 返回 `ToolExposure::ModelVisible`：

`subagent`、`spawn_teammate`、`list_agents`、`send_message`、`interrupt_agent`、`drain_inbox`、
`team_task_create`、`team_task_update`、`team_task_list`（常量定义：`tools.rs:156-165`）。

`workflow` **必须**落回 `_ => ToolExposure::Internal`（`:7470`）——即 `:7449-7460` 的 `ModelVisible` 复合臂中
**不得**再包含 `TOOL_WORKFLOW`（C1.7）。

product 名（对外模型名）对 9 项＝ primitive 名本身：`product_visible_tool_name()`（`tools.rs:7337`）在
`model_visible_tool_name_opt()`（`:7274-7313`）命中时返回登记值，未命中则原样透传。C1.6 要求显式登记。
**不得**新增 CamelCase 别名（别名表 `builtin_aliases_for_primitive()` 区域保持 `vec![同名]`）。

### C1.2 38 项精确清单（模型可见面，含顺序）

顺序真值源＝ registry 排序键 `(!visible, product_first_index)`（`tools.rs:555-566`）＋ `builtin_tools()` 定义序
（`tools.rs:5392` 起；10 项团队工具注册于 `:5925`/`:5952`/`:5983`/`:6014`/`:6023`/`:6047`/`:6066`/`:6081`/`:6110`/`:6138`，
夹在 `view_image`（`:5894`）与 `todo_write`（`:6147`）之间）。

`workflow`（定义槽位 22）为 `Internal` → **不占可见面槽位**。因此 38 项 = 既有 29 项保持相对顺序，
**9 项**团队工具插在 `ViewImage` 之后、`TodoWrite` 之前（可见面数组 index 17..25）：

```
 1. Run
 2. Ask
 3. Read
 4. ReadDocument
 5. List
 6. Search
 7. Glob
 8. WebFetch
 9. WebSearch
10. MCPResource
11. ToolSearch
12. Write
13. Edit
14. BatchExecute
15. Plan
16. ViewImage
17. subagent
18. spawn_teammate
19. list_agents
20. send_message
21. interrupt_agent
22. drain_inbox
23. team_task_create
24. team_task_update
25. team_task_list
26. TodoWrite
27. TodoList
28. create_goal
29. get_goal
30. update_goal
31. Present
32. Find
33. GitDiff
34. JobStart
35. JobOutput
36. JobKill
37. JobList
38. skill_load
```

推导证据（逐项可复核，非猜测）：
- 既有 29 项顺序与 `tests/wave37_tool_surface.rs` 的冻结清单、`tests/wave25_capability_honesty_test.rs` 的
  `EXPECTED_MODEL_VISIBLE_TOOLS` 逐字一致（wave-61 迁移后应为 38）。
- `Run` 占 1 号位：`time_now` 与 `run_command` 共用 product 名 `Run`（`tools.rs:7276-7277`），
  `contract_priority` 让 `run_command`（100，`:6480`）胜过 `time_now`（10，`:6465`），槽位取 product 名
  **首次出现**下标（`:493-495`、`:555-566`）。
- 9 项团队工具 product 名唯一，`contract_priority` 取默认 50（`:6498` `_ => 50`），无 winner 争用；
  其可见槽位＝定义下标（21、23..30），落在 `view_image`(20) 与 `todo_write`(31) 之间。

### C1.3 变更类型标注与兼容性证据

| 面 | 类型 | 依据 |
|---|---|---|
| 模型/协议面（provider tools payload） | **additive（9 项）** | 既有 29 项的 name / description / input_schema / kind / permission facts 不变：`ToolDefinition::contract_view()` 与 `from_builtin_definitions`（`:475-566`）**只**由 `exposure` 影响 `provider_contract_views()` 的过滤结果；descriptor 集合、`snapshot_id`（`builtin-tool-catalog-v1`）、别名表不变。 |
| `workflow` | **无变更**（v1.2 回到 `Internal`） | 不新增、不删除、不改 handler；`builtin_tools()` 内部注册数仍为 45。 |
| Dispatcher 授权面 | **additive（放宽 9 项）** | `authorize_origin`（`dispatcher.rs:851-904`）仅按 `exposure == Internal` 与 turn view 允许集判定；无 schema/权限语义变更（9 项权限 facts 均 `permission_scope: None`、`requires_approval: false`，本波**不得**改动）。 |
| 执行面（handler） | **无变更** | 9 个 handler 已存在并已接线（`tools.rs:1535-1544` 派发）；`workflow` handler 保留但不可由模型触达。 |
| provider 工具数组前缀 | **breaking（一次性）** | 9 项插入 index 17..25，使原 index ≥17 的 13 项后移 9 位（`Run…ViewImage` 不变）。同一构建内数组逐 turn 恒定 → 只在升级后第一个 turn 发生一次前缀缓存 miss（ADR 0007 关注 **per-turn** 抖振）。 |
| 冻结测试契约面 | **breaking** | 6 个面必须演进，见 C4。 |

**为何不把 9 个定义挪到 `builtin_tools()` 末尾**：需要移动多段注册代码、打乱"定义序即产品面序"的既有惯例
（W37-C1/W42 两次迁移都依赖该惯例），为省一次 miss 引入结构性移动——判**不接受**。

### C1.4 为何不是 `Deferred`（对比 `tool_search` 提升路径）

`Deferred` 在本仓库是一条**受治理的发现→提升通道**，边界是硬编码的：

1. 搜索面只索引 `Deferred`：`ToolSearchElevator::search()` 过滤 `exposure == Deferred`
   （`tool_search_elevation.rs:81-83`），`elevate()` 再校验（`:126-129`），`TurnToolView::elevate_from_registry`
   亦要求 `Deferred`（`tool_runtime.rs:198-200`）。→ 保持 `Internal` 则**永不出现**在搜索结果；`Deferred` 才可被发现。
2. 提升是 **per-turn** 的：`elevated_descriptor_ids` 属于单次 `TurnToolView`（`tool_runtime.rs:152-185`），不跨 turn
   → 采用 `Deferred` 意味着**每个 turn 都要多一次「tool_search 发现 + elevate」往返**，而团队协作是会话级常驻能力。
3. `tool_search` 的 Deferred 是"默认延迟发现的**单一条目**"（`tools.rs:622-631` 注释与白名单例外）；把 9 项并入会让
   该通道承载 11 项，且与「模型**第一 turn** 就能派生队友」的缺陷目标直接冲突。
4. 参考实现对齐：DSH 在 Agent scope **无条件**注册整套团队工具与策略段（`tool-agent-team/src/index.ts:168-183`）。
5. `Deferred` 不能替代 fail-closed：wave25 T2 已证明"看不到 ≠ 不可用/不可调用"（`tests/wave25_capability_honesty_test.rs:197-210`）。

结论：**9 项 `ModelVisible`，且可见性不随 `subagent_runtime_wired()` 变化**（保证 provider 前缀稳定）；
「可用/可执行」由 `subagent_runtime_wired()` 门控（C2 fail-closed、C3 策略段）。
`workflow` 不在本论证范围——它被排除的**理由不是**曝光机制，而是**实现是桩**（C1.7）。

### C1.5 验收断言（可机器判定）

- **C1-A1**：`builtin_turn_tool_contract_views()`（`tools.rs:712`）返回项数 `== 38`，且 `Vec<name>` 与 C1.2 的 38 行清单**逐字、逐序相等**。
- **C1-A2**：对 9 个 primitive 名逐一断言 `tool_exposure_for_name(name) == ToolExposure::ModelVisible`。
- **C1-A3**：`builtin_tool_surface().provider_contract_views()`（`tools.rs:699`）与 `builtin_turn_tool_contract_views()` 名字集合完全一致。
- **C1-A4**：既有 29 项中每一项的 `kind` / `exposure` / `permission_facts` 与迁移前一致。
- **C1-A5**（可执行性）：`subagent_runtime_wired() == false` 的进程内经 ToolRouter 调 `subagent` → `error.code == "capability_unavailable"`；
  wired 进程内经 Dispatcher 以 `InvocationOrigin::Model` 调 `subagent` → **不得**出现 `origin_not_authorized`。
- **C1-A6**：`model_visible_tool_name("<团队工具>") == "<团队工具>"`（9 项逐一，**不得**回落 `"Run"`）；见 C1.6。
- **C1-A7**：`product_visible_tool_name` / `product_canonical_tool_name` 对 9 项返回原名，且 `model_visible_tool_name_opt` 的既有映射（含 `time_now => "Run"`）**不得**改动。

### C1.6 产品名登记条款（trace 证据不污染）

`model_visible_tool_name_opt()`（`tools.rs:7274-7313`）必须为 **9 个模型可见团队工具**登记「小写原名透传」
（与 `TOOL_SKILL_LOAD => TOOL_SKILL_LOAD`（`:7312`）**同构**）。

**因果**：`model_visible_tool_name()`（`:7329-7335`）对未登记名回落字面量 `"Run"`（`unwrap_or("Run")`），而
`crates/pony-agent-core/src/agent/telemetry.rs` 的 tool activity 记录点（`:306`、`:332`、`:508`、`:576`）正用
`model_visible_tool_name(&<tool>.name)` 生成 trace 的 tool `name`；`product_canonical_tool_name`（`tools.rs:7343-7345`）同源。
未登记 → 团队工具在 trace 里被记成 `Run`，**污染本波赖以定因的取证面**。

`workflow` 因 C1.7 不在模型面，**不纳入**本条款门禁；其产品名登记保留与否不影响任何断言（不得据此改动 exposure）。

### C1.7 桩能力不得进模型面（`workflow` → `Internal`）

**真值源**：`crates/pony-agent-core/src/agent/orchestration.rs:286-297` —— `workflow()` 只把 `args.steps` 的
`step.id` 逐个回显进 `executed_steps`，**固定返回** `status: "success"`，不执行任何 DAG、不产生步骤状态、不可观测。
配合 `tools.rs:1601` 的 fail-closed 门可知：它可以"成功"回执但没有任何真实副作用。

→ 若它出现在模型面，即 W25「假能力」回归（`tests/wave25_capability_honesty_test.rs:124-180` 的成立前提被破坏）：
模型会拿到 `status: "success"` 的**零执行**结果并据此编排后续步骤。

**恢复条件**：真实 DAG 引擎落地（`workflow()` 真实调度 + 步骤级状态/错误可观测）后，经 successor 契约恢复曝光。
在此之前 `workflow` 必须 `Internal`（隐藏但**不删除** handler，保留给未来引擎接管）。

- **C1-A8**：`tool_exposure_for_name("workflow") == ToolExposure::Internal`，且 `builtin_turn_tool_contract_views()`
  名字集合**不含** `workflow`，总数 `== 38`。
- **C1-A9**：`builtin_tools()` 仍含 `workflow`（内部注册数 45 不变）——「隐藏不等于删除」；
  `builtin_registry_cached().resolve("workflow").exposure == Internal`。

---

## C2 宿主引擎接线契约（注册点收敛到 core）

### C2.1 唯一注册点（由证据支持的 `path:line`）

**Lead 裁决**：注册点收敛到 core，**禁止**改 `src-tauri/src/lib.rs`（该文件属并发会话 write scope）。

证据链（三条宿主全部经由同一个 core 漏斗 `AgentRuntimeBuilder::build()`）：

| 宿主 | bootstrap 证据 | 到漏斗的路径 |
|---|---|---|
| tauri（桌面） | `src-tauri/src/lib.rs:1339` `.manage(HostControlPlane::new())` | `HostControlPlane::new()`（`control_plane/mod.rs:1315-1317`）→ `DesktopHostPreset::build()`（`:1311-1313`）→ `HostControlPlaneBuilder::build()` → `mod.rs:1042` `AgentRuntimeBuilder::new().plan_store(...).build()` → **`runtime/builder.rs:79`** |
| non-tauri harness | `crates/pony-agent-core/src/bin/non_tauri_harness.rs:93-103` | `HostControlPlaneBuilder::runtime_builder(...)` → `control_plane/mod.rs:994-995` `runtime_builder.build()` → **`runtime/builder.rs:79`** |
| TUI | `crates/pony-agent-core/src/bin/tui.rs:2723-2731` | 同上（`mod.rs:994-995`）→ **`runtime/builder.rs:79`** |
| CLI（附带获益） | `crates/pony-agent-core/src/bin/cli.rs:394` | 直接 `AgentRuntimeBuilder::…build()` → **`runtime/builder.rs:79`** |

→ **唯一注册点＝ `crates/pony-agent-core/src/agent/runtime/builder.rs:79` `AgentRuntimeBuilder::build(self) -> AgentRuntime`**
（等价入口 `AgentRuntime::build()` → `builder.rs:131-133` 走同一函数）。

### C2.2 注册语义（硬约束，已实现形态）

1. **注册内容**：`Arc<dyn SubagentTurnEngine>` = `HostTurnEngine`（`subagent_runtime.rs:103-147`），
   经 `with_workspace_root(<builder 的 workspace_root>)`（`:122-126`）携带父会话工作区根，满足 wave-58 F1-5
   （`tests/wave58_context_skills.rs:567-576`）。取值必须在 `builder.rs:117-119` 把 `workspace_root` move 进字符串
   **之前** clone（现实现于 `:87` 传参）。
2. **幂等（OnceLock）**：槽为 `static SUBAGENT_ENGINE: OnceLock<...>`（`subagent_runtime.rs:37`）；
   `set_subagent_engine` 重复注册返回 `Err("subagent engine already set")`（`:40-45`）。**首写者胜，后续静默 no-op**。
   → 注册点必须是 `ensure_host_subagent_engine()`（`builder.rs:138-156`）：先 `subagent_engine().is_some()` 早退，
   再 `let _ = set_subagent_engine(...)`（`:153`）；**不得** `.expect()` / `unwrap()`。
3. **不得阻断宿主启动**：`build()` 返回 `AgentRuntime`（非 `Result`）→ 注册失败（含已注册、含 `HostTurnEngine`
   自建栈的嵌套 build）一律忽略，不 panic、不改返回值、不阻塞。
4. **公共 API 不变**：`set_subagent_engine` / `subagent_engine` 签名与 `Err` 语义不变
   （`tests/wave37_tool_surface.rs:279-287`、`tests/pa110_teams_acceptance.rs:28` 依赖）。
5. **不得在宿主侧新增注册调用**：`src-tauri/**`、`crates/pony-agent-core/src/bin/**` 中 `set_subagent_engine` 出现次数必须为 **0**。

### C2.3 「未接线时 fail-closed」保持不变（约束，不得放宽）

10 处门保持不变（`tools.rs:1571-1573`、`:1600-1602`、`:1629-1631`、`:1658-1660`、`:1678-1680`、`:1707-1709`、
`:1736-1738`、`:1765-1767`、`:1794-1796`、`:1823-1825`）：`!subagent_runtime_wired()` →
`capability_unavailable_result(...)`（`:8723-8726`）。`orchestration.rs:244-252` 的结构化错误文案亦不变。
C2 只增加"生产路径上确实会接线"，**不删除**任何 fail-closed 分支。
（注：`workflow` 的 fail-closed 门保留，即便它已不在模型面——宿主/内部调用路径仍受保护。）

### C2.4 已知限制（显式登记，不隐藏）

- `workspace_root` 由**首次** `build()` 决定并固化进全局槽；同进程后续不同根的 `build()` 不覆盖。
  桌面单进程单宿主可接受；多根并发（测试、子运行时）下子代理沿用首个根。**[UNVERIFIED]**：tauri + harness 同进程并存的影响。
- `HostTurnEngine::run_turn` 内部会自建 `AgentRuntimeBuilder::new()…build()`（`subagent_runtime.rs:167-170`），
  嵌套 `build()` 再次调用注册 → 必为 no-op（同上第 2 条），不得因此报错或改变并发门。

### C2.5 验收断言（可机器判定）

- **C2-A1**：`builder.rs:79-121` 的 `build()` 首步调用 `ensure_host_subagent_engine(self.workspace_root.clone())`（`:87`）；
  `ensure_host_subagent_engine`（`:138-156`）是**唯一** `set_subagent_engine` 调用点（`:153`），且具「槽非空早退 + `let _ =` 忽略 Err」形态。
- **C2-A2**：`grep -c "set_subagent_engine" src-tauri/src crates/pony-agent-core/src/bin` == **0**（宿主零改动）。
- **C2-A3**：连续两次 `AgentRuntimeBuilder::new().build()` 不 panic；第二次注册返回 `Err` 且被忽略。
- **C2-A4**：构建过 runtime 的进程内 `subagent_runtime_wired() == true`；从未构建者 `false` 且 `subagent()` 返回
  `capability_unavailable`（沿用 `tests/wave25_capability_honesty_test.rs:210`、`tests/orchestration_acceptance_test.rs:126-156`）。
- **C2-A5**：注册的引擎携带父根：`ensure_host_subagent_engine` 以 builder 的 `workspace_root` 构造 `HostTurnEngine::with_workspace_root`。

---

## C3 团队策略提示契约（仅当已接线时注入）

### C3.1 门控

注入条件**唯一**：`subagent_runtime_wired()`（`tools.rs:152`），经 `agent_team_policy_enabled()`（`context.rs:76-78`）暴露。
为 `false` 时**完全不注入**（不得注入空段/占位段/"不可用"提示），未接线宿主提示面逐字节不变。

### C3.2 文本静态性（DSH 对齐的硬约束）

- 策略段必须是 `&'static str` **常量全文**（`context.rs:63-78` `AGENT_TEAM_POLICY_PROMPT`），**禁止**运行期插值
  （成员名、角色、路径、工具数量、可用性开关等一律不进文本）。
- 证据：DSH `teamPolicy()` 是纯静态模板（`packages/experimental/tool-agent-team/src/index.ts:34-58`），以
  `systemPrompt.section({ name: 'team:policy', order: getSectionOrder('TEAM_POLICY'), text: teamPolicy(...) })`
  注册进 **System 稳定段**（`:173-177`；`TEAM_POLICY: 600` 见 `packages/core/system-prompt/src/index.ts:135`）；
  变化事实（lead/teammate 身份与名字）走 `systemPrompt.context({ name: 'team:identity', … })`（`:178-185`）
  ——**身份类事实不进静态前缀**。
- Pony 侧：本波**只**注入静态策略段；成员名单/角色/任务板计数**禁止**进稳定前缀（由工具结果或动态 Developer 层承载）。

### C3.3 落层位置（保持前缀缓存稳定与层序不变量）

- 注入为**紧跟 `BASE_SYSTEM_PROMPT` 之后的第二条 `ProviderRole::System` 消息**：
  `context.rs:451-454`（`base_system_messages` push 策略段）、组装点 `:468-484`、层序 `debug_assert!` `:483-487`；
  Anthropic native 面同源 `:533-552`（`:547-552`）。
- **禁止改写 `BASE_SYSTEM_PROMPT` 本体**：`context.rs:2999` 断言 `base_system_prompt().len() == 3790`。
- 不变量必须保持：
  1. `prompt_layers_are_ordered(messages) == true`（`context.rs:90-111`）：System 全在 Developer 之前；
  2. 首条 System 以 `You are Pony Agent` 开头（BASE 仍是首条，天然满足）；
  3. 前缀稳定：门控值由 `OnceLock`（`subagent_runtime.rs:37-45`）在 bootstrap 后恒定 → 不产生 per-turn 抖振；
  4. **衍生计数一致**：多注入的 System 条数必须由 `reserved_system_message_extra_count()`（`context.rs:84-86`）单一表达，
     其全部消费点（`trace_timeline.rs:198`、`control_plane/mod.rs:3230`、`context.rs:558`）必须与实际注入条数一致（0 或 1）
     —— 否则 `message_count > 2` 这类代理判定会恒真（L3-P finding#9）。
- **动态前缀污染防护**：策略段严禁包含 `session=` / `graph=` 子串（`provider::looks_like_dynamic_prefix_content` 会把命中
  的 System 消息判为动态前缀，导致 stable/semi-stable 分层错位）。

### C3.4 段落内容要点（本仓真名口径，逐项可查）

策略段必须以标题行 `Agent Teams is available in this session` 开头（与 DSH `:36` 同一锚串，便于机器判定），并覆盖：

1. **共享工作区**：Lead 与所有队友共享同一工作目录与文件系统，编辑对彼此立即可见；写域必须切分为互不重叠的 scope
   并登记在共享任务上；write-scope 重叠只是 advisory，**不是锁**。
2. **CAS 冲突重试**：文件写冲突的错误码是 **`cas_conflict`**（真值源 `tools.rs:2478`/`:2514`/`:2672`）；命中后应读取当前文件、
   把意图 rebase 到新内容后重试。**禁止**写成 `FS_STALE_VERSION`（本仓无此码：
   `grep -rn FS_STALE_VERSION crates/pony-agent-core/src` 为空）。
3. **进程类路径无版本守卫**：shell/格式化/代码生成/脚本**不受**文件版本守卫保护——该路径在本仓的工具名是 **`Run`**
   （`model_visible_tool_name_opt` `tools.rs:7276`：`run_command => "Run"`）。**禁止**写成 `Bash`（本仓无 `Bash` 工具名：
   `grep -rn '"Bash"' crates/pony-agent-core/src` 为空）。
4. **任务板**：共享任务流程＝ `team_task_list` 读出看板（每行回显当前 revision）→ `team_task_update`（`action: claim` +
   `expected_revision`）→ 执行 → `team_task_update`（`action: complete` + 新 revision）；任务就绪**不会**启动 owner。
5. **send_message 语义**：target 取自 `spawn_teammate` 或 `list_agents` 返回；running 目标在最近 step 边界收到，
   inactive 目标会被启动/恢复；消息**以发送者名开头**（`orchestration.rs:115` `QueuedMessage` 的 sender 字段）；
   **发送成功 ≠ 已被处理**。
6. **成员状态单一真源**：只以 `list_agents` 返回的 `status` 字段为准（`running` / `inactive` / `provisioning` / `failed`）；
   `inactive` 仅表示"当前没有 turn 在执行"，**不**代表任务完成/成功/失败；`provisioning` 表示成员创建中，`failed` 表示创建失败。
   推进成员用 `send_message`；每次动作后**重新 list**（`list_agents` + `team_task_list`），不假设进展。
7. **成员创建与收尾**：需要写入或较长的独立运行时用 `spawn_teammate` 派生队友，用 `team_task_*` 维护任务板；
   `interrupt_agent` 中止某队友当前 turn。Lead 在给出最终答复前**不得**仍有必需队友处于 running/provisioning：
   反复 `list_agents` + `team_task_list` 直到必需成员 `inactive` 且任务完成（或 `failed`），并如实报告失败。

> **v1.2 文本口径修正（L3-P BLOCKING #1）**：策略段只能引用**本仓真实存在**的工具名与错误码（`Run`、`cas_conflict`、
> `subagent`/`spawn_teammate`/`list_agents`/`send_message`/`interrupt_agent`/`team_task_*`）；
> **禁止**引用参考实现专有物：`Bash`、`FS_STALE_VERSION`、`wait_agent`。
> 依据：本仓 `wait_agent` 是**宿主侧 Rust API**（`orchestration.rs:538`，仅 `tools.rs:58` re-export），
> **不在** `builtin_tools()` 工具面 → 模型无法调用，写进策略段即为不可执行指引（L3-P finding#11）。

### C3.5 验收断言（可机器判定）

- **C3-A1**（已接线）：组装输出的 `ProviderRole::System` 消息数 `== 1 + reserved_system_message_extra_count()`，
  且额外那条（存在时）以 `Agent Teams is available in this session` 开头；
  `prompt_layers_are_ordered(&messages) == true`；`messages[首个 System].content == base_system_prompt()`（逐字节）。
- **C3-A2**（未接线）：`reserved_system_message_extra_count() == 0`，且全体消息文本中不存在字符串
  `Agent Teams is available in this session`。
- **C3-A3**（静态性）：`AGENT_TEAM_POLICY_PROMPT` 定义处无任何 `format!`/`{}` 插值（源码级判定：该常量为纯字面量）。
- **C3-A4**（前缀稳定）：同一进程内连续两个 turn 的策略段文本逐字节相等（长度与内容哈希一致）。
- **C3-A5**（真名口径·正）：策略段文本包含 `Run` 与 `cas_conflict`，且包含 `list_agents`、`spawn_teammate`、`send_message`。
- **C3-A6**（真名口径·负）：策略段文本**不含** `Bash`、`FS_STALE_VERSION`、`wait_agent`；
  且不含 `session=` / `graph=` 子串。
- **C3-A7**（动态层隔离）：`reserved_system_message_extra_count()` 的 3 个消费点（`trace_timeline.rs:198`、
  `control_plane/mod.rs:3230`、`context.rs:558`）与实测注入条数一致，无硬编码 `+1`。

---

## C4 冻结测试演进门禁（7 个面，Test Agent 执行，Lead 一票冻结）

> **纪律**：冻结断言只能**收紧或等价改写**，**不得放宽**（禁删用例、禁 `#[ignore]`、禁换成 `println!`、禁吞异常）。
> 每条改写必须在本节留痕（旧断言 → 新断言 + 理由 + 真值源）。

### C4.1 面①：`crates/pony-agent-core/tests/tool_surface_wave20_contract.rs:4`

- 旧断言：`test_stub_tools_are_no_longer_model_visible`，`:8-17` 逐项 `assert!(!names.contains(&"<tool>"), "… must not be exposed to models")`（10 项）。
- 新断言（**9 项反转 + 1 项保持负向**）：
  - 9 项（`subagent`、`spawn_teammate`、`list_agents`、`send_message`、`interrupt_agent`、`drain_inbox`、
    `team_task_create`、`team_task_update`、`team_task_list`）→ `assert!(names.contains(...))`，并追加
    `tool_exposure_for_name(...) == ModelVisible` 与未接线时 `error.code == "capability_unavailable"`（"不再是 stub"的真正立意）；
  - **`workflow` 保持负向断言**：`assert!(!names.contains(&"workflow"))`，失败信息改为
    `"workflow must stay internal: orchestration.rs:286-297 is still a stub (C1.7)"`——这是**新增的强断言**，不是放宽。
- 用例更名建议：`t1_team_tools_are_model_visible_and_workflow_stays_internal`。
- 保留不动：`:20-31` `test_goal_tools_are_canonical_and_model_visible`。

### C4.2 面②：`crates/pony-agent-core/tests/wave37_tool_surface.rs`（`t1_model_visible_surface_is_exact_29_tool_list`，原 `:81`）

- 旧断言：`names.len() == 29`；29 项逐字清单。
- 新断言：`names.len() == 38`，`expected` 逐字替换为 C1.2 的 38 行清单（含顺序）；用例更名
  `t1_model_visible_surface_is_exact_38_tool_list`；kind 表覆盖新增 9 项
  （`subagent` → `composite`；`spawn_teammate`/`list_agents`/`send_message`/`interrupt_agent`/`drain_inbox` → `interactive`；
  `team_task_create`/`team_task_update` → `write`；`team_task_list` → `read`；kind 真值源 `tools.rs:7362-7370`）。
- **追加断言**：`!names.contains(&"workflow")`（锁定 C1.7）。

### C4.3 面③：`crates/pony-agent-core/tests/wave25_capability_honesty_test.rs`

- `t1_fake_capability_tools_are_absent_from_model_visible_surface`（原 `:124`）：常量 `EXPECTED_MODEL_VISIBLE_TOOLS`（原 `:52-83`）
  扩为 38 项（与 C1.2 逐字一致）；原 10 个"假能力"名字拆为两组：
  `TEAM_TOOLS_VISIBLE: [&str; 9]`（断言**必须出现**）+ `WORKFLOW_INTERNAL: &str = "workflow"`（断言**必须不出现**，理由 C1.7）。
- `t4_fake_capability_descriptors_are_not_model_visible`（原 `:400`）：对 9 项断言 exposure **==** `ModelVisible`；
  对 `workflow` 断言 exposure **==** `Internal`（方向反转 + 保留原负向语义）。
- `t2_fake_capability_tools_return_capability_unavailable_error`（原 `:210`）：**保持不变**——它是 C2.3「未接线 fail-closed」的锚点
  （该文件不构建 runtime，槽为空，语义仍成立）。可改名，断言不得改。

### C4.4 面④：`crates/pony-agent-core/src/agent/provider/mod.rs`

- `openai_tools_payload_uses_product_tool_surface_for_builtin_tools`（原 `:5140`）与
  `anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools`（原 `:5219`）：逐字 29 项数组 → **38 项**
  （插入位置同 C1.2：`ViewImage` 之后、`TodoWrite` 之前；`workflow` 不出现）。
- `fallback_tool_payload_filters_internal_builtin_but_keeps_dynamic_tools`（原 `:5195`）：原以 `subagent` 作为"Internal builtin
  必须被 fallback 过滤"的样例。**v1.2 起改以 `workflow` 作样例**（C1.7 后它是本仓唯一函数级 `Internal` builtin，
  见 `is_internal_builtin_tool_name` `:3091-3095` + `tool_exposure_for_name` `:7413-7470`），**保留原断言形状**
  （`assert_eq!(names, vec!["mcp__demo__echo"])`）与动态工具透传断言 → 无需改写为"空集"形式（v1.1 方案作废）。
- **并发写域风险（Lead 必读）**：该文件在 wave-60/61 期间曾被并发会话改动；Test Agent 迁移前必须与 Lead 确认时序避免 hunk 冲突。

### C4.5 面⑤：`crates/pony-agent-core/src/agent/tools.rs`（characterization 用例，原 `:6611`）

- 用例：`characterization_product_surface_keeps_unique_names_and_json_object_schemas`
  —— `assert_eq!(tools.len(), 45)`（**不变**，内部注册数不因 exposure 变化）、`assert_eq!(names.len(), 29)` → **38**、
  29 项清单 → C1.2 的 38 行；`input_schema` 的 object/properties 校验保留。

### C4.6 面⑥：`crates/pony-agent-core/src/agent/tools.rs`（registry 投影用例，原 `:6802`）

- 用例：`builtin_registry_projects_the_existing_product_surface` —— `names.len() == 29` → **38**；
  追加 `registry.resolve("workflow").exposure == Internal`（C1.7）与 9 项 `ModelVisible` 断言；
  既有 `resolve("ToolSearch") == Deferred`、`snapshot_id == "builtin-tool-catalog-v1"` 断言**不得**改动。

### C4.6b 面⑦：`crates/pony-agent-core/tests/pa110_teams_acceptance.rs`（AC-05 队列顺序用例）

- 用例：`ac05_...`（AC-05 队列/插队顺序语义，`tests/pa110_teams_acceptance.rs:171-221`）。
- **旧断言**：3 次经模型可见入口 `send_message(...)`（`Queue`、`Queue`、`Steer`）后校验收件箱顺序与条数。
- **新断言**：同样 3 次入队改经 **additive seam** `orchestration::enqueue_message(...)`（`orchestration.rs:576`，纯入队、无驱动副作用），
  **顺序/条数断言逐字未改**（`:204-220`）。
- **理由**：C7 起 `send_message` 在入队后**立即后台驱动**队友 turn（真实执行，`send_message_in_workspace` `:525-552`），
  入队与消费竞态会让队列观测面不稳定；队列语义必须由纯入队 seam 验证，而"入队即真实驱动 + 回执"语义由
  `c7_send_message_drives_teammate_turn`（`tests/wave61_team_activation.rs:585`）覆盖。
- **授权**：**Lead 一票授权**（冻结测试迁移特批）；**提交号**：`922e960`（`feat(wave-61-team): real teammate execution
  + workspace-root propagation + honest 38-tool surface`）。
- 断言强度：未放宽（顺序/条数断言逐字保留）；新增 seam 是被测出口，覆盖点从"模型入口"下移到"队列层"，
  模型入口一侧由 C7-A1/C7-A3 补偿。

### C4.7 面⑤/⑥ 的执行方式（编译衍生同步豁免）

面⑤/⑥ 与被迁移的生产代码同处 `crates/pony-agent-core/src/agent/tools.rs`（同 crate、同编译单元），**不单独出红相**，
故按 **「编译衍生同步豁免 + Lead 复核 diff」** 处置：

1. Test Agent 在 C1 实现同一次提交内完成两处断言同步（38 项 + 清单逐字）；
2. 豁免仅限"无独立红相"，**不放宽断言**；
3. Lead 逐行复核（C4-A3）；
4. 若未同步，`cargo test -p pony-agent-core --lib` 必红（C4-A2），视同门禁失败。

### C4.8 门禁

- **C4-A1**：`cargo test -p pony-agent-core --test tool_surface_wave20_contract --test wave37_tool_surface --test wave25_capability_honesty_test` 全绿（Exit 0）。
- **C4-A2**：`cargo test -p pony-agent-core --lib` 全绿（面④/⑤/⑥）。
- **C4-A3**：`git diff` 中上述 7 个面的断言数量只增不减（旧断言逐条可映射；面⑦的 seam 迁移以 `922e960` 为留痕基准）。
- **C4-A4**：冻结由 Lead 一票决定：本文件状态为 `FROZEN v1.2`，测试断言后续改动必须走 successor 契约。

---

## C5 同源门控对齐（DSH `install()` 同源注册的 Pony 等价）

DSH 中团队工具注册与策略段由**同一个** `install()` 同步注册（`tool-agent-team/src/index.ts:168-183`），
故"工具可用 ⇔ 策略存在"是结构保证。Pony 侧两者由同一判定 `subagent_runtime_wired()` 派生，但**分工不同**：

- 工具**可见性**：静态（不门控）—— 保证 provider 工具数组逐字节稳定；
- 工具**可执行性** + **策略段**：同一门 `subagent_runtime_wired()`。

### C5.1 验收断言（可机器判定）

- **C5-A1**：`subagent_runtime_wired() == true` 时，`builtin_turn_tool_contract_views()` **包含**全部 9 项团队工具
  （且不含 `workflow`）**且** System 稳定层文本包含 `Agent Teams is available in this session`。
- **C5-A2**（反向边界）：`subagent_runtime_wired() == false` 时，工具面仍为 38 项（含 9 项），策略段**缺席**，
  且经 ToolRouter 调团队工具返回 `capability_unavailable`——三者必须同时成立。
- **C5-A3**（同源证据）：C3 注入点（`context.rs:76-78`）与 `tools.rs` 10 处 fail-closed 门使用**同一个**判定
  `subagent_runtime_wired()`（`grep -n "subagent_runtime_wired()" crates/pony-agent-core/src` 覆盖 11 处，无第二套接线判定）。

---

## C6 并发契约（修正 L3-P Q3 / 许可守恒）

### C6.1 单点扣减 + 自持门能力标志（Lead 裁决，已替代 v1.2 的方案 A/B）

**现状缺陷（v1.2 记录，`@60e7cb8`）**：同一静态许可被取两次 —— `orchestration.rs:256` 入口门 + `subagent_runtime.rs:153`
`HostTurnEngine::run_turn` 内门，命中同一 `SUBAGENT_INFLIGHT`（`subagent_runtime.rs:60`/`:67-80`），中途不归还
→ `SUBAGENT_MAX_CONCURRENT = 2` 下生产有效并发由 2 降为 1，第二个并发调用与"子代理内再派生 subagent"必 `subagent_busy`
（L3-P BLOCKING：`orchestration.rs:256`）。

**裁决方案（Executor 已落地，**优于** v1.2 列出的方案 A/B，二者已作废）**：

1. `SubagentTurnEngine` 新增 **additive** 能力标志方法
   `fn gates_concurrency_internally(&self) -> bool { false }`
   —— 默认 `false`（`subagent_runtime.rs:38-47`）：注入式引擎（测试 Stub、宿主自定义引擎）**不自持**门。
2. `HostTurnEngine` 覆写为 `true`（`subagent_runtime.rs:164-166`）：它是许可的唯一真源（`run_turn` 内 `subagent_gate_enter`）。
3. `orchestration::subagent()` **仅在 `gates_concurrency_internally() == false` 时**于入口扣门
   （`orchestration.rs:255-266`：`let gate = if engine.gates_concurrency_internally() { None } else { Some(subagent_gate_enter(&args.prompt)?) };`，
   后台分支把 `gate` move 进工作线程）。

**结果（必须同时成立）**：
- 自持引擎（`HostTurnEngine`）单次 `subagent()` **只扣 1 个许可** → 有效并发恢复 **2**；
- 注入式 Stub 仍在入口门 → **冻结用例 `t3_j`（`tests/wave37_tool_surface.rs:460-478`）与 `t3_k`（`:446-462`）零迁移**，
  wave-37 C3.2「Stub 引擎注入场景下 busy/oversize 契约同样生效」不回退。

**规则**：`InflightGuard::acquire()`（`subagent_runtime.rs:67-80`）在**任一次** `subagent()` 调用路径上只允许被扣减 **1 次**；
体积门（`SUBAGENT_MAX_PROMPT_BYTES`，`:56`）与并发门在**同一处**判定。

- **C6-A1**：`HostTurnEngine` 的 `gates_concurrency_internally() == true`；任一注入式 Stub 的默认实现 `== false`（不得被覆写为 true 以绕过入口门）。
- **C6-A2**：`orchestration::subagent()` 中 `subagent_gate_enter` 出现在**单一条件分支**内（源级判定：同一路径只可能执行一次）。
- **C6-A3**：3 个并发 `orchestration::subagent()`（barrier/sleep 注入）的**同时在跑引擎数** ≤ 2，且**等于 2**（自持引擎不再被压成 1）。

### C6.2 超时许可守恒

**现状缺陷**：`run_turn` 在**父线程**持有 `_inflight`（`subagent_runtime.rs:153`）；超时分支直接
`return Err("subagent_timeout: orphan session_id=…")`（`:203-208`）→ 父线程返回时 Guard Drop **提前归还许可**，
而孤儿工作线程（`:167-201` spawn）仍在跑 → 实际并发可突破 `SUBAGENT_MAX_CONCURRENT = 2`（L3-P finding#3 LEAK）。

**规则**：许可所有权必须 **move 进工作线程**（`std::thread::spawn` 的 move 闭包内持有），线程正常结束或
`catch_unwind` 展开结束时才释放；父线程的超时返回**不得**释放许可。

> **实施状态（v1.4 实测，已闭环）**：`subagent_runtime.rs:206` `let _inflight = inflight;` 已把许可 move 进工作线程
> （注释 `Wave-61 C.2`），父线程超时返回不再提前归还许可 → 原缺陷（并发可突破 2）已消除。
> **残余后果（必须登记）**：孤儿线程会**持续占用许可**直到其自建 runtime 结束（provider 通道 30min 上界），
> 期间新的 `subagent()` / `drive_teammate_turn_in()` 会持续命中 `subagent_busy` —— 见 §6.2 豁免条目的 `guarantee`。

- **C6-A1**：超时路径下（父线程已返回 `subagent_timeout`），孤儿线程存活期间不得出现"在跑 turn 数 > 2"；
  线程结束后许可可重新获取。
- **C6-A2**：3 个并发 `orchestration::subagent()`（barrier/sleep 注入）的**同时在跑引擎数** ≤ 2。
- **C6-A3**：`subagent_gate_enter` 的调用点在 `subagent()` 单次路径上计数为 1（源码级判定）。

### C6.3 既有 NFR 值不变（复登）

| 值 | 真值源 | 值 | 本波处置 |
|---|---|---|---|
| `subagent_max_concurrent` | `subagent_runtime.rs:58` `SUBAGENT_MAX_CONCURRENT` | `2` | **值不得改动**；在 `nfr-baseline.json` 增量复登 |
| `subagent_turn_timeout_ms` | `orchestration.rs:237` `SUBAGENT_TURN_TIMEOUT_MS` | `120000` | 同上 |
| `subagent_max_prompt_bytes` | `subagent_runtime.rs:56` `SUBAGENT_MAX_PROMPT_BYTES` | `200000` | 同上 |

`subagent_turn_timeout_ms = 120000` 与 `external_call_timeout_ms = 3000` 的关系：前者是**端到端子代理 turn 预算**
（经 `run_turn(prompt, timeout_ms)` 注入，`orchestration.rs:265`/`:276`），不是请求级往返预算；与流式例外同性质，
故以**独立字段**登记（答复 L3-P finding#4 TIMEOUT ADVISORY 与 finding#10 NFR_VIOLATION「权威 baseline 未复登」）。

- **C6-A4**：`.dev-team/nfr-baseline.json` 三字段值与上表逐值相等（`2` / `120000` / `200000`），且与源码常量一致。

---

## C7 真实能力契约（诚实性：模型可见工具必须是真实现）

**依据 L3-A@`60e7cb8` BLOCKING（`orchestration.rs:299`）**：`@60e7cb8` 时
`spawn_teammate`（`:299-340`）丢弃 `args.prompt` 且成员恒置 `Active`（`:305`）；
`send_message`（`:365-423`）只把消息塞进进程内 `VecDeque`，**无消费者**；
`interrupt_agent`（`:426-439`）只翻状态标志（引擎无 cancel）；
`drive_teammate_turn`（`:569`）只有定义、无生产调用点；`workflow` 伪造成功（C1.7）。
而 `tools.rs:1658/1678/1707/1736/1765/1794/1823` 仅以 `subagent_runtime_wired()`（`tools.rs:152` = 引擎槽非空）放行
→ 模型拿到 `status:"ok"` 的**零执行**能力，正是 wave-25 冻结的假能力回归
（`tests/wave25_capability_honesty_test.rs:224-232`）。

**规则**：9 项模型可见团队工具必须是**真实现**——有消费者、有状态迁移、回执有真实来源；
放行门必须是**能力门**（真实可执行）而不是"引擎槽非空"。

- **C7-A1**（驱动执行）：`send_message` 入队后必须被**驱动执行**——注入的记录型 Stub 引擎的
  `run_turn` 调用计数 **≥ 1**（且入参 prompt 含该消息内容）。
- **C7-A2**（状态机可见）：成员状态必须发生 `Running → Inactive` 迁移，且该迁移可经 `list_agents` 观测到
  （不得恒为 `Active`；`inactive` 定义与 C3.4-6 同源）。
- **C7-A3**（回执真实）：队友回执必须可从 Lead 收件箱经 `drain_inbox` 取到，且**内容来自引擎输出的
  `SubagentRunOutcome.final_text`**（逐字节可比对），禁止伪造成固定串。
- **C7-A4**（中断语义·已知限制，必须显式登记）：`interrupt_agent` 只保证"**下一次 `drain_inbox` 之前**停止"，
  不保证立即中止正在执行的 turn——引擎无 stop API（`subagent_runtime.rs` 无 cancel 面，超时路径亦只能遗留孤儿）。
  该限制必须同时出现在：① 本契约（本条）；② 策略段/工具描述的用户可见说明；③ 相关测试的断言注释。
  **不得**在文档或 UI 中承诺"立即停止"。
- **C7-A5**：`workflow` 仍为 `Internal`（C1.7 保持，恢复条件不变）。

> **门禁含义**：C7 的断言必须由**注入式 Stub 引擎**（可计数/可记录/可回放）验证，不得用真实 provider 做断言前提；
> 若实现无法满足 C7-A1..A3，则对应工具必须退回 `Internal`（同 C1.7 处置），不得以"已接线"放行。

## C8 工作区根透传契约（桌面宿主 workspace_root 缺失修复）

**依据 L3-A@`60e7cb8` BLOCKING（`runtime/builder.rs:87`）**：桌面漏斗
`control_plane/mod.rs:1042` 是 `AgentRuntimeBuilder::new().plan_store(..).build()`——**没有** `.workspace_root()`；
`HostControlPlane::new()`（`:1315-1317`）→ `DesktopHostPreset::build()`（`:1311-1313`）被 `src-tauri/src/lib.rs:1339`
直接 `manage`，因此 `ensure_host_subagent_engine(None)`（`builder.rs:87`）注册的引擎根为 `None`
→ `resolved_workspace_root()`（`subagent_runtime.rs:136-146`）回落 `compute_default_workspace_root()`（`workspace.rs:137-145`），
`run_turn` 又没有 per-turn 根参数 → **桌面宿主每个子代理/队友 turn 都建在默认工作区**，
违反 wave-58 F1-5，也与策略段"共享同一工作目录"的承诺（`context.rs:65`）矛盾。C2.4 只登记了多根争用，未覆盖该时序。

**规则（additive，不破坏既有调用点）**：

1. `SubagentTurnEngine` 新增 `fn run_turn_in(&self, prompt: &str, timeout_ms: u64, workspace_root: Option<&std::path::Path>) -> Result<SubagentRunOutcome, String>`
   —— **默认实现委托 `run_turn`**（忽略 root），注入式/第三方引擎零改动。
2. `HostTurnEngine` 覆写 `run_turn_in`：`Some(root)` 且为绝对路径 → 用该根；否则回落 `resolved_workspace_root()`。
3. `orchestration` 新增 `subagent_in_workspace(...)` 与 `drive_teammate_turn_in(...)`（队友驱动路径同样传根）。
4. `tools.rs` 的团队工具调用点必须传 `ToolRouter.workspace_root`（`crates/pony-agent-core/src/agent/tools.rs:1358` 字段）。
   —— 即以**调用期根**覆盖 C2 注册期根的缺失；这是本条的强制修复面，**不得**仅依赖注册期根。

**真值源**：`crates/pony-agent-core/src/agent/tools.rs:1358`（`ToolRouter { workspace_root: PathBuf, .. }`）、
`crates/pony-agent-core/src/agent/control_plane/mod.rs:1042`（桌面漏斗无 `.workspace_root()`）。

- **C8-A1**：注入记录型 Stub（覆写 `run_turn_in` 记录入参），经 `orchestration::subagent_in_workspace(prompt, Some(root))`
  调用时收到 `Some(root)`，且与 `ToolRouter.workspace_root` 逐字节相等。
- **C8-A2**：`run_turn_in(..., None)` 时 `HostTurnEngine` 回落 `resolved_workspace_root()`（默认根），不 panic。
- **C8-A3**：`run_turn` 旧调用点行为不变（默认实现委托，向后兼容）；`tools.rs` 的团队工具路径全部走 `*_in_*` 变体传根。
- **C8-A4**（可选加固，非本波强制）：在 `control_plane/mod.rs:1042` 补 `.workspace_root(...)` 可消除注册期根缺失；
  但 C8-A1 是**必须**成立的调用期根来源，二者不得同时缺席。

## C9 单次求值条款（消除门控谓词二次求值的 splice 越界风险）

**依据 L3-A@`60e7cb8` ADVISORY（`context.rs:557`）**：`@60e7cb8` 的组装路径中
`agent_team_policy_enabled()` 在 normalized 路径（`context.rs:452`）与 native 路径（`:547`）各求值一次，
而 splice 下标又经 `reserved_system_message_extra_count()`（`:84-86` = `usize::from(agent_team_policy_enabled())`）
在 `:558`/`:568` **再次求值同一谓词** → 若谓词在两次求值之间由 false 变 true（或反之），
splice 起点 `1 + 0` 会与实际已 push 的 2 条 System 错位 → 下标越界/插入错位。

**规则**：单次组装内 `agent_team_policy_enabled()` **只求值一次**，结果存入局部布尔，
并贯穿 ① 是否 push 策略段、② splice 下标、③ native 与 normalized 两条路径（同构语义）。

- **C9-A1**：单个组装函数体内 `agent_team_policy_enabled()` 的**直接调用次数 ≤ 1**（源级判定）。
- **C9-A2**：同一组装内的 splice 下标由该局部布尔推导，**不得**再次调用谓词或 `reserved_system_message_extra_count()`
  的二次求值路径。
- **C9-A3**：splice 下标 == 实际 System 消息条数（`1 + extra`），越界在结构上不可能；
  native 与 normalized 两条路径的 extra 取值一致。

## C10 驱动可靠性契约（方案审查收据要求）

**依据**：方案审查 @`922e960` BLOCKING（`orchestration.rs:452`）与复审 @`922e960` BLOCKING（`:746` 丢消息 / `:461` 无单飞）。
规则：队友消息的"入队 → 驱动 → 回执"链路必须**既不丢失也不并发重复**，且状态机器可观察。

### C10.1 每目标单飞（同一 target 同时最多一个驱动循环）

- 进程级单飞标记：`static DRIVING: Mutex<Option<HashSet<String>>>`（`orchestration.rs:134`），
  由 RAII 句柄 `DrivingGuard`（`:138-147`）持有——**任何**退出路径（成功 / 引擎 Err / panic 展开）都清除标记（`Drop` `:141-146`）。
- `begin_driving(target)`（`:150-158`）返回 `Some(guard)` 才允许派生驱动线程；已有人在跑时**只入队**、不派生第二个线程
  （`send_message_in_workspace` `:533-549`）。
- 驱动体为**循环** `drive_loop_in`（`:556-568`）：`loop { drain → 空则退出 → run_turn → 投递回执 }`，直到收件箱为空才结束。
- **C10-A1**：同一 target 并发 `send_message` N 次时，同时在跑的驱动循环数 ≤ 1；
  `is_driving(target)`（`:161-168`）在驱动结束后必须为 `false`（无标记泄漏）。
- **C10-A2**：驱动循环在"收件箱非空"期间持续消费——入队 K 条消息后，引擎 `run_turn` 调用次数与消息批次一致且**最终收件箱为空**。
- **C10-A3**：`DrivingGuard` 的清除点只有一个 `Drop` 实现；panic 展开路径同样清除（无 `mem::forget`/无手工漏清）。

### C10.2 失败补偿（消息绝不丢失、不得声称已处理）

- 引擎返回 `Err`（`subagent_busy` / `subagent_timeout` / `prompt_too_large` / 任意错误）时：
  1. 本轮 drain 出的整批消息**按原顺序放回队首**（`requeue_messages` `:170-…`，`drive_teammate_turn_in` `:869`）；
  2. 成员状态回 `Inactive`（`:870`）；
  3. 向 Lead 收件箱投递**错误回执**（含 target、被回队条数、错误文本；`:872-882`），不含任何假成功；
  4. 函数返回 `Err`（`:883`）——上层不得把它呈现为"已处理"。
- 确定性失败（如 `prompt_too_large`）会**反复失败而消息仍在队列**（不静默丢弃），由调用方拆分消息解决。
- **C10-A4**：注入返回 `Err("subagent_busy")` 的 Stub 引擎后，`send_message` 的返回值/回执**不得**声称 `delivered`/已处理；
  收件箱条数在失败后与失败前**相等**，且队首顺序与入队顺序逐条一致。
- **C10-A5**：失败后 `list_agents` 中该成员状态为 `Inactive`，且 Lead 收件箱新增一条错误回执（`drain_inbox` 可取，内容含错误码）。

### C10.3 状态可观察序列

- 引擎在位时，成员状态序列必须为 **`Active`（入队/派生）→ `Running`（驱动开始）→ `Inactive`（收口）**
  （`set_teammate_status` 调用点：`:537` Running、`:853` Running、`:899-906`/`:871` Inactive），且全部经 `list_agents` 可观测。
- 无引擎降级路径沿用既有 `Active` 口径（`:888`），**不**伪造 `Inactive`（fail-closed，与 C7 诚实性一致）。
- **C10-A6**：以记录型 Stub 引擎跑一次 `send_message`，采样 `list_agents` 必须依次观测到 `Running` 与 `Inactive`
  （不得跳过 `Running`，也不得在 turn 仍在执行时报告 `Inactive`）。

> **实施状态（本契约写作时的实测）**：C10.1/C10.2/C10.3 已在**工作树**落地
> （`DELIVERY_STATE_DRIVING` `:128`、`DRIVING` `:134`、`DrivingGuard` `:138-147`、`begin_driving` `:150-158`、
> `is_driving` `:161-168`、`requeue_messages` `:170-…`、`drive_loop_in` `:556-568`），但**尚未提交**；
> `crates/pony-agent-core/tests/**` 中暂无引用 `is_driving` / `requeue_messages` / `DELIVERY_STATE_DRIVING` 的用例
> → **C10-A1..A6 目前为无断言状态（假绿风险）**，Test Agent 必须补用例，否则 C10 视为未收口。

## C11 已知局限与劝告性条款登记（不得隐藏）

以下条款经实测确认**在本仓无代码强制点**，属**协议劝告**或**已接受风险**；本契约显式登记，禁止在文档/UI 中反向承诺：

| # | 局限 | 实测证据 | 定性 / 处置 |
|---|---|---|---|
| L1 | 策略段「A peer message starts with its sender name」无代码对应 | `QueuedMessage`（`orchestration.rs:114-119`）只有 `id`/`content`/`is_steer`，**无 sender 字段**；入队存原文（`:576` 起 `enqueue_message`），驱动仅 `- {content}` 平铺（`:857-860`） | **劝告**（非强制）；策略段该句须在 successor 中改写或实现补齐 |
| L2 | 策略段「running 目标在最近 step 边界接收」无代码对应 | 实现实为"新起线程跑新 turn"（`drive_loop_in` `:556-568` + `drive_teammate_turn_in` `:836`），不存在 step 边界抢占点 | **劝告**；措辞应改为"入队并在下一次驱动循环被消费" |
| L3 | 协作协议（写域切分 / 任务板 / 收尾纪律）无代码强制点 | `team_task_*` 只维护数据结构；无 write-scope 校验、无收尾门（Lead 收尾仅靠策略段第 7 条） | **劝告**；不得在任何位置声称"强制/校验" |
| L4 | `provisioning` / `failed` 两状态**无任何写入点** | `TeammateStatus` 枚举含两状态（`:55-61`），全仓仅读（`list_agents`），无 setter 调用 | **劝告 + 待修**：策略段宣传这两态会让模型等待永不出现的状态 |
| L5 | `interrupt_agent` 承诺与实现不一致（复审 BLOCKING） | 实现只翻状态位（`:661-675`），驱动仅在两个检查点观察（`:841` 进入前、`:845` drain 前）；**已进入的 turn 跑完并照常写工作区、照常发回执**；而工具描述（`tools.rs:6053`）与策略段（`context.rs:72`）写"aborts ... current turn" | **C7-A4 要求的三处登记未同步**（契约②/策略段③/测试注释）；必须补齐并改写承诺为"下一次 drain 前停止"；**不得**承诺立即停止 |
| L6 | 无 kill-switch | 无配置开关可关闭团队工具/策略段；回滚只能代码回退（revert `60e7cb8`/`922e960`） | **已接受风险** + 后续 backlog（不阻塞本波） |
| L7 | `spawn_teammate` 自动驱动分支与 C8 `ToolRouter` 接线**无独立用例** | `c7_send_message_drives_teammate_turn` 覆盖主路径；`spawn` 的 auto-drive（`:344-365`）与 tools→会话根链路无断言 | **覆盖缺口**：登记为待补用例（C7/C8 主路径已覆盖，故不阻塞） |

> **纪律**：L1–L7 中任何一条被修复后，必须在 successor 契约中删除对应行；**不得**在 L 未修复期间于策略段/工具描述/
> 发布说明中出现超出口径的承诺（诚实性底线，wave-25 冻结面同源）。

## C12 子代理工作线程的运行时上下文契约（真机举证发现的新 BLOCKING）

### C12.1 事实（链式证据）

1. **provider 流读取依赖 tokio reactor**：`crates/pony-agent-core/src/agent/provider/mod.rs:9` `use crate::agent::runtime_helper::block_on;`，
   流读取路径 `block_on(tokio::time::timeout(poll_tick, stream.next()))`（`provider/mod.rs:2733`；同构路径
   `provider/responses_api.rs:90`，注释见 `:60`）。`tokio::time::timeout` 需要**调用线程处于 tokio runtime 上下文**。
2. **子代理 worker 是裸线程**：`HostTurnEngine::run_turn` 用 `std::thread::spawn(...)` 起 worker
   （`crates/pony-agent-core/src/agent/subagent_runtime.rs` worker 闭包），该线程**没有** reactor 上下文。
3. **后果**：真机举证时队友 turn 在**首次 provider 调用**处 panic → 被 `catch_unwind` 映射为
   `Err("subagent_engine_panicked")`（`subagent_runtime.rs` 的 `mapped` 分支），表现为"队友永远不产出回执"，
   且错误信息不含 `no reactor running` 根因（被 catch_unwind 吞成通用串）。
4. **为何测试从未暴露**：注入式 Stub 引擎不调用 provider 流读取；而 **C2（宿主引擎接线）首次让 `HostTurnEngine`
   这条路径在生产可达** —— 即 C12 是 C2 的**暴露面**，不是新引入的回归。

### C12.2 规则

worker 闭包内、**构建 `AgentRuntimeBuilder` 之前**创建并**进入** current-thread tokio runtime：

```rust
let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()
    .expect("subagent tokio runtime");
let _enter = rt.enter();
// 之后才 AgentRuntimeBuilder::new()…build() + run_turn(...)
```

- `rt` 与 `EnterGuard` 必须活到**整个 turn 生命周期**（闭包结束即 Drop），与 `AgentRuntime` 同域；
- **panic 捕获语义不变**：`catch_unwind` 仍包裹闭包主体，panic 仍映射为 `subagent_engine_panicked`；
- **孤儿语义不变**：父线程超时返回仍遗留 worker（C6.2/C10 的既有口径不变，许可仍 move 进 worker 线程）。

> **实施状态（v1.7 实测，工作树）**：已实现于 `subagent_runtime.rs:208-220`
> （`new_current_thread().enable_all().build()` + `let _enter = rt.enter();` 位于 `AgentRuntimeBuilder::new()` 之前，
> 注释标注 `Wave-61 C12`）；**尚未提交**。

### C12.3 验收断言

- **C12-A1**（真机举证）：举证脚本 `examples/w61_e2e_probe.rs`（**临时产物**，用后已清理）输出 `RESULT=E2E_OK`；
  其中 STEP5 断言 `teammate_requests >= 1`（mock provider 侧计数）且 `lead_replies` 含 mock provider 的固定回执文本。
- **C12-A2**（根因消失）：队友 turn 执行线程内**不出现** `no reactor running`；
  失败路径不得只有 `subagent_engine_panicked` 而无根因（若仍 panic，必须保留原始 panic 文本可定位）。
- **C12-A3**（不回归）：既有 8 套件（`wave61_team_activation` / `pa110_teams_acceptance` / `wave37_tool_surface` /
  `wave25_capability_honesty_test` / `tool_surface_wave20_contract` / `orchestration_acceptance_test` /
  `team_task_order_contract` / `teammate_e2e_communication_test`）与 `cargo test -p pony-agent-core --lib` 全绿。

### C12.4 真机举证客观事实入账（evidence）

| 项 | 内容 |
|---|---|
| 举证方式 | 临时 example：`examples/w61_e2e_probe.rs`（跑 mock provider + 真实 `orchestration::send_message` / 队友驱动链路） |
| 观察输出 | `RESULT=E2E_OK`；STEP5 `teammate_requests >= 1`；`lead_replies` 含 mock provider 固定回执 |
| 修复前 | 同一脚本在首次 provider 调用处 panic（`subagent_engine_panicked`） |
| 产物留存 | **已清理**（`examples/w61_e2e_probe.rs` 在仓库中不存在）→ 仓库内**不可复现**，故标注 **[EVIDENCE-LEAD]**（由 Lead 真机执行并回报） |
| 结构化引用 | `.dev-team/green-phase-wave61-team.json` 的 `e2e_probe` 字段（命令 + 输出 + 产物留存状态） |

> **[EVIDENCE-LEAD]** 标记含义：该证据来自 Lead 的真机执行回报，本次会话无法独立重放（临时产物已删）；
> 其结构化形态已登记在 green-phase 的 `e2e_probe` 字段。C12-A1 的可复现化（把 probe 转成常驻 `#[ignore]` 用例或
> 独立 bin）登记为 backlog，不阻塞本波。

## 审查收据（L3-A / L3-P @`60e7cb8`；方案审查 / 复审 @`922e960`）

### A. L3-A（`.dev-team/review-wave61-a.json`）：`verdict: FAIL`，7 findings（2 BLOCKING / 5 ADVISORY）

| # | 严重度 | 位置 | 规则 | 处置（v1.2 / v1.3） |
|---|---|---|---|---|
| A1 | **BLOCKING** | `orchestration.rs:299` | CONTRACT_BREAK | 桩工具假能力：`spawn_teammate` 丢 prompt、成员恒 `Active`、`send_message` 无消费者、`interrupt_agent` 只翻标志、`drive_teammate_turn` 无生产调用 → **C7**（C7-A1..A5 + 无法满足则退回 Internal） |
| A2 | **BLOCKING** | `runtime/builder.rs:87` | CONTRACT_BREAK | 桌面漏斗 `control_plane/mod.rs:1042` 无 `.workspace_root()` → 引擎根为 `None`、子代理回落默认根 → **C8**（`run_turn_in` 调用期根透传 + `tools.rs:1358`） |
| A3 | ADVISORY | `context.rs:557` | CONCURRENCY | 门控谓词/下标二次求值的 splice 越界风险 → **C9**（C9-A1..A3） |
| A4 | ADVISORY | `runtime/trace_timeline.rs:198` | CORRECTNESS | 多出 System 条数须单一表达 → **C3.3-4 + C3-A7**（v1.2 已处置） |
| A5 | ADVISORY | `tools.rs:7212` | CONCURRENCY | 产品名回落 `Run` 污染 trace → **C1.6**（v1.2 已处置） |
| A6 | ADVISORY | `orchestration.rs:305` | CORRECTNESS | 成员恒 `Active`、无迁移 → **C7-A2**（`Running → Inactive` 可观测） |
| A7 | ADVISORY | `tests/wave61_team_activation.rs:290` | CORRECTNESS | 团队用例只校验标题锚串/System 条数，语义未落断言 → **C7-A1..A3** 落为可机器判定断言（与 L3-P #10 同点） |

### B. L3-P（`.dev-team/review-wave61-l3p.json`）：`verdict: FAIL`，11 findings（4 BLOCKING / 7 ADVISORY），六问 **Q1/Q3/Q4 FAIL**

| # | 严重度 | 位置 | 规则 | 处置 |
|---|---|---|---|---|
| P1 | **BLOCKING** | `orchestration.rs:286` | 模型可见 `workflow` 返回伪造成功 | **C1.7**（v1.2）：`workflow` → `Internal`；面 39 → **38** |
| P2 | **BLOCKING** | `context.rs:66` | 策略段引用本仓不存在的 `Bash` | **C3.4-3 + C3-A5/A6**（v1.2）：实数名 `Run` |
| P3 | **BLOCKING** | `orchestration.rs:256` | 同一静态许可被取两次（有效并发 2→1） | **C6.1**（v1.3）：`gates_concurrency_internally()` 自持门标志；方案 A/B 作废 |
| P4 | **BLOCKING** | `orchestration.rs:237` | `SUBAGENT_TURN_TIMEOUT_MS=120000` 未纳入基准映射（Q1 FAIL） | **C6.3 + §6.2 NFR 豁免**（v1.3）：`external_call_timeout_exceptions` 增条目 + 基线复登 |
| P5 | ADVISORY | `subagent_runtime.rs:201` | 超时分支遗留孤儿线程、许可提前归还（Q3 FAIL 之一） | **C6.2**（v1.2，仍开放）：许可须 move 进工作线程；孤儿线程登记为**已知限制** |
| P6 | ADVISORY | `context.rs:66` | `FS_STALE_VERSION` 本仓不存在（真实码为 `fs_not_observed` / `cas_conflict`） | **C3.4-2 + C3-A6**（v1.2） |
| P7 | ADVISORY | `context.rs:68` | 策略段宣称"消息以发送者名开头"，实现无 sender 拼接 | **C3.4-5** 保留承诺；实现补齐属 Executor，且 **C7-A1/A3** 以引擎输出比对兜底（未补齐则策略段须改写） |
| P8 | ADVISORY | `control_plane/mod.rs:2052` | 窗口内新增 6 处锁中毒 eprintln 使 `logging_eprintln_exempt_covers_new_lines=5` 口径失真（Q2 PASS） | **不改值**（NFR 字段冻结）→ 登记 dev-backlog |
| P9 | ADVISORY | `runtime/builder.rs:149` | 首写者胜静默无诊断、桌面根被固化 | **C8**（根因修复：调用期根）+ C2.4 限制保留；诊断面不新增日志（PA-119 债务） |
| P10 | ADVISORY | `context.rs:63` | 契约留痕缺项：策略段移除 `wait_agent` 未回流 C4 迁移表；C3 断言弱（只校验锚串+条数） | **v1.2 C3.4 已移除 `wait_agent` 并登记 C3-A6**；**v1.3 C7/C9** 补强语义断言 |
| P11 | ADVISORY | `control_plane/mod.rs:144` | `StopTurnCommand.stop_reason` 属 wave-60 引入的 breaking 未登记 | 非 wave-61 范围 → 登记 dev-backlog（不在本波契约内处置） |

### C. 六问收口（Q1 / Q3 / Q4）

- **Q1（外部调用超时）FAIL** → 处置：**登记为 NFR 豁免**（§6.2），不修改既有 `external_call_timeout_ms = 3000`。
  理由：`SUBAGENT_TURN_TIMEOUT_MS = 120000` 是**端到端 turn 预算**（`run_turn(prompt, timeout_ms)` 注入，
  `orchestration.rs:265`/`:276`/`:595`），不是单次外部往返预算；与既有 provider streaming 例外同性质。
  同时按 C6.3 在基线复登 `subagent_turn_timeout_ms=120000` / `subagent_max_concurrent=2` / `subagent_max_prompt_bytes=200000`。
- **Q3（资源泄漏）FAIL** → 处置：C6.1（许可唯一真源）+ C6.2（超时许可守恒，仍开放为**已知限制**：
  引擎无 stop API，孤儿线程最长存活到 provider 通道 30min 上界）。
- **Q4（并发与幂等）FAIL** → 处置：C6.1（自持门能力标志，消除双扣）+ C9（单次求值）；
  注册幂等（首写者胜）语义不变（C2.2-2）。

> **状态声明**：截至本契约 v1.3，P1/P2/P6 与 A4/A5 已在 v1.2 收口；**P3 已由 Executor 以 C6.1 裁决方案落地**；
> P4 由 §6.2 NFR 豁免 + C6.3 收口；**C7 / C8 / C9 为 v1.3 新增的实施面，其断言在实现落地前视为红相**。
> L3-A/L3-P 的最终 PASS 由 Lead 在下一轮复审给出。

### D. 方案审查（收口期补跑，`.dev-team/review-plan-wave61-team.json`）：FAIL @`922e960`，9 findings（2 BLOCKING / 7 ADVISORY）

| # | 严重度 | 位置 | 规则 | 处置（v1.4） |
|---|---|---|---|---|
| D1 | **BLOCKING** | `orchestration.rs:452` | CONCURRENCY（库层驱动无单飞与丢消息） | **C10.1 + C10.2** |
| D2 | **BLOCKING** | `tests/pa110_teams_acceptance.rs:187` | CONTRACT_BREAK（AC-05 迁移未落契约） | **C4.6b 面⑦**（Lead 授权 + `922e960` 留痕；旧→新断言与理由已写入 C4） |
| D3 | ADVISORY | `orchestration.rs:742` | CORRECTNESS | 驱动检查点/顺序 → **C10.1 / C10.3** |
| D4 | ADVISORY | `context.rs:63` | CORRECTNESS | 策略段留痕与断言强度 → **C11 L1/L2/L5 + C10 断言补强** |
| D5 | ADVISORY | `tests/wave61_team_activation.rs:832` | CORRECTNESS | 用例断言不足（假绿缺口）→ **C10 实施状态声明**（无 C10 断言即视为未收口） |
| D6 | ADVISORY | `orchestration.rs:764` | CONCURRENCY | 队友驱动路径的并发面 → 见 E8（C6-A3 覆盖范围缺口登记） |
| D7 | ADVISORY | 本契约 `:454`（v1.3 C6.1 段） | CONCURRENCY | 契约与实现一致性 → v1.3 已按 `gates_concurrency_internally()` 实现改写，v1.4 复核通过 |
| D8 | ADVISORY | `tools.rs:7466` | NFR_VIOLATION | 工具面项数口径 → **C1.2 / C1-A1**（38 项）复核 |
| D9 | ADVISORY | `context.rs:76` | NFR_VIOLATION | 策略段门控 → **C3.1 / C9-A1**（单次求值）复核 |

> D3–D9 审查件仅给位置+规则（无摘要文本），故按"位置+规则→契约条款"映射，未编造意图。

### E. 复审（`.dev-team/review-wave61-recheck.json`）：FAIL @`922e960`，13 findings（3 BLOCKING / 10 ADVISORY）

| # | 严重度 | 位置 | 要点 | 处置（v1.4） |
|---|---|---|---|---|
| E1 | **BLOCKING** | `orchestration.rs:746` | DATA_LOSS：先 drain 后跑 turn，Err 时消息已消费不回收；`send_message` 已返回 `delivered:true` | **C10.2**（回队首 + Inactive + 错误回执 + 返回 Err） |
| E2 | **BLOCKING** | `context.rs:72` | `interrupt_agent` 承诺与实现相反，C7-A4 要求的三处登记反向落地 | **C11 L5** + C7-A4（补齐 commit 前必须改写承诺口径） |
| E3 | **BLOCKING** | `orchestration.rs:461` | 无驱动去重/串行化：running 期间新消息会并发起第二个 turn、状态抖动 | **C10.1**（单飞标记 + DrivingGuard） |
| E4 | ADVISORY | `orchestration.rs:354` | `let _ = send_message_in_workspace(...)` 吞错、仍回 status:ok | 登记 backlog（不在本波写域） |
| E5 | ADVISORY | `orchestration.rs:464` | 新增非锁中毒 eprintln 落在豁免口径之外；`covers_new_lines=5` 失真 | §6.1 已存档；**不改既有值** → backlog（PA-119） |
| E6 | ADVISORY | `orchestration.rs:512` | `TEAMMATE_INBOXES` 无容量上界，回执含完整模型输出 | backlog（与 `resource_release` 的 bounded_queue 口径对齐） |
| E7 | ADVISORY | `orchestration.rs:461` | 每消息 `thread::spawn` 无 fallible 处理（实测 EAGAIN） | backlog（JobRegistry/线程池兜底） |
| E8 | ADVISORY | `orchestration.rs:768` | 队友驱动绕过入口门：注入式引擎下 team 路径无许可/无体积门 | **C6-A3 覆盖范围缺口**，登记；successor 须扩展到 `drive_teammate_turn_in` |
| E9 | ADVISORY | `tools.rs:1579` | 透传的是 router 默认根而非**会话根**（`resolved_workspace_root(context, ws_id)` 未被团队路径使用） | **C8 缺口登记**：C8-A1 只锁"透传"，未锁"会话绑定根"；successor 须补 |
| E10 | ADVISORY | `tests/wave61_team_activation.rs:772` | c7 用库层 `spawn_teammate`（非模型入口），spawn auto-drive 修复零断言；c7 未断言 prompt 含消息内容 | 见 **C11 L7**；C7-A1 断言须补"prompt 含消息内容" |
| E11 | ADVISORY | `tests/pa110_teams_acceptance.rs:204` | 实施者改写冻结用例需 Lead 追认 | **C4.6b 面⑦** 已按 Lead 一票授权 + `922e960` 留痕 ✔ |
| E12 | ADVISORY | `context.rs:68` | 策略段 sender 前缀与 `provisioning`/`failed` 两态均无实现对应 | **C11 L1 / L4** |
| E13 | ADVISORY | `.dev-team/nfr-baseline.json:70` | 豁免条目 `guarantee` 与 C6.2 修复后代码矛盾（许可已 move 进线程） | **§6.2 已校正**（surface 扩展到 `drive_teammate_turn_in`；guarantee 改为"孤儿占许可至 30min 上界"） |

> **收口判定**：E1/E2/E3 由 C10 + C11 承担（C10 实现已在工作树、**用例待补**；E2 的承诺改写属 Executor 的 src 写域，
> 本契约只登记"必须补齐，不得承诺立即停止"）。E4–E7、E9、E10 登记为 backlog/缺口，不伪造为已修。

### F. recheck 链（`fa` / `fp` / `fr` / `fv`）与真机举证 C12

| 件 | verdict | reviewed_commit | findings | 说明 |
|---|---|---|---|---|
| `.dev-team/review-wave61-fa.json` | **FAIL** | `ed6d35e` | 7（1 BLOCKING：`orchestration.rs:561` CONCURRENCY） | 驱动/中断并发面；已由 `6b9e23a`/`e403302` 修复链收口 |
| `.dev-team/review-wave61-fp.json` | **PASS** | `ed6d35e` | 7（0 BLOCKING） | 方案面通过 |
| `.dev-team/review-wave61-fr.json` | **FAIL** | `6b9e23a` | 8（1 BLOCKING：`orchestration.rs:892` CONCURRENCY） | 驱动循环/中断标记面；由 `e403302` 修复（idle interrupt 不再滞留下次驱动 + stale marker + 回归用例） |
| `.dev-team/review-wave61-fv.json` | **PASS** | `e403302` | 4（0 BLOCKING） | 终审通过；含 `c7e_interrupt_when_idle_does_not_strand_next_message` 实跑收据（`C7E_OK calls=1 delivery_state=driving prompts=1 final=Some(Inactive)`） |
| 收口提交 | `8481eda` | — | — | `chore(wave-61-team): final recheck chain receipts (A FAIL→closed, P PASS, v1.5/v1.6 regressions closed)` |

**真机举证（本轮新增 BLOCKING）**：真机 e2e 举证暴露 **C12**——`HostTurnEngine` worker 为裸 `std::thread::spawn`、
无 tokio reactor，而 provider 流读取用 `block_on(tokio::time::timeout(...))`（`provider/mod.rs:2733`、`responses_api.rs:90`）
→ 队友 turn 在首次 provider 调用处 panic（映射为 `subagent_engine_panicked`）。

**C12 与 C2 的关系（必须显式记录）**：C2（宿主引擎接线收敛到 `runtime/builder.rs:79`）**首次让 `HostTurnEngine`
在生产路径可达**；此前该路径只被测试以 Stub 引擎覆盖，Stub 不触发 provider 流读取，故该缺陷**不可见**。
即：C2 是**暴露面**、C12 是**被暴露的既有依赖缺口**（reactor 上下文），二者不互为回归。
处置：C12.2 规则 + C12.3 断言（A1 真机 / A2 `no reactor running` 消失 / A3 8 套件与 `--lib` 不回归），
实现已在工作树（`subagent_runtime.rs:208-220`）。

## 6. NFR 基准增量（`.dev-team/nfr-baseline.json`，additive-only）

追加字段（不改动任何既有字段；同步在 schema 中**增量**声明为可选属性，不进入顶层 `required`，
不关闭 `additionalProperties: false` 校验）：

| 字段 | 值 | 语义 | 门禁 |
|---|---|---|---|
| `team_tool_surface_exact_count` | `38` | 模型可见工具面精确项数（v1.2：29 + 9；`workflow` 排除） | C1-A1 / C1-A8 |
| `subagent_engine_registration_required_in` | `["tauri","harness","tui"]` | 必须**经由 core 唯一注册点**获得引擎的三条宿主 | C2-A1/A2 |
| `team_policy_prompt_injected_when_wired` | `true` | 已接线时必须注入静态策略段 | C3-A1 |
| `subagent_max_concurrent` | `2` | 子代理并发上限（值不变，v1.2 复登） | C6-A4 |
| `subagent_turn_timeout_ms` | `120000` | 子代理端到端 turn 预算（值不变，v1.2 复登） | C6-A4 |
| `subagent_max_prompt_bytes` | `200000` | 子代理 prompt 字节上限（值不变，v1.2 复登） | C6-A4 |

> `subagent_engine_registration_required_in` 的语义是"这三条宿主必须**受益于**注册"，实现方式收敛在
> `builder.rs:79` + `:138-156`，**不是**要求这三处源码被修改（C2-A2 反向约束）。

### 6.1 既有 schema 漂移存档（**HEAD 已存在，本波不修**）

严格门禁 `python3 .dev-team/nfr-lint.py`（`additionalProperties: false` + 未知字段拒绝）在 **HEAD 上已经红**。
基线口径：以 **c575243**（wave-61 提交前 HEAD）导出 instance+schema 后重跑，实测 **6 条错误**（`PRE_CHANGE_EXIT=1`），
错误项为 `$.observability`、`$.external_call_timeout_exceptions`、`$.logging_eprintln_exempt_note`、
`$.logging.trace_id` 四个"instance 有、schema 未登记"的漂移（由此前波次只改 instance 未改 schema 造成）。

**Lead 裁决（wave-61）**：该漂移与「团队工具面 / 宿主接线 / 策略提示」无因果关系，属**既有**缺陷；
按抗熵底线**禁止夹带修复进本波提交**。Protocol Agent 已按此裁决**回退**全部补登记动作（上述四处均已移出 schema）。

→ **登记为 dev-backlog 候选**（范围：`.dev-team/nfr-baseline.schema.json` 三处顶层既有字段登记
`observability` / `external_call_timeout_exceptions` / `logging_eprintln_exempt_note`，外加一处嵌套字段
`logging.trace_id`；纯增量声明即可，不得删除或改写既有取值）。
→ 本波 schema **只**新增 wave-61 自己的字段声明（v1.2 起为 6 个）；严格门在回退后与 HEAD **同为失败**
（exit code 见 §7，必须如实回报，不得为变绿而改 schema）。

### 6.2 NFR 豁免登记：`subagent_turn_timeout_ms = 120000`（答复 L3-P Q1 / P4）

在既有 `external_call_timeout_exceptions`（**追加条目，不改动既有 streaming 条目与任何既有取值**）登记子代理 turn 预算例外：

| 字段 | 值 |
|---|---|
| `surface` | subagent turn（`HostTurnEngine` / `orchestration::subagent` 的 `run_turn` / 队友驱动 `drive_teammate_turn_in`，`orchestration.rs:836`） |
| `code` | `crates/pony-agent-core/src/agent/orchestration.rs:237 SUBAGENT_TURN_TIMEOUT_MS = 120_000`（调用点 `:265`/`:276`/`:595`） |
| `semantics` | turn 级端到端预算（经 `run_turn(prompt, timeout_ms)` 注入），非 `external_call_timeout_ms = 3000` 的请求级往返预算 |
| `why_not_base` | 120000 是「一次子代理 turn 从建栈到产出」的总预算，套 3000ms 会截断任何真实子代理回合（含 provider 生成与工具往返）；与 `provider streaming (SSE)` 例外同性质 |
| `guarantee` | 超时以结构化 `subagent_timeout: orphan session_id=…` 返回；**许可已 move 进工作线程（C6.2 已闭环，`subagent_runtime.rs:206`）**，故不再提前归还；残余后果＝孤儿线程持续占许可直到 provider 通道 30min 上界，期间 `subagent`/队友驱动会持续 `subagent_busy`（引擎无 stop API，已登记） |
| `known_gap_ref` | L3-P@`60e7cb8` Q1 FAIL / finding `orchestration.rs:237`；C6.3 基线复登 `subagent_turn_timeout_ms=120000` |
| `l3p_blocking_resolution` | v1.3 按 Lead 裁决：登记豁免 + 基线复登；**不修改** `external_call_timeout_ms=3000`、不放宽既有 streaming 例外 |

> **基线口径声明**：wave-61 的 6 个增量字段（`team_tool_surface_exact_count=38` 等）**保持不变**；
> 本条只在 `external_call_timeout_exceptions` 数组内**追加**一条豁免记录。
> 该数组在仓库严格门禁 `nfr-lint.py` 中本就是 HEAD 既有漂移（§6.1），追加条目不改变错误条数（仍 6 条、同为既有项）。

## 7. 门禁与收据命令

```bash
# NFR 门禁（技能脚本，必须 Exit 0）
python3 /home/dm/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline.json

# 仓库严格门禁（jsonschema + additionalProperties:false）
# 预期：非 0（HEAD 既有漂移，见 §6.1）；脚本不接受位置参数（只用 --instance/--schema 或默认路径）
python3 .dev-team/nfr-lint.py

# 写域收据（只允许 .dev-team/ 下文件变化）
git status --porcelain -- .dev-team/
```

## 8. 未验证项（[UNVERIFIED]，禁止据此放行）

1. **[UNVERIFIED]** trace 中 28 项 `toolDefinitionsText` 与当前基线的构建指纹（推测为该 turn 早于 `skill_load` 落地）。
2. **[UNVERIFIED]** 注册点上移后 `cargo test -p pony-agent-core` 全量是否存在**测试执行顺序耦合**
   （构建过 runtime 的单测进程会把槽置为已接线，使"未接线"断言在**同进程**内不可达）。已核实 `context.rs` 内无消息条数精确断言
   （仅 `:2999` 的 `base_system_prompt()` 字节长度）；但全量 cargo test 未在本会话运行。
3. **[UNVERIFIED]** 9 项团队工具进入 provider payload 后，各 provider 侧是否存在工具数量/字节上限截断（未测真实请求体积）。
4. **[UNVERIFIED]** 多宿主同进程时 `workspace_root` 首写者语义的实际影响（C2.4）。
5. **[VERIFIED @工作树]** C6.1 已由 Executor 以 `gates_concurrency_internally()` 自持门标志实现（`subagent_runtime.rs:38-47`/`:164-166`、`orchestration.rs:255-266`）；**并发收益（有效并发恢复 2）未压测** → 仍属 [UNVERIFIED]。
6. **[VERIFIED @`922e960` + 工作树]** C7/C8/C9 已实现：`run_turn_in`（`subagent_runtime.rs:53` 默认实现 / `:294` HostTurnEngine 覆写）、
   `subagent_in_workspace`（`orchestration.rs:335`）、`drive_teammate_turn_in`（`:836`）、
   `agent_team_policy_enabled()` 在组装内单次求值（`context.rs:454` `let team_policy_injected = …`）；用例 `c7_*`/`c8_*`/`c9_*`（`tests/wave61_team_activation.rs:585/611/634`）。
   **仍属 [UNVERIFIED]**：c7 的断言强度（复审 E10：未断言 prompt 含消息内容、未覆盖模型入口 auto-drive）。
7. **[VERIFIED @工作树]** C10.1/C10.2/C10.3 已实现（`DRIVING` `orchestration.rs:134`、`DrivingGuard` `:138-147`、`begin_driving` `:150-158`、
   `requeue_messages` `:170-…`、`drive_loop_in` `:556-568`），但**未提交**且 **`tests/**` 无任何 C10 断言** → C10-A1..A6 处于假绿风险，
   在用例补齐前**不得**声称 C10 已收口。
8. **[UNVERIFIED]** C7-A4 的"下一次 drain 前停止"语义未做端到端计时验证（引擎无 stop API，只能以 drain 时序断言近似）；
   `interrupt_agent` 的承诺口径（工具描述 + 策略段）尚未按 C11 L5 改写。
9. **[UNVERIFIED]** 复审 E9 指出的"团队路径透传 router 默认根而非会话根"未验证：C8-A1 只锁"透传"链，未锁 `workspace_id` 会话绑定链。

---

## 附：本契约的写入边界声明

本契约由 Protocol Agent 写入，**只**新增/修改 `.dev-team/contracts/wave-61-team-activation.md`、
`.dev-team/nfr-baseline.json`（增量字段 + §6.2 的 `external_call_timeout_exceptions` **追加**一条豁免条目）、
`.dev-team/nfr-baseline.schema.json`（**只**新增 wave-61 自己的字段声明；按 Lead 裁决已**回退**所有既有字段补登记动作，
漂移仅存档于 §6.1 并登记为 dev-backlog 候选）。
未触碰任何 Rust 源码、测试、`.dev-team/state.json`。
