# W2 RED 收据（task-7）

- 角色：独立 Test Agent（只写测试，不改生产逻辑，不做 git 写）
- 工作区：`/home/dm/pony-agent`
- 接口桩基线：`d169a287 chore(protocol): freeze w2 provider-inheritance interfaces`
- 变更范围：**仅新增** `crates/pony-agent-core/tests/w2_provider_inheritance_test.rs`（未修改任何已有文件）
  - `git status --porcelain` 唯一相关条目：`?? crates/pony-agent-core/tests/w2_provider_inheritance_test.rs`
- 未执行 git commit。
- 确定性：不使用全局 `SUBAGENT_ENGINE` OnceLock、不联网、不读写外部配置；每用例只构造本地值。

## 1. 命令与退出码（物理凭证）

```
$ cargo test -p pony-agent-core --test w2_provider_inheritance_test -- --nocapture
```

退出码：**101**（失败 = 预期 RED）

```
   Compiling pony-agent-core v0.1.154 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `test` profile [unoptimized] target(s) in 38.76s
     Running tests/w2_provider_inheritance_test.rs (target/debug/deps/w2_provider_inheritance_test-85b1140749a4b82b)

running 7 tests

thread 'c1_inherits_host_selection_when_no_override' (1551127) panicked at crates/pony-agent-core/tests/w2_provider_inheritance_test.rs:66:21:
C1：无显式覆盖时必须继承宿主选择，实际 Err = not_implemented

thread 'c2_overrides_model_without_changing_provider' (1551128) panicked at crates/pony-agent-core/tests/w2_provider_inheritance_test.rs:91:21:
C2：显式 model 覆盖必须解析成功，实际 Err = not_implemented

[C3] err = not_implemented
thread 'c3_missing_host_selection_is_rejected_with_stable_code' (1551129) panicked at crates/pony-agent-core/tests/w2_provider_inheritance_test.rs:118:13:
C3：错误信息必须包含 `missing_host_provider_selection`，实际 = not_implemented

[C4] err = not_implemented
thread 'c4_unknown_provider_override_is_rejected_with_stable_code' (1551130) panicked at crates/pony-agent-core/tests/w2_provider_inheritance_test.rs:144:13:
C4：错误信息必须包含 `unknown_provider`，实际 = not_implemented

[C6] observed = requested_name=w2-host-requested provider_name=w2-host-provider model=w2-host-model base_url=https://host.invalid/v1
[C6] injected = requested_name=w2-host-requested provider_name=w2-host-provider model=w2-host-model base_url=https://host.invalid/v1
[C5] with_overrides provider_id=Some("w2-provider-x") model_id=Some("w2-model-y")
[C5] without_overrides provider_id=None model_id=None
[C7] run_turn_calls=0 run_turn_in_calls=1 final_text=run_turn_in session_id=run_turn_in-session

test c6_provider_manager_selection_round_trips_injected_selection ... ok
test c7_default_run_turn_with_selection_delegates_to_run_turn_in_once ... ok
test c5_spawn_teammate_args_serde_carries_provider_overrides ... ok
test c1_inherits_host_selection_when_no_override ... FAILED
test c2_overrides_model_without_changing_provider ... FAILED
test c3_missing_host_selection_is_rejected_with_stable_code ... FAILED
test c4_unknown_provider_override_is_rejected_with_stable_code ... FAILED

failures:
    c1_inherits_host_selection_when_no_override
    c2_overrides_model_without_changing_provider
    c3_missing_host_selection_is_rejected_with_stable_code
    c4_unknown_provider_override_is_rejected_with_stable_code

test result: FAILED. 3 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## 2. 用例清单（行号 / 契约 / 期望 / 实测）

| 契约 | 测试函数 | 文件:行号 | 期望（冻结契约） | 实测（桩基线 d169a287） |
| --- | --- | --- | --- | --- |
| C1 | `c1_inherits_host_selection_when_no_override` | `tests/w2_provider_inheritance_test.rs:59` | `Ok`，provider_name/requested_name/model == host | **FAILED**（Err `not_implemented`） |
| C2 | `c2_overrides_model_without_changing_provider` | `…:84` | `Ok`，model==`custom-model-xyz`，provider 不变 | **FAILED**（Err `not_implemented`） |
| C3 | `c3_missing_host_selection_is_rejected_with_stable_code` | `…:108` | `Err` 含 `missing_host_provider_selection` | **FAILED**（Err `not_implemented`，不含该码） |
| C4 | `c4_unknown_provider_override_is_rejected_with_stable_code` | `…:128` | `Err` 含 `unknown_provider` | **FAILED**（Err `not_implemented`，不含该码） |
| C5 | `c5_spawn_teammate_args_serde_carries_provider_overrides` | `…:154` | provider_id/model_id 解析成功；缺省 None；None 不落盘 | PASS |
| C6 | `c6_provider_manager_selection_round_trips_injected_selection` | `…:215` | `selection()` 原样返回注入值 | PASS |
| C7 | `c7_default_run_turn_with_selection_delegates_to_run_turn_in_once` | `…:309` | 默认实现委托 `run_turn_in` 恰好一次 | PASS |

辅助：`host_selection()` `:25`（本地构造宿主选择，不触网）、`describe()` `:47`、`CountingEngine`（只覆写 `run_turn_in`，计数 `run_turn`/`run_turn_in` 各 1 个 `AtomicUsize`）。

## 3. RED / GREEN 判定

- **RED 成立（C1–C4）**：四处都在 `resolve_teammate_provider_selection` 的桩返回值 `Err("not_implemented")` 上失败，属接口未实现而非断言错误。
- **当前即通过（C5–C7，作为不可回归保护）**：
  - C5：`SpawnTeammateArgs` 的 `#[serde(default, skip_serializing_if = "Option::is_none")]` 已冻结；断言双向（解析 None、序列化不写键）。
  - C6：`ProviderManager::selection()` 直接返回 `&self.config`；9 个字段逐一比对（`ResolvedProviderSelection` 未实现 `PartialEq`，且本任务禁止新增 `pub` API，故用字段级断言）。
  - C7：`CountingEngine` 只覆写 `run_turn_in`；`run_turn_with_selection(..., None)` 实测 `run_turn_in_calls=1`、`run_turn_calls=0`、`workspace_root=None` 原样透传、返回值出自 `run_turn_in`。

## 4. 未覆盖 / 边界说明（供 GREEN 与验收参考）

1. 未覆盖 `subagent_in_workspace_with_provider` / `spawn_teammate_in_workspace_with_provider` 两个桩入口（不在 task-7 契约范围内；若 W2 GREEN 要接线，需另派测试）。
2. C6 未做整体对象相等断言（`ResolvedProviderSelection` 无 `PartialEq`；为守「不新增 pub API」红线，改用字段级等价）。
3. C7 只验证 `selection=None` 的默认委托；「引擎真正按 selection 切 provider」属 GREEN 实现行为，需另行集成用例（且会触网/需 provider 注册表，本 RED 明确排除）。
4. 未涉及全局 `SUBAGENT_ENGINE` OnceLock 装配路径（显式排除，保证确定性与无全局态污染）。
5. C1/C2 仅断言契约冻结字段（provider_name / requested_name / model），未额外约束 base_url、protocol、api_key 的继承细节，避免过度约束实现。
