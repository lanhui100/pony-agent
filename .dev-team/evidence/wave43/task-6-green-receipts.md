# task-6（B6 契约迁移）绿相收据

## 1. 前端全量：`npx vitest run` → **Exit 0**
```
 Test Files  93 passed (93)
      Tests  986 passed | 10 skipped (996)
   Duration  46.35s
```
（含契约迁移后的 `tests/TelemetryPage.spec.ts`、`tests/settings.store.spec.ts`、
`tests/otel-reporting-switch.spec.ts` UI-1/UI-2，以及新增 toolRules 往返用例）

## 2. Rust 验收：`cargo test --manifest-path src-tauri/Cargo.toml -p pony-agent-core --test otel_trace_migration_test --target-dir target-test-exact-a` → **Exit 0**
```
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.72s
```
关键用例全部转绿：
- `ot7_retired_symbols_have_zero_hits_repo_wide`（**已无 PENDING 豁免表**，全仓 0 命中强断言）
- `ot7_retired_ponysentry_trace_symbols_have_zero_hits` / `ot7_issue_reporting_keep_surface_still_exists`（双向）
- `ot8_settings_bridge_syncs_core_gate_on_load_and_save`（OT-8d）
- `ot8_exit_path_releases_otel_provider`（OT-8e）
- OT-1 / OT-3 / OT-4 / OT-6 / OT-8a-c 全部绿。

## 3. 覆盖继承补测：`cargo test … --test ponysentry_issue_resilience_test --target-dir target-test-exact-a` → **Exit 0**
```
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
- `issue_reporting_burst_over_bounded_queue_never_blocks`（burst 2000 > QUEUE_CAPACITY 1024，入队 <500ms）
- `issue_queue_stays_bounded_and_drops_on_overflow`（静态锚点：QUEUE_CAPACITY=1024 / try_send / TrySendError::Full / 丢弃日志）

## 4. 覆盖对照结论
见 `deleted-tests-coverage-map.md`：16 条被删用例中 6 条继承、9 条合理消失、2 类「不该消失」——
① 有界队列抗雪崩 → 本任务已补；② 终态门控映射 → 语义迁到 turn span，`begin_turn_trace` 已在
`runtime/turn_sync.rs:460` 与 `runtime/turn_stream.rs:1292` 落地（executor 侧完成），
`is_trace_reportable_terminal` 的删除因此可判为合理消失。

## 5. 本轮改动文件
- `tests/TelemetryPage.spec.ts`（契约迁移 + 独立说明断言）
- `tests/settings.store.spec.ts`（严格载荷 2 键 + toolRules 往返用例）
- `crates/pony-agent-core/tests/ponysentry_issue_resilience_test.rs`（新增，覆盖继承）
- `crates/pony-agent-core/tests/otel_trace_migration_test.rs`（OT-7 收紧 / OT-8d / OT-8e）
未触碰任何 `src/**`。
