# Agent Note: test-hygiene 3b-1 —— 修复既有测试竞态/夹具漏洞并抽离 CLI 库边界（bin-as-module 50→49）

Status: implemented

## Problem

在 CI 流水线收敛与 PR #86/#87 验证期间暴露出四类影响主干绿相与测试可信度的既有缺陷，同时 `crates/pony-agent-core/tests/` 仍有 50 个测试文件以 `#[path = "../src/bin/..."]` 将二进制根当模块编译，掩蔽死代码并拉高编译体积：

1. **`tests/acceptance/wave47-ob2-enforcement.spec.ts` T5/T6 夹具缺失 `hint`**：fixture 内 `forbidden_in_other_files` 条目缺少必填字段 `hint`，使结构校准前置失败或偏离真实声明格式。
2. **`scripts/install-cli.spec.mjs` P4 Linux 占位就绪竞态**：`spawn(dest, ["30"])` 后仅盲等 `200ms`，在高负载 CI runner 上 `exec` 尚未完成时即读取 `/proc/<pid>/exe`，可能漏检 holder。
3. **`crates/pony-agent-core/src/agent/jobs.rs` `promoted_job` 异步回收竞态**：`JobSession::kill_internal` 先写 `status = "killed"` 并释放 `status_guard` 锁，再在锁外重试调用 `release_promoted_entry`（至多 `20×10ms`）；单测仅等待 `status != "running"` 即断言 `!manager.is_tracked(...)`，产生 TOCTOU 竞争窗口。
4. **`crates/pony-agent-core/src/agent/otel/mod.rs` `begin_turn_trace_with_parent` TOCTOU（及测试未加锁）**：在 `if spans_allowed()`/`current_state()` 与 `state.tracer.build_with_context(...)` 之间，若并发测试/宿主关闭了门禁或调用了 `shutdown()`，`GatedSampler`/已关闭 provider 会产出 `SpanContext::NONE`（`is_valid() == false`）；原代码仍将该无效上下文登记为活跃 turn，导致 `current_trace_id()` 与 `current_traceparent()` 双双返回 `None`（直接命中 Windows CI `main` 偶发失败），且 `otel/tests.rs` 中的 `begin_turn_trace_with_parent_inherits_trace_id_locally` 未持 `test_lock()`。
5. **`crates/pony-agent-core/tests/cli_acceptance.rs` 耦合 `bin/cli.rs`**：测试为测 `parse_cli_args`、`SubCommand`、`SlashCommand`、`ModelsCommand` 等纯解析与配置报告函数而 `#[path = "../src/bin/cli.rs"] mod cli`。

## Decision

作为分期收口 **3b-1**（先止血再大规模整合测试目录）一次性落地以下两块改动：

1. **修复既有 4 处缺陷**：
   - `tests/acceptance/wave47-ob2-enforcement.spec.ts` T5/T6 fixture 补齐 `hint: 只能在收口点实现`；
   - `scripts/install-cli.spec.mjs` P4 改为最长 `2000ms` 轮询 `/proc/${child.pid}/exe === dest`，就绪后再执行替换自检；
   - `crates/pony-agent-core/src/agent/jobs.rs` `promoted_job_cancel_token_terminates_process_group` 在 `status != "running"` 后显式等待 `!manager.is_tracked(session_id, &handle)`（上限 `500ms`），与 `kill_internal` 无锁释放契约对齐；
   - `crates/pony-agent-core/src/agent/otel/mod.rs` extract helper `build_turn_trace_guard` 消重（保持 `otel/mod.rs` 不突破 `file-size-baseline.json` 4427 行基线），并在 `begin_turn_trace_with_parent` 仅当 `span_context.is_valid()` 时登记真实 SDK span，无效时 `drop(span)` 回落本地降级上下文；同时在 `otel/tests.rs` `begin_turn_trace_with_parent_inherits_trace_id_locally` 加回 `let _guard = test_lock();`。
2. **抽离 `pony_agent_core::cli` 库模块**：
   - 新建 `crates/pony-agent-core/src/cli.rs` 并由 `crates/pony-agent-core/src/lib.rs` 导出 `pub mod cli;`，承载 `SubCommand`、`SlashCommand`、`MAX_TIMEOUT_SECS`、`CliArgs`、`parse_cli_args`、`flag_value`、`ModelsCommand`、`parse_models_command`、`execute_models_command`；
   - `crates/pony-agent-core/src/bin/cli.rs` 改为 `use pony_agent_core::cli::{...}`（`bin/cli/acp.rs` 显式 `use std::str::FromStr;` 消除对父模块隐式导入依赖）；
   - `crates/pony-agent-core/tests/cli_acceptance.rs` 删除 `#[path = "../src/bin/cli.rs"] mod cli;` 与 `#[allow(dead_code, ...)]`，改为正常通过公共库 `use pony_agent_core::cli::{self, parse_cli_args};` 测试，并将 `scripts/bin-as-module-baseline.json` 由 `50` 处收紧至 `49` 处。

## Verification

- `npm run size:check`：`FILE-SIZE GATE: 0 new violation(s) / 58 over limit / 58 grandfathered / 0 shrunken baseline entr(ies)`
- `npm run contract:bin-imports && npm run contract:bin-imports:selftest`：通过，基线 `50 → 49` 文件 / `49` 处引用，`check-bin-test-imports.spec` 7/7 PASS
- `npm run rust:lint`：通过（`实测 3843 / 基线 3870，新增/恶化 0`，新模块 `src/cli.rs` 零告警）
- `npm run rust:lint:tauri`：通过（`实测 3303 / 基线 3322，新增/恶化 0`）
- `npm run cargo:check:shared`：通过
- `npm run cargo:test:shared -- -p pony-agent-core --lib promoted_job`：`3 passed; 0 failed`
- `npm run cargo:test:shared -- -p pony-agent-core --lib agent::otel::tests`：`42 passed; 0 failed`，连续 5 轮压测均 `42 passed; 0 failed`
- `npm run cargo:test:shared -- -p pony-agent-core --test cli_acceptance`：`10 passed; 0 failed`
- `npm run install-cli:selftest`：P1–P4 全部通过
- `npm run test:unit`：`Test Files 184 passed (184)`，`Tests 1652 passed | 12 skipped (1664)`（含 `wave47-ob2-enforcement.spec.ts` 7/7）
- `npm run build`、`npm run contract:check`、`npm run contract:cli`、`npm run ext:check`、`npm run version:check`、`npm run adr:check`、`npm run backlog:gate`、`GATE_BASE=origin/main node scripts/observability/gate.mjs`、`audit-diff.sh --base origin/main`：全部退出 0

## Alternatives considered

- **一步把剩余 49 个 `bin/tui.rs` 测试也全部拆完再交一整个大 PR**：`bin/tui.rs` 耦合 `TuiState` 与终端事件流（涉及 3 平台 `clippy-baseline.json` 共 600+ 存量条目与多处结构性拆分），合并进同一 PR 会导致评审面过大并与同期主干会话发生 rebase 冲突；按 3b-1（修既有缺陷 + CLI 解耦）→ 3b-2（测试目录合套）→ 3b-3（TUI 解耦）三只小 PR 顺序推进更稳妥。
- **只在 `jobs.rs` `kill_internal` 里把 `release_promoted_entry` 挪进 `status_guard` 锁内**：`release_promoted_entry` 内含 `20×10ms` `sleep` 轮询退出确认，持锁睡眠会阻塞并发的 `job_output` 查询；让调用方等待回收收口并保持锁外清理更符合既有非阻塞设计。
- **放宽 `file-size-baseline.json` 给 `otel/mod.rs` 涨 6 行**：`otel/mod.rs` 本就有 `TurnTraceGuard` 三次重复字面量初始化，抽私有 `build_turn_trace_guard` 反而净减行数且消除重复代码。
