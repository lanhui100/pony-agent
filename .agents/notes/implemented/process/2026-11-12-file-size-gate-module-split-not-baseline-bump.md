# Agent Note: file-size 门禁的收口方向——按主题拆模块，不抬基线

Status: implemented

## Problem

分支 `verify/long-running-stress` 上 `node scripts/check-file-size.mjs --baseline
scripts/file-size-baseline.json` 实测 **9 条「新增/恶化」违规**（SIZE_EXIT=1）：

| 文件 | 实测 | 基线允许 |
|---|---|---|
| `agent/provider/mod.rs` | 9811 | ≤9768 |
| `agent/runtime/turn_stream.rs` | 3582 | ≤3578 |
| `agent/runtime/turn_stream.rs:198 handle_stream_tool_turn` | 1590 | ≤1588 |
| `agent/runtime/turn_stream.rs:1822 start_turn_stream_with_control_and_facts` | 1737 | ≤1735 |
| `agent/runtime/turn_sync.rs:689 run_turn_inner` | 623 | ≤621 |
| `agent/session/store.rs` | 5879 | ≤5801 |
| `agent/session/tests.rs` | 6747 | ≤6739 |
| `agent/sqlite_session.rs` | 6277 | ≤6267 |
| `agent/turn_flow.rs` | 3677 | ≤3601 |

其中只有 `provider/mod.rs` 的 +43 行属「指标诚实化」lane，其余 8 条是分支上多 lane 合入后
未回写/未收口的历史漂移。这九条都是 grandfather 条目 —— 门禁语义是**只许减不许涨**。

关键判断：`npm run size:check:update` 会把 9 条一次转正。那不是「修门禁」，是把门禁改成
计数器：真退化与合法增长从此不可区分，下一次 3000 行阈值被突破时也不再有人被拦住。

## Decision

1. **一律拆模块回到基线，`scripts/file-size-baseline.json` 一个字节都不改**（不 update、不手改数字）。
2. 按主题切，不按行数切（每块必须是「读的人会一起看的一类东西」）：
   - `provider/mod.rs` 9811 → 9754：`extract_openai_usage` + `normalize_token_usage` → `provider/usage.rs`，
     `extract_chat_delta_text` / `openai_safe_tool_name` → `provider/openai_wire.rs`
     （边界与理由见 [`architecture/2026-11-12-provider-usage-module-split.md`](../architecture/2026-11-12-provider-usage-module-split.md)）。
   - `runtime/turn_stream.rs` 3582 → 3537、三处 25 个位置参数的内联 `emit_stream_event(...)`
     → `runtime/turn_events.rs`（`turn:started` / `turn:hop_complete` 的带类型窄接口；
     `run_turn_inner` 的同步发**射点同批收敛），顺带消掉 `turn_sync.rs:run_turn_inner` 与
     `turn_stream.rs` 两个超限函数。
   - `session/store.rs` 5879 → 5780：trace→历史消息映射（`extract_reasoning_content` /
     `derive_status_from_trace` / `enrich_history_from_traces`）与只读默认快照、env 采集
     → `session/store/store_history_view.rs`（`enrich_history_from_traces` 的 `pub(super)`
     改写为等价的 `pub(in crate::agent::session)`，保住 `session/mod.rs` 的 re-export）。
   - `turn_flow.rs` 3677 → 3379：PA-091/094 的事件持久化 + 冲刷注册表（静态量、会话绑定 guard、
     flush 入口）整体搬入 `turn_flow/turn_flow_registry.rs`，父模块按「生产 API / `#[cfg(test)]`
     测试专用 API」两组逐项 `pub use` re-export，`turn_flow::<fn>` 寻址契约不变。
   - `session/tests.rs` 6747 → 6726：`memory_backend_keeps_turns_in_process` 迁到
     `session/memory_backend_tests.rs`（先例：`host_continuation_goal_round_tests` /
     `history_compaction_tests`），并立下「`tests.rs` 不再新增用例子文件」的默认动作。
   - `agent/sqlite_session.rs` 6277 → 6233：库路径解析（`default_sqlite_path` /
     `data_dir_override` / `unique_test_sqlite_path`）→ `sqlite_session/sqlite_paths.rs`，
     以 re-export 保住 `sqlite_session::default_sqlite_path` 的 6 处外部调用点。
3. **可见性只收紧不扩张**：搬出后跨模块访问一律 `pub(super)` / `pub(in crate::agent::session)`
   / `pub(crate)`，公开面靠逐项 re-export 复原；`#[cfg(test)]` 门随代码原样迁移。
4. 子模块落位遵循 Rust 2018 的目录规则：`foo.rs` 的子模块必须在 `foo/` 目录下
   （`session/store/store_history_view.rs`、`turn_flow/turn_flow_registry.rs`、
   `sqlite_session/sqlite_paths.rs`），避免 `mod` 解析到错误路径。

## Alternatives considered

- **一次性 `size:check:update` 抬基线**：最省力，代价是 9 条真实退化转正、门禁此后只剩计数语义；
  与「门禁是承诺的可验形态」直接冲突。
- **只抬本 lane 涉及的那 1 条**：形式上更诚实，但仍属同一次退化的橡皮图章 —— 8 条历史漂移
  会因为「不归我」永久留着，下一个人只学会「反正可以 update」。
- **给越线项加 `#[allow(...)]` 或豁免名单**：size 门禁没有行级豁免语义，加了就出现第二套判据，
  两份判据必然漂移。
- **维持红相、只把 9 条记为 tech debt**：门禁长期红等于没有信号，CI 里会被忽略；
  B118–B120 三条 backlog 只是记账，不解决「没人能看见新的越线」。
- **更激进的重构（把 store.rs / turn_flow.rs 各拆成 4–5 个千行内模块）**：超出「回到基线」的
  最小必要闭包，且会与并行 lane 的编辑面大面积重叠；按需再拆。

## Consequences

- 门禁恢复：9 条中 8 条在本次消除，`provider/mod.rs` 的收口见上表与架构条
  （其后被并行 lane 的 525 重试用例重新推到 9842，属该 lane 写域，须以同一纪律收口，**不得抬基线**）。
- 新增 6 个子模块文件（`provider/openai_wire.rs`、`runtime/turn_events.rs`、
  `session/store/store_history_view.rs`、`turn_flow/turn_flow_registry.rs`、
  `session/memory_backend_tests.rs`、`sqlite_session/sqlite_paths.rs`），每个都远低于 3000 行阈值。
- 门禁实测（19:33，本次拆分完成时刻）：`FILE-SIZE GATE: 0 new violation(s) / 63 over limit /
  63 grandfathered`，退出码 0；`scripts/file-size-baseline.json` 未被修改。
- **诚实条款（19:56 复测）**：同一命令又报 2 条新增越线 —— `provider/mod.rs` 9772（≤9768）、
  `runtime/turn_stream.rs` 3680（≤3578）。两处都不是本次拆分回退，而是**并行 lane 在同一批文件上
  继续追加**：525 重试分类（`retry.rs` / `tool_recovery.rs` / `turn_stream.rs`，19:48–19:49 连续写入）
  与 turn speed 证据结算（`turn_flow.rs::settle_turn_speed_evidence`，新增 `turn/speed` 事件）。
  这两条仍按本决策处理：**由各自 lane 拆模块收口，禁止 `size:check:update`**。
- 证据：`cargo check -p pony-agent-core --tests` 退出码 0；`cargo test -p pony-agent-core --lib`
  **1523 passed / 0 failed**；`node scripts/check-file-size.mjs --baseline
  scripts/file-size-baseline.json` 中上述 8 条消失；基线文件未被修改（`git status` 无该项）。
- 代价：模块数 +5，跨文件跳转增加；换来的是「按主题找代码」而不是「按 9000 行文件滚屏」。
  后续同类改动必须先立模块边界再写代码，否则同一文件会再次触线。
