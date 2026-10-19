# Wave-39 NFR 达标报告：TUI Composer 收边 + 底部指标行

- 波次：wave-39（分支 `composer-metrics`）
- 契约：`.dev-team/contract-matrix-tui-dialog-wave39.md`（v1 + §5 v2）
- NFR 基准：`.dev-team/nfr-baseline-wave39.json`（`dev-nfr-lint.py` Exit 0）
- 实现：`crates/pony-agent-core/src/bin/tui.rs`（+~200 行，单文件）
- 决策：ADR 0042

## 机器门禁

| 门禁 | 命令 | 结果 |
| :-- | :-- | :-- |
| 空桩冻结 | `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony` | Exit 0（protocol v1 `ba530bd`、v2 `f9f400b`） |
| 红相 A1-A10 | `cargo test -p pony-agent-core --test tui_dialog_wave39` | Exit 101（0 passed / 10 failed；commit `316245b` 锚定） |
| 绿相 A1-A10 | 同上 | Exit 0（10 passed） |
| 红相 A11 | `cargo test -p pony-agent-core --test tui_usage_wave39_delta` | Exit 101（0 passed / 4 failed；commit `cf4129e` 锚定） |
| 绿相 A11 | 同上 | Exit 0（4 passed） |
| 无警告编译 | `RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony` | Exit 0 |
| 回归基线 | `tui_acceptance` / `wave38-tui-workspace-contract` / `tui_render_layout` | 34/1、15/0、3/5 —— 与波次起点逐项一致，无新增失败（既有红灯见 PA-147） |
| 真实 TUI E2E | `tools/tui-audit/run_audit.sh --phase after --bin target/debug/pony` | Exit 1：365/369 pass；4 fail 为既有命令面漂移（见 PA-146），wave-39 相关 58/58 PASS |

E2E 证据：`.dev-team/evidence/tui-polish/after/`（18 场景 / 369 检查 / PNG+ANSI），归因件 `.dev-team/evidence/tui-polish/wave39-attribution/`。

## NFR 五问映射（L3-P，`review-wave39-p.json`）

| 维度 | 基准字段 | 结论 | 证据 |
| :-- | :-- | :-- | :-- |
| 外部调用超时 | `external_call_timeout_ms` ≤3000 | PASS（machine） | 本波次无新增网络/外部 IO；唯一新增 IO 为 `ProviderRegistryStore::load_view()` 本地单文件读（实测 0.165 ms/次） |
| 日志规范与防吞 | `logging.format=json` + trace_id | PASS（machine） | diff 无 print/tracing；新增 `let _ = tx.send(..)` 与既有 sink 同惯用法（SendError = UI 已退出） |
| 资源释放 | `resource_release` | PASS（non-machine → Lead 复核） | 无线程/句柄/跨 await 锁；`load_view()` 即读即释放；调用点在 UI 主线程每轮 1 次 |
| 并发与幂等 | `concurrency_lock` | PASS（machine） | `apply_turn_usage` 唯一调用点为主事件循环；按轮幂等覆盖（A9/A11）；`StreamUsage` 唯一构造点 `tui.rs:1295`，仅 `turn:output_end` 发射 |
| 配置与自检 | `resource_release` 条目 | PASS | 两条新条目可由 grep 复核（`metrics_line` 纯函数；usage 更新单点） |

## 残余风险（已登记 state.json → `waves["39"].adjudications`）

1. Esc 竞态：`Esc` 直接置 `is_streaming=false`，若 stop 落在最后一次 cancel 判定之后、`output_end` 之前（persist 毫秒级窗口），旧轮 usage 仍可能落到新一轮 slot；窗口已由 v2 从「completed 钩子长尾」缩至「persist 写入」，与基线同级，判非回归。
2. `char_width` 缺 emoji 区（0x1F680–0x1F6FF 等）：含 emoji 的 cwd 可能少算 1 列，仅导致右簇尾部 1 列被裁，不越界；HEAD 同样存在，本波次不修。
3. `fmt_token_count` 在 999_950..=999_999 显示 `1M`（而非 `1000k`）：Lead 裁决接受，A7 数值契约不受影响。

## 关联工单

- PA-146：TUI E2E 命令面漂移（`/thinking` 后 8 命令 vs 冻结 7 命令期望）。
- PA-147：`tui_render_layout` 5 例 + `tui_acceptance` 1 例陈旧基线红。
