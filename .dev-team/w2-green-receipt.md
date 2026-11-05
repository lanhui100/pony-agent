# W2 GREEN 收据（task-8）

- 角色：Executor（实现；不改冻结测试 W1/W2，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 接口桩基线：`d169a287`；RED 冻结：`dfd07551`；W1 已提交：`3e7e7bec`
- 变更范围（6 个生产文件 / +311 / −14，**未改任何测试文件、未 git add/commit**）：
  - `crates/pony-agent-core/src/agent/orchestration.rs`
  - `crates/pony-agent-core/src/agent/subagent_runtime.rs`
  - `crates/pony-agent-core/src/agent/tools.rs`
  - `crates/pony-agent-core/src/agent/runtime/batch_plan.rs`
  - `crates/pony-agent-core/src/agent/runtime/turn_sync.rs`
  - `crates/pony-agent-core/src/agent/runtime/turn_stream.rs`
- 未改动 `provider/mod.rs`：`ProviderManager::selection()` 已由桩提交 `d169a287` 提供（只读借用），本波次无需再动。

## 1. 命令与退出码（物理凭证）

### 1.1 W2 主门禁

```
$ cargo test -p pony-agent-core --test w2_provider_inheritance_test -- --nocapture
```

退出码：**0**

```
running 7 tests
[C1] resolved  = requested_name=w2-host-requested provider_name=w2-host-provider model=w2-host-model base_url=https://host.invalid/v1
[C2] resolved  = requested_name=w2-host-requested provider_name=w2-host-provider model=custom-model-xyz base_url=https://host.invalid/v1
[C3] err = missing_host_provider_selection: 未注入宿主 provider 选择，且未显式指定 provider
[C4] err = unknown_provider: provider `definitely-no-such-provider` 不在本机 provider 注册表中
[C6] observed = requested_name=w2-host-requested provider_name=w2-host-provider model=w2-host-model base_url=https://host.invalid/v1
[C7] run_turn_calls=0 run_turn_in_calls=1 final_text=run_turn_in session_id=run_turn_in-session
test c1_inherits_host_selection_when_no_override ... ok
test c2_overrides_model_without_changing_provider ... ok
test c3_missing_host_selection_is_rejected_with_stable_code ... ok
test c4_unknown_provider_override_is_rejected_with_stable_code ... ok
test c5_spawn_teammate_args_serde_carries_provider_overrides ... ok
test c6_provider_manager_selection_round_trips_injected_selection ... ok
test c7_default_run_turn_with_selection_delegates_to_run_turn_in_once ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 1.2 编译门禁

```
$ cargo check -p pony-agent-core
    Finished `dev` profile [unoptimized] target(s) in 11.12s
```

退出码：**0**

### 1.3 回归门禁（不跑全量）

| 命令 | 通过数 | 退出码 |
| --- | --- | --- |
| `cargo test -p pony-agent-core --lib agent::planner` | 33 passed | 0 |
| `cargo test -p pony-agent-core --lib agent::runtime::stream_support` | 8 passed | 0 |
| `cargo test -p pony-agent-core --lib agent::runtime::batch_plan` | 3 passed | 0 |
| `cargo test -p pony-agent-core --test orchestration_acceptance_test` | 4 passed | 0 |
| `cargo test -p pony-agent-core --test pa110_teams_acceptance` | 5 passed | 0 |
| `cargo test -p pony-agent-core --test teammate_e2e_communication_test` | 1 passed | 0 |
| `cargo test -p pony-agent-core --test wave61_team_activation` | 16 passed | 0 |
| `cargo test -p pony-agent-core --test team_task_order_contract` | 1 passed | 0 |
| `cargo test -p pony-agent-core --test tool_surface_wave20_contract` | 2 passed | 0 |
| `cargo test -p pony-agent-core --test ask_pending_contract` | 6 passed | 0 |

`agent::orchestration` / `agent::subagent_runtime` 在 lib 内无单测（0 filtered），故用上述集成目标覆盖。

## 2. 契约 → 实现 file:line

| 契约 | 实现点 |
| --- | --- |
| 解析语义（C1–C4） | `orchestration.rs:490` `resolve_teammate_provider_selection`：`(host, None)` ⇒ host 缺失 `Err("missing_host_provider_selection…")`，存在则 clone；显式 `provider_id` 命中 `host.requested_name`/`host.provider_name` ⇒ host clone；否则 `ProviderRegistryStore::new().load_view().providers` 按 id/name 查找，未命中 `Err("unknown_provider…")`，命中取其 `id` 交给 `resolve_selection(Some(id), model_id)`；末尾统一用 `model_id` 覆盖 `selection.model`（含宿主分支） |
| legacy 容忍（None） | `orchestration.rs:531` `resolve_optional_teammate_provider_selection`：`host=None` 且 `args` 无覆盖 ⇒ `Ok(None)`（旧行为）；否则必须解析成功 ⇒ `Err` 前置失败。**公共 `resolve_teammate_provider_selection` 本身仍严格遵守 C3**（`None,None,None` ⇒ Err） |
| 队友派生入口 | `orchestration.rs:587` `spawn_teammate_in_workspace_with_provider`：先解析（失败即返回，**不登记/不投递/不起线程**）→ `register_teammate` → `set_teammate_provider(target, selection)`（`orchestration.rs:362`，存入 `TEAMMATE_PROVIDERS`，`orchestration.rs:358`）→ 复用既有 `send_message_in_workspace`（入队 + 后台自动驱动） |
| 旧入口兼容 | `orchestration.rs:573` `spawn_teammate_in_workspace` = `_with_provider(args, root, None)`；`subagent_in_workspace` 同理委托 |
| 子代理入口 | `orchestration.rs:409` `subagent_in_workspace_with_provider`：先解析 → 许可门（自持引擎跳过）→ 同步/后台均调用 `engine.run_turn_with_selection(..., selection.as_ref())`（后台线程 move 所有权） |
| 驱动消费 | `orchestration.rs:1098` `drive_teammate_turn_in`：`teammate_provider_selection(target)`（`orchestration.rs:373`）→ `engine.run_turn_with_selection(..., selection.as_ref())`；无登记 ⇒ `None` ⇒ 引擎旧链 |
| 引擎覆写 | `subagent_runtime.rs:379` `HostTurnEngine::run_turn_with_selection`：`None` ⇒ 逐字回落 `run_turn_in`；`Some` ⇒ 解析根（显式 root 优先，否则 `resolved_workspace_root()`）→ `run_turn_with_root_and_selection`（`subagent_runtime.rs:237`）→ `AgentRuntimeBuilder::new().workspace_root(root).provider_resolver(Box::new(FixedSelectionResolver::new(sel)))`，`TurnInput.provider_id/model_id = None` |
| 固定选择器 | `subagent_runtime.rs:29` `FixedSelectionResolver` + `:39` `impl ProviderSelectionResolver`（`resolve_provider_selection` 忽略入参、clone 快照） |
| 上下文接线 | `batch_plan.rs:194` `execute_tool_call_with_turn_context(..., provider_selection: Option<&ResolvedProviderSelection>)`，`:205` 写入 `ToolExecutionContext { provider_selection: provider_selection.cloned(), ..Default::default() }` |
| 宿主选择下传 | `turn_sync.rs:323/354`、`turn_stream.rs:777/809`：`Some(provider.selection())`（`&ProviderManager` 只读借用，无 clone） |
| 工具入口 | `tools.rs:1616` `execute_subagent` → `subagent_in_workspace_with_provider(args, root, context.provider_selection.as_ref())`；`tools.rs:1690` `execute_spawn_teammate` → `spawn_teammate_in_workspace_with_provider(...)`；错误分别落既有 `subagent_failed` / `spawn_failed`（ok 前失败） |
| 接口导出 | `tools.rs:59-60` 追加 `pub use` 两个新入口 |

## 3. 行为要点

1. **失败前置**：provider/model 解析在 `register_teammate` 与后台线程之前完成；`missing_host_provider_selection` / `unknown_provider` 直接以 `spawn_failed` / `subagent_failed` 返回，绝不「先 ok 后异步失败」。
2. **继承**：工具路径总会传 `Some(provider.selection())`（= 本 turn `ProviderManager` 的解析结果），队友/子代理因此继承宿主 provider/model/密钥/推理档位。
3. **覆盖**：`args.provider_id`/`args.model_id`（工具 JSON）→ 解析为独立快照；只给 `model_id` 时 provider 不变（C2）。
4. **驱动**：同名 target 再派生会覆盖登记；无登记（legacy 库调用 / 无引擎路径）保持 `run_turn_in`。
5. **Stub 引擎零破坏**：`run_turn_with_selection` 的 trait 默认实现仍是委托 `run_turn_in`，所有既有 Stub/宿主自定义引擎行为不变（C7 通过）。

## 4. 说明 / 未覆盖 / 风险

1. **未改动 `provider/mod.rs`**（writeScopes 列了它，但 `selection()` 已在桩提交提供）；**无需机械补 `ToolExecutionContext` 字面量**——`batch_plan.rs` 原有 `..Default::default()` 自动补齐，本波次只显式写入新字段。
2. 未跑全量 `cargo test`（红线）；`agent::runtime::tests`（>1200 例）与 TUI 相关目标未跑，仅按门禁要求跑 W2 + 相关 wave61/team/orchestration 目标 + W1 两个过滤。
3. `resolve_teammate_provider_selection` 的注册表查找**不区分协议/模型是否存在**：`model_id` 在该 provider 下不存在时由 `ProviderRegistryStore::resolve_selection` 既有规则回落（selected/first model）后再被 `model_id` 覆盖为字符串本身——即用户显式 model 会原样生效（与宿主分支一致），不额外校验模型可用性。
4. 显式 `provider_id` 与宿主匹配是**字符串精确匹配**（未 trim 宿主侧名字、未做大小写折叠）；空串/空白按「未提供」处理。
5. `TEAMMATE_PROVIDERS` 是进程内表，无逐条清理（target 同名覆盖）；`subagent` 每次派生独立解析、不落全局表。
6. `HostTurnEngine` 用固定选择器时，子代理栈的 `ProviderRegistryStore`（工具/模型目录等）仍与宿主隔离，仅 provider 选择被固定——这是既定架构（子代理独立建栈），未改变。
