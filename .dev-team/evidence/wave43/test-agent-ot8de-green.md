# task-6 · OT-8d/8e 绿相收据（Test Agent）

日期：2026-10-24 17:23 · 触发：executor-core 落盘 `lib.rs` 桥接（17:21:11）后复跑。

## 命令
`cargo test --manifest-path src-tauri/Cargo.toml -p pony-agent-core --test otel_trace_migration_test --target-dir target-test-exact-a`

## 结果
- `test result: FAILED. 19 passed; 2 failed`，Exit 101。
- 通过：`ot8_settings_trace_flag_defaults_false` / `..._roundtrips_with_camel_case_key` /
  `..._legacy_settings_without_trace_flag_reads_false` / **`ot8_exit_path_releases_otel_provider`** /
  **`ot8_settings_bridge_syncs_core_gate_on_load_and_save`** / `ot6_force_export_bypasses_closed_gate_and_keeps_gate_state`。
- 剩余 2 红：`ot7_retired_symbols_have_zero_hits_repo_wide`、`ot7_retired_ponysentry_trace_symbols_have_zero_hits`
  —— 均为 D6 退役未完成（executor-core 侧 B3 尚未删除 ponysentry trace 实现 + task-4 尚未删除 3 个存量测试）。
- 原始输出：`.dev-team/evidence/wave43/green-ot8de-1726.log`

## 被断言的真实锚点（非自证）
| 断言 | 锚点 |
| :--- | :--- |
| 8d core API | `crates/pony-agent-core/src/agent/otel/mod.rs:332 pub fn set_agent_trace_reporting` → 转调 `set_reporting_enabled` |
| 8d 加载路径 | `src-tauri/src/lib.rs:588` `set_agent_trace_reporting(settings.agent_trace_reporting)` |
| 8d 保存路径 | `src-tauri/src/lib.rs:596` `set_agent_trace_reporting(saved.agent_trace_reporting)`（以落盘回显值为准） |
| 8e 启动装配 | `src-tauri/src/lib.rs:1236` `otel::init(OtelConfig::from_env())` |
| 8e 退出释放 | `src-tauri/src/lib.rs:1348-1349` `if let tauri::RunEvent::Exit = event { otel::shutdown(); }` |

## 前端契约同步复核
`npx vitest run tests/TelemetryPage.spec.ts tests/settings.store.spec.ts tests/otel-reporting-switch.spec.ts`
→ `Test Files 3 passed (3)` / `Tests 14 passed (14)`，Exit 0。

## 遗留（非本任务写域）
- OT-6 曾出现 `导出路径必须为 /v1/traces，实际 /`：属 executor 编辑中间态（17:20 前），本轮已转绿，不作缺陷登记；
  若后续再次出现，按真实缺陷上报。
- OT-7 收紧条件：executor-core 删除 D6 源符号 + task-4 删除 `PENDING_TASK4_CLEANUP_FILES` 三个文件后，
  自动变为全仓 0 命中（无需改测试）。
