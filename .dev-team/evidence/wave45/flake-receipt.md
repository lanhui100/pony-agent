# wave-45 全量 Rust lib 结果与既有 flake 证据

## 结果对照
- `cargo test -p pony-agent-core --lib --target-dir target-test`（并行，默认线程）：
  `1154 passed; 2 failed` → 失败 = ① `agent::config::tests::missing_capabilities_fields_use_stable_defaults`（基线既有，wave-43 已记录）；② `agent::ponysentry::turn_failure_capture_tests::governance_block_failure_is_not_reported`
- `cargo test -p pony-agent-core --lib --target-dir target-test -- --test-threads=1`：
  `1155 passed; 1 failed` → 仅剩基线既有 config 失败。

## flake 归因（非本波引入）
- 仅跑模块：`cargo test --lib agent::ponysentry -- --test-threads=8` 连跑 20 次 → **2 次失败**（run 4/19），失败用例名相同。
- 机制：`turn_failure_capture_tests` 多个用例共享全局 `LAST_ENQUEUED`（`testing_last_enqueued_payload`），并行时彼此覆盖 baseline → 断言 `event_id` 不变被打破。
- 结论：既有测试竞态，wave-45 代码未触碰 ponysentry 模块；登记为 backlog（建议加 `serial_test` 或改为每测试独立队列）。

## 冻结集成测试稳定性（排除 L3-A 观察到的单次失败）
- `cargo test --test otel_resource_turn_status_test --test otel_resource_env_test --target-dir target-test` 连跑 6 次 → **6/6 全绿**（env 1 + status 6）。
- 结论：该次组合失败不可复现（疑与并行 cargo 调用/端口预算竞争），冻结测试稳定；已记录，不改断言。
