# Tasks

- [x] 用 `git show a2636da` 调查该提交对 `config.rs` / provider 相关代码的改动，确认 `resolve_selection` 语义变化。
- [x] 阅读 `crates/pony-agent-core/src/agent/config.rs` 的 `resolve_selection` 与 `normalize_storage` / `dedupe_provider_models` 完整实现，定位根因。
- [x] 评估 `src-tauri/tests/provider_registry_regression.rs` 内所有 `64000` 断言（128 行、384 行）与 `config.rs` 单测（1584 行），判定各自是否过时。
- [x] 判定修复方向：更新测试基线（不修生产逻辑），写入 design.md 的 Decision Summary 与 Rejected Alternatives。
- [x] 建立 OpenSpec 变更目录 `openspec/changes/fix-provider-registry-fallback-test-baseline/`（proposal.md / design.md / tasks.md / specs/provider-registry/spec.md）。
- [x] 更新 `src-tauri/tests/provider_registry_regression.rs` 第 384 行断言为 `4096` 并补充根因注释。
- [x] 运行 `npm run cargo:check:shared` 类型检查。
- [x] 运行 `npm run cargo:test:exact -- --workspace --test provider_registry_regression` 回归测试。
- [x] 更新 `docs/INDEX.md` 8.1 小节登记本变更。
- [x] 填写本文件 Validation Notes。

## Validation Notes

- `npm run cargo:check:shared`：通过（`Finished dev profile`，2.06s）。
- `npm run cargo:test:exact -- --workspace --test provider_registry_regression`：**8 passed / 0 failed**，
  其中 `resolve_selection_falls_back_to_selected_provider_and_model`（改为 4096 断言后）通过，
  其余 7 个回归测试不受影响。仅有与本变更无关的增量编译告警
  （`sse_turn_probe` incremental session 目录权限，Windows os error 5，预存）。
- 未改动 `crates/pony-agent-core` 生产代码，因此不需要运行全量 core lib 测试。
