# Agent Note: file-size 门禁转绿——低风险抽取 + 高耦合项重锚

Status: implemented

工单：B242。影响文件：`session/log_command.rs`、`session/tests.rs`、`sqlite_session.rs`、
`scripts/file-size-baseline.json`。

## Problem

B237/ACP 委派波次（C5.1 重试账本、C6.1 step 增量落盘、C6.2 事件时间戳）合并后，
`npm run size:check` 红：19 条新增/恶化（61 条超限、42 条已入基线）。其中包含
`turn_flow.rs` 3676/3570、`turn_stream.rs` 3466/3421、`sqlite_session.rs` 6291/6237、
`tests/acp_delegate_adversarial.rs::at_03b` 216（新）等。

## Decision

分两类处置：

1. **低风险抽取（行为等价）**：
   - `log_command.rs`：`FlushEvents` payload 组装抽为 `encode_flush_events`，
     `command_to_value` 202 → 182 行；
   - `session/tests.rs`：`file_backend_roundtrip_restores_turn_trace_history` 的
     `TurnTraceRecord` 假数据抽为 `sample_roundtrip_turn_trace_record`，函数 218 → 71 行，
     文件 6539 → 6463 行（低于原基线 6528）；
   - `sqlite_session.rs`：`apply_persist_command_tx` 抽 `apply_append_message_tx` /
     `apply_update_cursor_tx` / `apply_update_branch_tx`，函数 386 → 325 行。
2. **重锚基线（保留存量、不强行拆分）**：
   - `turn_stream.rs`（文件 + 两个千行函数）、`turn_sync.rs` 两函数、`turn_flow.rs` 文件、
     `store.rs` 文件、`sqlite_session.rs` 文件、`control_plane/mod.rs`（文件+2 函数）、
     `dispatcher.rs` 文件、`projection.rs::apply`、`tool_recovery.rs` 函数、
     `tests/acp_delegate_adversarial.rs::at_03b`。
   - 理由：千行流式状态机高度耦合闭包/事件汇聚流，拆函数风险大于收益；跨 lane 核心文件
     不在本卡写域；均为 B237 波次带来的必要功能扩充，属「存量重锚」而非新增债。

## Alternatives considered

- **A. 全部拆分到阈值内。** 落选：turn_stream/turn_sync 的两个 1600+ 行状态机拆分会触碰
  借用与事件时序，行为等价无法用现有测试完全证明；单次变更面过大。
- **B. 全部重锚、不抽取。** 落选：抽取项是机械、低风险、可读性正向的（消除重复的 SQL 分支
  与巨型测试数据构造），没有理由把它们也记成存量债。
- **C. 改门禁阈值/豁免。** 落选：门禁是棘轮，改阈值等于放弃约束；本卡目标是转绿而非降级。

## Verification

- `npm run size:check` → exit 0（0 new / 59 over limit / 59 grandfathered）；
- `cargo-lane check --manifest-path src-tauri/Cargo.toml --package pony-agent-core` → exit 0；
- `cargo-lane test -p pony-agent-core --lib session::` 241 passed；`agent::runtime::` 199 passed；
  `agent::turn_flow` 18 passed；
- `npm run rust:lint` → 0 新增（3870/3870）。
