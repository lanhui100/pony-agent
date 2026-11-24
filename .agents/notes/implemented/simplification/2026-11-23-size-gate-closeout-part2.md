# Agent Note: size 棘轮收口第二步——两个巨型测试文件按主题外移

Status: implemented

## Problem

`npm run size:check` 在 `main` 上原有 4 条新增超限。第一步（ADR
`2026-11-23-size-gate-closeout-part1`）收掉了 `config.rs` 与 `process.rs::start_inner`，
剩两条都是**测试文件**：

| 位置 | 实测 | 基线 |
|---|---|---|
| `agent/provider/tests.rs` | 3057 行 | 基线无此条目（新增超限，阈值 3000） |
| `agent/runtime/tests.rs` | 13913 行 | ≤ 13902 |

测试文件超限不能靠"压缩断言"或"登记基线"解决：前者伤可读性，后者等于宣布
"测试可以无限长"。仓里已有先例可循——`provider/retry_status_tests.rs` 就是用
`#[path = "..."] mod retry_status_tests;` 拆出去的兄弟模块。

## Decision

1. **`provider/tests.rs` 3057 → 2772**：把尾部 287 行（请求净化 `sanitize_request_strips_images_*`、
   流式读超时 `streaming_client_uses_idle_read_timeout_*`、冷却窗口 `followup_cooldown_*` /
   `cooldown_window_*`、工具面 wire 串）外移到 `agent/provider/tests_streaming_wire.rs`，
   用 `#[path]` 声明为 `tests` 的子模块；新文件沿用 `retry_status_tests.rs` 的写法（顶层 +
   `use super::*`）。
2. **`runtime/tests.rs` 13913 → 13813**：把尾部 C9「中断轮次清理」用例（103 行）外移到
   `agent/runtime/tests_interrupted_cleanup.rs`，同样用 `#[path]` 引入。
3. **只搬不改**：被搬的测试体逐字未改（断言、注释、sleep 预算全部原样），因此测试数与语义不变。

## Verification

- `npm run size:check`：`4 new violation(s)` → **`2 new violation(s)`**（剩 `config.rs` / `process.rs`，
  已由 ADR part1 的分支处理；两者合入后应为 0 条新增）
- `cargo-lane test -p pony-agent-core --lib` → **1665 passed / 0 failed**（与拆分前同数，证明无遗漏）
- 拆分后 `provider::tests` 117 个用例全绿，其中新模块用例被正确归属
  （`agent::provider::tests::tests_streaming_wire::streaming_client_uses_idle_read_timeout_not_total_deadline`）

## Alternatives considered

- **给两个测试文件在 `file-size-baseline.json` 登记豁免**：最省事，但等于承认"测试文件不受棘轮约束"，
  而这两条正是最近长出来的（provider/tests 还是新增条目），没有历史包袱可讲。
- **把测试拆成 `tests/` 目录 + `mod.rs`**：更"标准"，但要同时改 `provider/mod.rs` 的 `mod tests;`
  路径与所有相对 `#[path]`（如已存在的 `retry_status_tests.rs`），改动面翻倍且与既有写法不一致。
- **压缩断言/合并用例来减行**：直接降低可读性与故障定位能力换绿灯，是最差的一种做法。
- **只拆 `provider/tests.rs`（它超得最多）**：`runtime/tests.rs` 只超 11 行，但同样是"长出来"的，
  留着它下次照样红；两个一起做成本只多十几分钟。

## Reintroduction conditions

- **把 `tests_streaming_wire.rs` 的内容搬回 `provider/tests.rs`**：仅当父文件因其它重构整体变短
  （实测 ≤2700 行，即距 3000 阈值还有 10% 余量）且这些用例重新成为「必须与邻近断言同读」时；
  在那之前由文件行数棘轮守着。
- **把 `tests_interrupted_cleanup.rs` 的内容搬回 `runtime/tests.rs`**：仅当该父文件被整体拆分
  （例如按 turn/stream/tool 三段分家）时随那次拆分一并迁移；不得因「少一次 `mod` 声明」搬回。
- **再外移新块的条件**：`tests.rs` 再次逼近阈值（>90% 阈值）或出现明确成组的新主题时，按本次同样的
  `#[path]` 兄弟模块写法外移，不新增 `file-size-baseline.json` 条目。
