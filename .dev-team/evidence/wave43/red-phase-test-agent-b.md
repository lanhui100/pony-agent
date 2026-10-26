# Wave43 红相收据 · Test Agent（接棒者）

## Deliverables
- `crates/pony-agent-core/tests/otel_trace_migration_test.rs`（18 用例）
- `tests/otel-reporting-switch.spec.ts`（4 用例）
- 日志：`red-rust-otel_trace_migration_test.log` / `red-vitest-otel-reporting-switch.log`

## Command A（Lead 指定原样命令）
`cargo test --manifest-path src-tauri/Cargo.toml --test otel_trace_migration_test --target-dir target-test-exact-a`
→ Exit 101，**未进入编译/运行**：
```
error: no test target named `otel_trace_migration_test` in default-run packages
help: available test in `pony-agent-core` package:
    otel_trace_migration_test
```
（根 `Cargo.toml` 是虚拟 workspace，`--manifest-path src-tauri/Cargo.toml` 默认选中 `pony-agent` 包；集成测试在 `pony-agent-core` 包内，必须加 `-p pony-agent-core`。）

## Command B（最小修正：仅补包选择器，其余原样）
`cargo test --manifest-path src-tauri/Cargo.toml -p pony-agent-core --test otel_trace_migration_test --target-dir target-test-exact-a`
→ Exit 101（红相成立）：
```
test result: FAILED. 6 passed; 12 failed; 0 ignored; 0 measured; 0 filtered out
error: test failed, to rerun pass `-p pony-agent-core --test otel_trace_migration_test`
```
失败原语：`unimplemented!("Wave43 空桩：...")` panic（otel::*）+ OT-7 真实残留 12 个退役符号。
已绿 6 条：`attr_value_variants_*` / `frozen_span_names_attribute_keys_*` / OT-8a/b/c（app_settings 已实现）/ `ot7_issue_reporting_keep_surface_still_exists`。

## Command C（前端）
`npx vitest run tests/otel-reporting-switch.spec.ts` → Exit 1，4/4 失败：
- UI-1 默认关闭：`Unable to get [data-testid="config-agent-trace-reporting-switch"]`
- UI-1 落盘：同上（开关不存在）
- UI-1 normalize 透传：`expected { workspaceMode: 'coding' } to match object { agentTraceReporting: true }`
- UI-2 文案：`expected '<section ...' not to contain 'PonySentry'`（aria-label + toast 仍含）
