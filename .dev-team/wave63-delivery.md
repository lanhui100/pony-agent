# wave-63 交付说明：剩余待办清偿（composer stopping / TUI 按键反馈 / V10 元测试递归）

## 1. 三项待办与落点
| # | 待办 | 处置 | 提交 |
|---|---|---|---|
| 1 | 桌面端 `WorkspaceComposer` stopping 禁用态（wave-60 T4 因并发 wave-61 冲突被排除） | 3+5 行：`primaryActionDisabled` 判 `phase==="stopping"`；`primaryActionTitle` 提示「正在停止…（等待本轮安全边界收口）」。真源是 store，不另存状态 | `86042d4` |
| 2 | TUI stopping 期按键被静默吞（wave-62 L3-A ADVISORY） | `TuiState::note_stopping_key()` + `status_word` stopping 分支透出提示；Esc / Ctrl+C 两处接线（不重复派发、不伪造终态） | `889fb98` |
| 3 | 全量 vitest 因 V10 元测试子进程递归跑全量而 >20min 不收敛 | 双保险嵌套守卫：子进程注入 `PDS_V10_NESTED=1` + 本文件被嵌套收集时 skip；新增 V10-4 自证 | `93bdca3` |

## 2. 机器收据（Lead 亲跑）
| 门禁 | 结果 |
|---|---|
| `cargo test --test tui_stop_key_feedback -j 1` | Exit 0 · 6 passed（3 新 + 3 既有 bin 单测） |
| `cargo test --test tui_stop_wave60 --test tui_stop_static_wave60 --test tui_batch2_regression --test tui_batch3_regression -j 1` | Exit 0 · 12 / 16 / 3 / 6 passed |
| `cargo check --all-targets -j 1` | Exit 0（仅既有 dead_code 警告） |
| `npx vitest run tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` ×2 | Exit 0 · 4 passed · 205s / 206s（幂等收敛；原单跑 3m10s 但嵌套会放大） |
| `npx vitest run`（全量，V10 修后） | 见交付时的最终收据（本文件末尾追加） |

## 3. 红相与分权
- 第 2 项走红相→绿相：`crates/pony-agent-core/tests/tui_stop_key_feedback.rs`（3 用例）先锚定（`2866ebe`，tag `wave-63-red`），实现后转绿；Executor 全程未改测试。
- 第 3 项属测试基建自身修复，无红相（其价值由 V10-4 自证与"全量收敛"证明）。
- 第 1 项是已评审代码的补提交（wave-60 T4 交付的一部份），无新增逻辑。

## 4. 已知事项
1. 主工作区仍有并发 wave-61 的未提交改动（`provider/*`、`subagent_runtime.rs` 等）——本波提交严格按路径定向暂存，未纳入其文件。
2. 未跟踪目录 `crates/pony-agent-core/examples/`、`target-base/` 出现在工作树（非本波产物，未处理）。
3. `wave-62/tool-cancel` 分支仍未合并（wave-62 交付说明 §6）。
