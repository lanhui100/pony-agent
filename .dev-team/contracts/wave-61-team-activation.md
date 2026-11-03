# Wave-61 契约：Agent Team 激活（工具面 × 宿主接线 × 策略提示）

- 契约编号：wave-61-team-activation
- 状态：`FROZEN v1.1（wave-61，Lead one-vote freeze @069587a；v1.1 增补 C1.6 + C4 面⑤/⑥）`
- 作者：Protocol Agent（协议特派员）
- 唯一写域：`.dev-team/`（本文件 + `.dev-team/nfr-baseline.json` + `.dev-team/nfr-baseline.schema.json` 的**增量**声明；schema 只含 wave-61 的 3 个字段）
- 禁止写域：任何 Rust 源码与测试（含 `crates/**`、`src-tauri/**`、`tests/**`）、`.dev-team/state.json`
- 分级：L3（协议契约 + 跨模块集成 + 宿主 bootstrap）

## 0. 缺陷因果（Lead 已用 trace 证据确认，本契约复述为设计前提）

`build_context_observations` 中 session-4bdcc39a / turn-0c9aebe0：dev-team 技能正文已注入，但该 turn 的
`toolDefinitionsText` 只有 28 项且**完全不含** subagent / workflow / spawn_teammate / list_agents /
send_message / interrupt_agent / drain_inbox / team_task_*。更早的 turn-62edc32c 工具面里有这些工具。
→ 结论：不是模型"忘了用"，而是**工具面在某个 turn 变异/收窄**，且即便可见也会 fail-closed。

三条独立真值源（本契约逐条给出 `path:line`）：

| 编号 | 真值源 | 现状 | 因果 |
|---|---|---|---|
| S1 可见性 | `crates/pony-agent-core/src/agent/tools.rs:7341` `tool_exposure_for_name()` → `:7381` `_ => ToolExposure::Internal` | 10 项团队工具全部落 `Internal` | `provider_contract_views()`（`tools.rs:622-631`）只保留 `ModelVisible` 或 `primitive_name == tool_search`；`TurnToolView::from_registry`（`tool_runtime.rs:161-176`）同一过滤，且 Dispatcher 对 `Internal` 直接拒（`dispatcher.rs:856-866` `origin_not_authorized`）→ 既不可见、也不可能被模型调用 |
| S2 可执行性 | `crates/pony-agent-core/src/agent/tools.rs:152` `subagent_runtime_wired()` = `subagent_engine().is_some()` | `set_subagent_engine` 在**生产代码中从未被调用**（仅 `tests/wave37_tool_surface.rs:282`、`tests/pa110_teams_acceptance.rs:28` 调用） | 一旦可见也会被 `tools.rs:1571-1573` 等 10 处 fail-closed 门打成 `capability_unavailable` |
| S3 策略面 | `crates/pony-agent-core/src/agent/context.rs:15` `BASE_SYSTEM_PROMPT` | 全文无任何「Agent Team 可用」策略段 | 参考实现 DSH 在 `packages/experimental/tool-agent-team/src/index.ts:34-58` 注入 `Agent Teams is available in this session` 策略段（`:173-177` 注册） |

> 28 vs 29 的口径说明：当前 `builtin_turn_tool_contract_views()` 为 **29** 项（W42/PA-149 增 `skill_load`，
> 见 `crates/pony-agent-core/tests/wave37_tool_surface.rs:11-13`、`:91-125`）。trace 观测到的 28 项与 29 项的差异
> 归因于该 turn 早于 `skill_load` 落地。**[UNVERIFIED]**：未在本次会话内回放该 trace 的构建指纹。

---

## C1 工具面暴露契约（10 项全部 `ModelVisible`）

### C1.1 目标状态

`tool_exposure_for_name()`（`crates/pony-agent-core/src/agent/tools.rs:7341`）必须为下列 10 个 primitive
返回 `ToolExposure::ModelVisible`（当前全部落 `:7381` 的 `_ => Internal`）：

`subagent`、`workflow`、`spawn_teammate`、`list_agents`、`send_message`、`interrupt_agent`、`drain_inbox`、
`team_task_create`、`team_task_update`、`team_task_list`（常量定义：`tools.rs:156-165`）。

product 名（对外模型名）＝ primitive 名本身：`model_visible_tool_name_opt()`（`tools.rs:7216-7255`）对这 10 项
**没有**映射分支，`product_visible_tool_name()`（`:7265-7269`）原样透传小写名。
→ 模型面工具名为小写原形，**不得**为它们新增 CamelCase 别名（别名表 `builtin_aliases_for_primitive()`
`tools.rs:7092-7101` 已是 `vec![同名]`，保持不变）。

### C1.2 39 项精确清单（模型可见面，含顺序）

顺序真值源＝ registry 排序键 `(!visible, product_first_index)`（`tools.rs:555-566`）＋ `builtin_tools()`
定义序（`tools.rs:5392` 起，10 项团队工具注册于 `:5925`/`:5952`/`:5983`/`:6014`/`:6023`/`:6047`/`:6066`/`:6081`/`:6110`/`:6138`，
夹在 `view_image`（`:5894`）与 `todo_write`（`:6147`）之间）。因此 39 项 = 既有 29 项保持相对顺序，
10 项团队工具**插在 `ViewImage` 之后、`TodoWrite` 之前**：

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
18. workflow
19. spawn_teammate
20. list_agents
21. send_message
22. interrupt_agent
23. drain_inbox
24. team_task_create
25. team_task_update
26. team_task_list
27. TodoWrite
28. TodoList
29. create_goal
30. get_goal
31. update_goal
32. Present
33. Find
34. GitDiff
35. JobStart
36. JobOutput
37. JobKill
38. JobList
39. skill_load
```

推导证据（逐项可复核，非猜测）：
- 既有 29 项顺序与 `tests/wave37_tool_surface.rs:91-125`、`tests/wave25_capability_honesty_test.rs:52-83` 的冻结清单逐字一致（本次实测读取）。
- `Run` 占 1 号位是因为 `time_now` 与 `run_command` 共用 product 名 `Run`（`tools.rs:7218-7219`），
  `contract_priority` 让 `run_command`（100，`:6480`）胜过 `time_now`（10，`:6465`），但槽位取
  product 名**首次出现**的下标（`:493-495`、`:555-566`）。
- `subagent`…`team_task_list` 的槽位为其定义下标 21..30，落在 `view_image`(20) 与 `todo_write`(31) 之间；
  这 10 项 product 名唯一，`contract_priority` 取默认 50（`:6498` `_ => 50`），无 winner 争用。

### C1.3 变更类型标注与兼容性证据

| 面 | 类型 | 依据 |
|---|---|---|
| 模型/协议面（provider tools payload） | **additive** | 既有 29 项的 name / description / input_schema / kind / permission facts 均不变：`ToolDefinition::contract_view()`（`tools.rs:5339-5356`）与 `from_builtin_definitions`（`:475-566`）**只**由 `exposure` 影响 `provider_contract_views()` 的过滤结果；descriptor 集合、`snapshot_id`（`builtin-tool-catalog-v1`）、别名表均不变。10 个新增条目是 payload 增量。 |
| Dispatcher 授权面 | **additive（放宽）** | `authorize_origin`（`dispatcher.rs:851-904`）仅按 `exposure == Internal` 与 `TurnToolView` 允许集判定；`ModelVisible` 且进入 turn view 后即获准。无 schema/权限语义变更（权限 facts：`tools.rs:7418-7460`，10 项均 `permission_scope: None`、`requires_approval: false`，本次**不得**改动）。 |
| 执行面（handler） | **无变更** | 10 个 handler 已存在并已接线（`tools.rs:1535-1544` 派发 → `:1571-1838` 实现），本波不新增实现、不改错误码。 |
| provider 工具数组前缀 | **breaking（一次性）** | 10 项插入 index 17..26，使原 index ≥17 的 13 项后移 10 位（`Run…ViewImage` 不变）。同一构建内该数组逐 turn 恒定，故只在升级后第一个 turn 发生一次前缀缓存 miss（ADR 0007 关注的是 **per-turn** 抖振，非一次性迁移）。 |
| 冻结测试契约面 | **breaking** | 4 处精确清单/负向断言必须演进，见 C4。 |

**为何不选择"把 10 个定义挪到 `builtin_tools()` 末尾以保持尾部追加"**：那会让 provider 数组变成尾部追加
（前缀只增不改），但需要移动 10 段注册代码（`:5925-6138`），产生更大 diff 且打乱"定义序即产品面序"的既有惯例
（W37-C1/W42 两次迁移都依赖该惯例）。判定：**接受一次性前缀失效**，不为省一次 miss 引入结构性移动。

### C1.4 为何不是 `Deferred`（对比 `tool_search` 提升路径）

`Deferred` 在本仓库是**一条受治理的发现→提升通道**，其边界是硬编码的：

1. 搜索面只索引 `Deferred`：`ToolSearchElevator::search()` 过滤 `descriptor.exposure == ToolExposure::Deferred`
   （`crates/pony-agent-core/src/agent/tool_search_elevation.rs:81-83`），`elevate()` 再次校验 `!= Deferred` 即报错
   （`:126-129`）；`TurnToolView::elevate_from_registry` 同样要求 `Deferred`（`tool_runtime.rs:198-200`）。
   → 若保持 `Internal`，这 10 项**永不出现在搜索结果**里；改成 `Deferred` 才可被发现。
2. 提升是 **per-turn** 的：`elevated_descriptor_ids` 属于单次 `TurnToolView`（`tool_runtime.rs:152-185`），
   不跨 turn 保留 → 采用 `Deferred` 意味着**每个 turn 都要多一次「tool_search 发现 + elevate」的模型往返**，
   团队协作是会话级常驻能力，不是按需能力。
3. `tool_search` 的 Deferred 是"默认延迟发现的**单一条目**"（`tools.rs:622-631` 的 filter 注释与白名单例外），
   把 10 项并入会让该通道承载 12 项，且与「让模型**第一 turn** 就能派生队友」的缺陷目标直接冲突。
4. 参考实现对齐：DSH 在 Agent scope **无条件**注册整套团队工具与策略段（`packages/experimental/tool-agent-team/src/index.ts:168-183`），
   不存在"先发现再提升"的一跳。
5. `Deferred` 无法替代 fail-closed：wave25 T2 已证明"看不到 ≠ 不可用/不可调用"（`tests/wave25_capability_honesty_test.rs:197-210`），
   因此正确的能力门是 C2/C3 的 `subagent_runtime_wired()`，而不是隐藏工具面。

结论：**`ModelVisible`，且可见性不随 `subagent_runtime_wired()` 变化**（避免工具面随进程状态抖动、破坏 provider 前缀稳定）；
"可用/可执行"才由 `subagent_runtime_wired()` 门控（C2 保持 fail-closed，C3 门控策略段）。

### C1.5 验收断言（可机器判定）

- **C1-A1**：`builtin_turn_tool_contract_views()`（`tools.rs:712`）返回项数 `== 39`，且 `Vec<name>` 与 C1.2 的 39 行清单**逐字、逐序相等**。
- **C1-A2**：对 10 个 primitive 名逐一断言 `tool_exposure_for_name(name) == ToolExposure::ModelVisible`。
- **C1-A3**：`builtin_tool_surface().provider_contract_views()`（`tools.rs:699`）与 `builtin_turn_tool_contract_views()` 的名字集合完全一致（交叉投影一致性）。
- **C1-A4**：既有 29 项中每一项的 `kind` / `exposure` / `permission_facts` 与迁移前一致（仅新增项进入可见面，不改旧项）。
- **C1-A5**（可执行性）：在 `subagent_runtime_wired() == false` 的进程内直接经 ToolRouter 调 `subagent`，返回 `error.code == "capability_unavailable"`；在 wired 进程中经 Dispatcher 以 `InvocationOrigin::Model` 调 `subagent`，**不得**出现 `origin_not_authorized`。

### C1.6 产品名登记条款（trace 证据不污染）

`model_visible_tool_name_opt()`（`crates/pony-agent-core/src/agent/tools.rs:7216-7252`）必须为 10 个团队工具
登记「**小写原名透传**」映射（与 `TOOL_SKILL_LOAD => TOOL_SKILL_LOAD`（`:7251`）**同构**），即
`spawn_teammate => "spawn_teammate"`、`subagent => "subagent"`、…、`team_task_list => "team_task_list"` 共 10 条。

**因果（为什么必须登记，而不是靠 `product_visible_tool_name` 的原样透传）**：
`model_visible_tool_name()`（`:7257-7261`）对未登记名回落字面量 `"Run"`（`unwrap_or("Run")`），而
`crates/pony-agent-core/src/agent/telemetry.rs` 的 tool activity 记录点（`:306`、`:332`、`:508`、`:576`）正是用
`model_visible_tool_name(&<tool>.name)` 生成 trace 里的 tool `name`；`product_canonical_tool_name`（`tools.rs:7271-7273`）
同源。若不登记，团队工具在 trace / 用户可见的副作用面里会被**记成 `Run`**，直接污染我们本波赖以定因的
`toolDefinitionsText` / tool-activity 证据链（即 §0 的取证面被污染）。

- **C1-A6**：`model_visible_tool_name("spawn_teammate") == "spawn_teammate"`（**不得**回落 `"Run"`）；
  对 10 个团队工具名逐一成立。
- **C1-A7**：`product_visible_tool_name` / `product_canonical_tool_name` 对 10 个名字返回原名（与 C1.2 模型面名一致），
  且 `model_visible_tool_name_opt` 的既有映射（含 `time_now => "Run"` 等）**不得**改动。

---

## C2 宿主引擎接线契约（注册点收敛到 core）

### C2.1 唯一注册点（由证据支持的 `path:line`）

**Lead 裁决**：注册点收敛到 core，**禁止**改 `src-tauri/src/lib.rs`（该文件属并发会话 write scope：
`git status` 显示 `src-tauri/src/lib.rs`、`tauri_adapter.rs` 等未提交改动，属 wave-60/61 streaming 工作）。

证据链（三条宿主全部经由同一个 core 漏斗 `AgentRuntimeBuilder::build()`）：

| 宿主 | bootstrap 证据 | 到漏斗的路径 |
|---|---|---|
| tauri（桌面） | `src-tauri/src/lib.rs:1339` `.manage(HostControlPlane::new())` | `HostControlPlane::new()`（`crates/pony-agent-core/src/agent/control_plane/mod.rs:1315-1317`）→ `DesktopHostPreset::build()`（`:1311-1313`）→ `HostControlPlaneBuilder::build()` → `mod.rs:1042` `AgentRuntimeBuilder::new().plan_store(...).build()` → **`crates/pony-agent-core/src/agent/runtime/builder.rs:79`** |
| non-tauri harness | `crates/pony-agent-core/src/bin/non_tauri_harness.rs:93-103` | `HostControlPlaneBuilder::runtime_builder(...)` → `control_plane/mod.rs:994-995` `runtime_builder.build()` → **`builder.rs:79`** |
| TUI | `crates/pony-agent-core/src/bin/tui.rs:2723-2731` | 同上（`mod.rs:994-995`）→ **`builder.rs:79`** |
| CLI（附带获益） | `crates/pony-agent-core/src/bin/cli.rs:394` | 直接 `AgentRuntimeBuilder::…build()` → **`builder.rs:79`** |

→ **唯一注册点＝ `crates/pony-agent-core/src/agent/runtime/builder.rs:79` `AgentRuntimeBuilder::build(self) -> AgentRuntime`**
（等价入口 `AgentRuntime::build()` → `builder.rs:123-125` 走同一函数）。

### C2.2 注册语义（硬约束）

1. **注册内容**：`Arc<dyn SubagentTurnEngine>` = `HostTurnEngine`（`crates/pony-agent-core/src/agent/subagent_runtime.rs:103-147`），
   并以 `with_workspace_root(<builder 的 workspace_root>)`（`:117-124`）携带父会话工作区根，满足 wave-58 F1-5
   （子代理必须以父根建栈：`tests/wave58_context_skills.rs:567-576`）。
   取值必须在 `builder.rs:100-102` 把 `workspace_root` move 进字符串**之前** clone。
2. **幂等（OnceLock）**：槽为 `static SUBAGENT_ENGINE: OnceLock<...>`（`subagent_runtime.rs:37`）；
   `set_subagent_engine` 重复注册返回 `Err("subagent engine already set")`（`:40-45`）。**首写者胜，后续注册静默 no-op**。
   → `build()` 中的注册必须写成 `let _ = set_subagent_engine(...)` 形态；**不得** `.expect()` / `unwrap()`。
3. **不得阻断宿主启动**：`build()` 签名返回 `AgentRuntime`（非 `Result`），注册失败（含已注册）**不得** panic、
   不得改变返回值、不得阻塞；只允许：忽略错误（首写者胜是该槽的既定语义）＋在既有诊断面记一条非致命日志
   （结构化日志属 PA-119 债务，本波**不新增**日志框架，`nfr-baseline.json` 的 `logging_compliance_claimed` 仍为 false）。
4. **公共 API 不变**：`set_subagent_engine` / `subagent_engine` 的签名与 `Err` 语义保持不变
   （`tests/wave37_tool_surface.rs:279-287` 依赖"注入后可读"、`tests/pa110_teams_acceptance.rs:28` 依赖"已注入时不 panic"）。
5. **不得在宿主侧新增注册调用**：`src-tauri/**`、`crates/pony-agent-core/src/bin/**` 中 `set_subagent_engine` 出现次数必须为 **0**。

### C2.3 「未接线时 fail-closed」保持不变（约束，不得放宽）

10 处门保持不变（`crates/pony-agent-core/src/agent/tools.rs:1571-1573`、`:1600-1602`、`:1629-1631`、`:1658-1660`、
`:1678-1680`、`:1707-1709`、`:1736-1738`、`:1765-1767`、`:1794-1796`、`:1823-1825`）：`!subagent_runtime_wired()`
→ `capability_unavailable_result(...)`（`:8633-8636`）。`orchestration.rs:244-252` 的结构化错误文案亦不变。
C2 只增加"生产路径上确实会接线"，**不删除**任何 fail-closed 分支。

### C2.4 已知限制（必须在契约内显式登记，不隐藏）

- `workspace_root` 由**首次** `build()` 决定并固化进全局槽；同进程内后续不同根的 `build()` 不覆盖。
  桌面单进程单宿主场景可接受；多根并发（测试、`workflow` 子运行时）下子代理沿用首个根。**[UNVERIFIED]**：
  未验证多宿主同进程（tauri + harness 同时）的根争用是否会改变行为——生产部署为单宿主，登记为限制。
- `HostTurnEngine::run_turn` 内部会自建 `AgentRuntimeBuilder::new()…build()`（`subagent_runtime.rs:168`），
  即嵌套 `build()` 会再次尝试注册 → 必为一律 no-op（同上第 2 条），不得因嵌套注册而报错或改变并发门
  （`SUBAGENT_MAX_CONCURRENT = 2`，`:59-85`）。

### C2.5 验收断言（可机器判定）

- **C2-A1**：`crates/pony-agent-core/src/agent/runtime/builder.rs:79-104` 函数体内存在对
  `subagent_runtime::set_subagent_engine` 的调用，且调用点包裹在"槽为空才写 / 忽略 Err"的语义中。
- **C2-A2**：`grep -c "set_subagent_engine" src-tauri/src crates/pony-agent-core/src/bin` == **0**（宿主零改动）。
- **C2-A3**：连续两次 `AgentRuntimeBuilder::new().build()` 不 panic；第二次注册返回 `Err` 且被忽略（无测试失败）。
- **C2-A4**：构建过 runtime 的进程内 `subagent_runtime_wired() == true`；从未构建过 runtime 的进程内为 `false`
  且 `subagent()` 返回 `capability_unavailable`（沿用 `tests/wave25_capability_honesty_test.rs:210`、`tests/orchestration_acceptance_test.rs:126-156`）。
- **C2-A5**：注册的引擎携带父根：`HostTurnEngine::with_workspace_root` 被 `builder.rs` 调用，且传入值为该 builder 的 `workspace_root`。

---

## C3 团队策略提示契约（仅当已接线时注入）

### C3.1 门控

注入条件**唯一**：`subagent_runtime_wired()`（`crates/pony-agent-core/src/agent/tools.rs:152`）为 `true`。
为 `false` 时**完全不注入**（不得注入空段、占位段、"不可用"提示），使未接线宿主的提示面逐字节不变。

### C3.2 文本静态性（DSH 对齐的硬约束）

- 策略段必须是 `&'static str` **常量全文**，**禁止**任何运行期插值（成员名、角色、路径、工具数量、
  `interrupt_agent` 是否可用等一律不进文本）。
- 证据：DSH `teamPolicy()` 是纯静态模板（`packages/experimental/tool-agent-team/src/index.ts:34-58`），
  以 `systemPrompt.section({ name: 'team:policy', order: getSectionOrder('TEAM_POLICY'), text: teamPolicy(...) })`
  注册进 **System 稳定段**（`:173-177`，`TEAM_POLICY: 600` 见
  `packages/core/system-prompt/src/index.ts:135`）；而变化事实（lead/teammate 身份与名字）走
  `systemPrompt.context({ name: 'team:identity', … })`（`:178-185`）——**身份类事实不进静态前缀**。
- Pony 侧对应约束：本波**只**注入静态策略段；**禁止**把成员名单/角色/任务板计数写进稳定前缀
  （这些应由工具调用结果或动态 Developer 层承载）。

### C3.3 落层位置（保持前缀缓存稳定与层序不变量）

- 注入为**紧跟在 `BASE_SYSTEM_PROMPT` 之后的第二条 `ProviderRole::System` 消息**。
  组装点：`crates/pony-agent-core/src/agent/context.rs:411`（`base_system_messages`）与 `:418-443`
  （`reserved_messages` 组装，`:441-445` 为 `prompt_layers_are_ordered` 的 `debug_assert!`）。
- **禁止改写 `BASE_SYSTEM_PROMPT` 本体**：`context.rs:2947` 断言 `base_system_prompt().len() == 3790`，
  改动常量会破坏既有冻结断言（抗熵底线：不做非因果修改）。
- 不变量必须保持：
  1. `prompt_layers_are_ordered(messages) == true`（`context.rs:52-73`）：System 全在 Developer 之前；
  2. `messages[首个 System].content.starts_with("You are Pony Agent")`（`:57-60`）—— 因 BASE 仍是首条，天然满足；
  3. 前缀稳定：门控值 `subagent_runtime_wired()` 由 `OnceLock`（`subagent_runtime.rs:37-45`）在 bootstrap 后恒定，
     故同一进程内策略段存在性**不随 turn 变化**，不产生 per-turn 前缀抖动。

### C3.4 段落内容要点（7 项，DSH 语义对齐，逐项可查）

策略段必须以标题行 `Agent Teams is available in this session` 开头（与 DSH `:36` 同一锚串，便于机器判定），并覆盖：

1. **共享工作区**：Lead 与所有队友共享同一工作目录与文件系统，编辑对彼此立即可见；写域必须切分为互不重叠的
   scope 并登记在共享任务上；write-scope 重叠只是 advisory，**不是锁**。
2. **任务板**：共享任务流程＝ list → get → claim（带当前 revision）→ 执行 → complete；任务就绪**不会**启动 owner。
3. **send_message 语义**：target 取自 `spawn_teammate` 或 `list_agents` 返回；running 目标在最近 step 边界收到，
   inactive 目标会被启动/恢复；消息以发送者名开头；**发送成功 ≠ 已被处理**。
4. **wait_agent 语义**：只观测调用**之后**的变更，永不唤醒成员；无活跃成员时立即 `noProgress`；醒来或超时后必须
   重新 list（`list_agents` / `team_task_list`），不得轮询。
5. **三态语义**：`inactive` 仅表示"当前没有 turn 在执行"，**不**代表任务完成/成功/失败；`provisioning` 表示成员创建中，
   `failed` 表示创建失败。
6. **成员创建**：需要写入/完成任务时优先 `spawn_teammate` 派生队友，用 `team_task_*` 维护共享任务板；
   `interrupt_agent` 用于中止某个队友的当前 turn。
7. **Lead 收尾纪律**：Lead 在给出最终答复前**必须**等待所有必需队友（`wait_agent` + 重新 list 任务板），
   不得在队友仍在 running/provisioning 时收尾。

> 文本允许措辞微调，但上述锚点（标题行 + 7 类语义关键词）是 C3-A1/C5-A1 的机器判定依据。

### C3.5 验收断言（可机器判定）

- **C3-A1**（已接线）：同一组装路径输出的消息序列中 `ProviderRole::System` 消息数 `== 2`，第 2 条以
  `Agent Teams is available in this session` 开头；`prompt_layers_are_ordered(&messages) == true`；
  `messages[0].content == base_system_prompt()`（逐字节）。
- **C3-A2**（未接线）：同路径 `System` 消息数 `== 1`，且全体消息文本中不存在字符串
  `Agent Teams is available in this session`。
- **C3-A3**（静态性）：策略段常量内不含任何 `{}`/`format!` 插值调用点（源码级判定：该常量定义处无宏展开参数）。
- **C3-A4**（前缀稳定）：同一进程内连续两个 turn 的策略段文本逐字节相等（长度与内容哈希一致）。

---

## C4 冻结测试演进门禁（6 个面，Test Agent 执行，Lead 一票冻结）

> **纪律**：冻结断言只能**收紧或等价改写**，**不得放宽**（禁止删除用例、禁止"只跑不报错"、禁止 `#[ignore]`、
> 禁止把断言换成 `println!`）。每条改写必须在本节留痕（旧断言 → 新断言 + 理由 + 真值源）。

### C4.1 面①：`crates/pony-agent-core/tests/tool_surface_wave20_contract.rs:4`

- 旧断言：`test_stub_tools_are_no_longer_model_visible`，`:8-17` 逐项 `assert!(!names.contains(&"<tool>"), "… must not be exposed to models")`。
- 新断言（**必须可见且可执行**，等价改写、方向反转）：
  - `assert!(names.contains(&"<tool>"), "<tool> must be exposed to models (wave-61 C1)")` 逐项覆盖同 10 个名字；
  - 追加"可执行"侧：`tool_exposure_for_name("<tool>") == ModelVisible`，且未接线时经 `ToolRouter` 调用返回
    `error.code == "capability_unavailable"`（承接原用例"不是假能力"的真正立意：**不再是 stub**）。
- 用例更名建议：`t1_team_tools_are_model_visible_and_executable`（名称可调，断言集合不得缩减）。
- 理由与真值源：`tools.rs:7341` / `:7381`（本次目标）＋ `tools.rs:1571-1573`（fail-closed 门仍在）。
- 保留不动：同文件 `:20-31` `test_goal_tools_are_canonical_and_model_visible`（不受本波影响）。

### C4.2 面②：`crates/pony-agent-core/tests/wave37_tool_surface.rs:81`

- 旧断言：`t1_model_visible_surface_is_exact_29_tool_list`，`:85-88` `names.len() == 29`，`:91-125` 29 项逐字清单。
- 新断言：`names.len() == 39`，`expected` 逐字替换为 C1.2 的 39 行清单（含顺序）；用例更名
  `t1_model_visible_surface_is_exact_39_tool_list`；`:127-200` 的 kind/exposure/permission 表检查覆盖新增 10 项
  （`subagent`/`workflow` → `composite`；`spawn_teammate`/`list_agents`/`send_message`/`interrupt_agent`/`drain_inbox`
  → `interactive`；`team_task_create`/`team_task_update` → `write`；`team_task_list` → `read`；
  kind 真值源 `tools.rs:7297-7304`）。
- 理由与真值源：`tools.rs:7341`（exposure）＋ `tools.rs:5925-6138`（定义序）＋ `:555-566`（槽位排序）。

### C4.3 面③（本波新发现，必须同波迁移）：`crates/pony-agent-core/tests/wave25_capability_honesty_test.rs`

- `:124` `t1_fake_capability_tools_are_absent_from_model_visible_surface`：`:151-163` 双向集合相等（`EXPECTED_MODEL_VISIBLE_TOOLS`
  `:52-83` 29 项）＋ `:165-180` "10 个名字逐个不得出现"。
  → 迁移：`EXPECTED_MODEL_VISIBLE_TOOLS` 扩为 39 项（与 C1.2 逐字一致）；"逐个不得出现"反转为"逐个必须出现"；
  重命名 `FAKE_CAPABILITY_TOOLS` → `TEAM_TOOLS`（保留常量内容与 10 项顺序）。
- `:400` `t4_fake_capability_descriptors_are_not_model_visible`：断言 exposure ∉ {Internal, Deferred}。
  → 迁移为断言 exposure **==** `ModelVisible`（方向反转，强度不变）。
- `:210` `t2_fake_capability_tools_return_capability_unavailable_error`：**保持不变**——它是 C2.3「未接线 fail-closed」的
  锚点用例（该文件不构建 runtime，进程内槽为空，语义仍成立）。改名可，断言不得改。
- 理由与真值源：C1（可见性）＋ C2.3（可执行性/未接线）。

### C4.4 面④（本波新发现，含并发写域风险）：`crates/pony-agent-core/src/agent/provider/mod.rs`

- `:5140` `openai_tools_payload_uses_product_tool_surface_for_builtin_tools` 与
  `:5219` `anthropic_tools_payload_uses_product_tool_surface_for_builtin_tools`：`assert_eq!` 逐字 29 项数组
  （`:5153-5182`、`:5231-5260`）。
  → 迁移：两处数组同步扩为 39 项（插入位置同 C1.2：`ViewImage` 之后、`TodoWrite` 之前）。
- `:5195` `fallback_tool_payload_filters_internal_builtin_but_keeps_dynamic_tools`：以 `subagent` 作为"Internal builtin
  必须被 fallback 过滤"的样例。C1 后 `is_internal_builtin_tool_name("subagent")`（`:3091-3095`）将返回 false，
  该断言必然失败。**实测补充**：`tool_exposure_for_name()` 的可见/延迟分支已覆盖 `builtin_tools()` 的其余全部名字
  （`tools.rs:7341-7381`），故 `subagent` 等 10 项是当前**唯一**的函数级 `Internal` 样例，C1 之后该集合为空
  —— 不存在"换一个 Internal builtin 当样例"的迁移路径，必须按下面的规则改写。
  → 迁移规则（**不得删用例、不得放宽**）：改为对 `builtin_tools()` 全部名称求
  `{ n | is_internal_builtin_tool_name(n) }` 并断言其**恰为空集**（C1 的直接推论：模型面不再依赖 fallback 摘除任何
  builtin），同时保留"动态工具 `mcp__demo__echo` 必须透传"的原有断言。该写法比原用例更严（若未来有人把某个 builtin
  重新标成 Internal，用例会红）。
- **并发写域风险（Lead 必读）**：该文件当前已被另一并发会话改动（`git status` 中
  `M crates/pony-agent-core/src/agent/provider/mod.rs`，属 wave-60/61 streaming 工作）。Test Agent 迁移以上三处时
  **必须**与 Lead 确认时序，避免 hunk 冲突；本契约只登记规则，不主张写入时机。

### C4.5 面⑤：`crates/pony-agent-core/src/agent/tools.rs:6611`（src 内 characterization 用例）

- 用例：`characterization_product_surface_keeps_unique_names_and_json_object_schemas`
  （`assert_eq!(tools.len(), 45)` 于 `:6621-6625`；`assert_eq!(names.len(), 29)` 于 `:6626-6630`；
  29 项逐字清单于 `:6631-6663`）。
- 新断言：`names.len() == 39` 且 29 项清单替换为 C1.2 的 39 行（团队工具插在 `ViewImage` 之后、`TodoWrite` 之前）；
  `tools.len() == 45`（**内部注册数不变**，C1 只改 exposure，不增定义）+ `input_schema` 的 object/properties 校验保留。

### C4.6 面⑥：`crates/pony-agent-core/src/agent/tools.rs:6802`（src 内 registry 投影用例）

- 用例：`builtin_registry_projects_the_existing_product_surface`（`assert_eq!(names.len(), 29)` 于 `:6812-6816`）。
- 新断言：`names.len() == 39`；`registry.resolve("subagent") / resolve("team_task_list")` 的
  `exposure == ModelVisible`（逐个 10 项）；既有 `resolve("ToolSearch") == Deferred`、
  `snapshot_id == "builtin-tool-catalog-v1"` 断言**不得**改动。

### C4.7 面⑤/⑥ 的执行方式（编译衍生同步豁免）

面⑤/⑥ 与被迁移的生产代码同处 `crates/pony-agent-core/src/agent/tools.rs`（同一 crate、同一编译单元），
**不单独出红相**（无法在 C1 实现落地前产生"仅这两处失败"的独立红相），故按
**「编译衍生同步豁免 + Lead 复核 diff」** 处置：

1. Test Agent 在 C1 实现同一次提交内完成两处断言同步（29 → 39、清单逐字）；
2. 豁免仅限于"无独立红相"这一点，**不放宽断言**（数量与逐字清单都改严/等价，不删用例、不改 `#[ignore]`）；
3. Lead 必须逐行复核这两处 diff，确认旧断言可一一映射到新断言（C4-A3）；
4. 若两处未同步，`cargo test -p pony-agent-core --lib` 必红（C4-A2），视同门禁失败。

### C4.8 门禁

- **C4-A1**：迁移后 `cargo test -p pony-agent-core --test tool_surface_wave20_contract --test wave37_tool_surface --test wave25_capability_honesty_test`
  全绿（机器 Exit Code 0）。
- **C4-A2**：迁移后 `cargo test -p pony-agent-core --lib` 全绿（面④/⑤/⑥）。
- **C4-A3**：`git diff` 中上述 6 个面的断言数量只增不减（人工/脚本核对：旧断言逐条可映射到新断言）。
- **C4-A4**：冻结由 Lead 一票决定：本文件状态已是 `FROZEN v1.1`（@069587a），测试断言任何后续改动必须走新契约（本文件的 successor）。

---

## C5 同源门控对齐（DSH `install()` 同源注册的 Pony 等价）

DSH 中团队工具注册与策略段由**同一个** `install()` 同步注册（`packages/experimental/tool-agent-team/src/index.ts:168-183`），
因此"工具可用 ⇔ 策略存在"是结构保证。Pony 侧两者由同一判定 `subagent_runtime_wired()` 派生，但**分工不同**（见 C1.4 结论）：

- 工具**可见性**：静态（不门控）—— 保证 provider 工具数组逐字节稳定；
- 工具**可执行性** + **策略段**：同一门 `subagent_runtime_wired()`。

### C5.1 验收断言（可机器判定）

- **C5-A1**（Lead 指定）：`subagent_runtime_wired() == true` 时，`builtin_turn_tool_contract_views()` 名字集合
  **包含**全部 10 项团队工具 **且** 组装后的 System 稳定层文本包含 `Agent Teams is available in this session`。
- **C5-A2**（反向边界）：`subagent_runtime_wired() == false` 时，工具面仍含 10 项（39 项不变），
  策略段**缺席**，且经 `ToolRouter` 调团队工具返回 `capability_unavailable`——三者必须同时成立。
- **C5-A3**（同源证据）：C3 注入点与 `tools.rs:1571-1573` 等 10 处 fail-closed 门使用**同一个**判定函数
  `subagent_runtime_wired()`（源码级：`grep -n "subagent_runtime_wired()"` 覆盖 11 处，无第二套接线判定）。

---

## 6. NFR 基准增量（`.dev-team/nfr-baseline.json`，additive-only）

追加 3 个顶层字段（不改动任何既有字段；同步在 `.dev-team/nfr-baseline.schema.json` 中**增量**声明为可选属性，
不进入顶层 `required`，不关闭 `additionalProperties: false` 校验）：

| 字段 | 值 | 语义 | 门禁命令 |
|---|---|---|---|
| `team_tool_surface_exact_count` | `39` | 模型可见工具面精确项数（C1.2/C1-A1） | 见 §7 |
| `subagent_engine_registration_required_in` | `["tauri","harness","tui"]` | 必须**经由 core 唯一注册点**获得引擎的三条宿主（C2.1） | 见 §7 |
| `team_policy_prompt_injected_when_wired` | `true` | 已接线时必须注入静态策略段（C3.1/C3-A1） | 见 §7 |

> 说明：`subagent_engine_registration_required_in` 的语义是"这三条宿主必须**受益于**注册"，实现方式是
> 收敛在 `builder.rs:79`，**不是**要求这三处的源码被修改（C2-A2 反向约束）。

### 6.1 既有 schema 漂移存档（**HEAD 已存在，本波不修**）

严格门禁 `python3 .dev-team/nfr-lint.py`（`additionalProperties: false` + 未知字段拒绝）在 **HEAD 上已经红**。
基线口径：以 **c575243**（wave-61 提交前的 HEAD）导出 instance+schema 后重跑，实测 **6 条错误**（`PRE_CHANGE_EXIT=1`），
错误项为 `$.observability`、`$.external_call_timeout_exceptions`、`$.logging_eprintln_exempt_note`、
`$.logging.trace_id` 四个"instance 有、schema 未登记"的漂移（由此前波次只改 instance 未改 schema 造成）。

**Lead 裁决（wave-61）**：该漂移与「团队工具面 / 宿主接线 / 策略提示」无因果关系，属**既有**缺陷；
按抗熵底线**禁止夹带修复进本波提交**。Protocol Agent 已按此裁决**回退**全部补登记动作
（`observability` / `external_call_timeout_exceptions` / `logging_eprintln_exempt_note` / `logging.trace_id` 四处均已移出 schema）。

→ **登记为 dev-backlog 候选**（范围：`.dev-team/nfr-baseline.schema.json` 三处顶层既有字段登记
`observability` / `external_call_timeout_exceptions` / `logging_eprintln_exempt_note`，外加一处嵌套字段
`logging.trace_id`；纯增量声明即可，不得删除或改写既有取值）。
→ 本波 `nfr-baseline.schema.json` **只**新增 wave-61 自己的 3 个字段声明；严格门在回退后与 HEAD **同为失败**
（exit code 见 §7，必须如实回报，不得为变绿而改 schema）。

## 7. 门禁与收据命令

```bash
# NFR 门禁（技能脚本，必须 Exit 0）
python3 /home/dm/.agents/skills/dev-team/scripts/dev-nfr-lint.py .dev-team/nfr-baseline.json

# 仓库严格门禁（jsonschema + additionalProperties:false）
# 预期：非 0（HEAD 既有漂移，见 §6.1）；正确调用形式无位置参数（该脚本只接受 --instance/--schema）
python3 .dev-team/nfr-lint.py

# 写域收据（只允许 .dev-team/ 下文件变化）
git status --porcelain -- .dev-team/
```

## 8. 未验证项（[UNVERIFIED]，禁止据此放行）

1. **[UNVERIFIED]** trace 中 28 项 `toolDefinitionsText` 与当前 29 项基线之间的构建指纹（推测为 `skill_load` 落地前的构建）。
2. **[UNVERIFIED]** 注册点上移后，`cargo test -p pony-agent-core` 全量是否存在**测试执行顺序耦合**
   （构建过 runtime 的单测进程会把槽置为已接线，从而使"未接线"断言在**同进程**内不可达）。已核实
   `context.rs` 内无消息条数精确断言（仅 `:2947` 的 `base_system_prompt()` 字节长度），故 C3 风险较低；
   但全量 cargo test 尚未在本会话运行。
3. **[UNVERIFIED]** 10 项团队工具作为 `ModelVisible` 进入 provider payload 后，各 provider 侧是否存在工具数量/字节上限
   截断（本会话未测真实 provider 请求体积）。
4. **[UNVERIFIED]** 多宿主同进程（tauri + harness 并存）时 `workspace_root` 首写者语义的实际影响（见 C2.4）。

---

## 附：本契约的写入边界声明

本契约由 Protocol Agent 写入，**只**新增/修改 `.dev-team/contracts/wave-61-team-activation.md`、
`.dev-team/nfr-baseline.json`（追加 3 字段）、`.dev-team/nfr-baseline.schema.json`（**只**追加 wave-61 自己的
3 个字段声明；按 Lead 裁决已**回退**所有既有字段补登记动作，漂移仅存档于 §6.1 并登记为 dev-backlog 候选）。
未触碰任何 Rust 源码、测试、`.dev-team/state.json`。
