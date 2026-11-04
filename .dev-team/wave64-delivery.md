# wave-64 交付说明：停止即时生效 + 终态由「最后一条状态消息」唯一承载

- 契约：`.dev-team/contracts/wave-64-stop-terminal-status.md`
- 分级：B（跨 core/src-tauri 表面/前端；命中 blast 关键字 lock/thread/io；含 UI 交互）
- 基线 commit：`06472465` ｜ 红相锚定：`af599ef4` ｜ 实现提交：见 `git log --grep="wave-64"`

## 1. 用户诉求 → 根因 → 修复

| # | 用户可见问题 | 根因（取证） | 修复 |
|---|---|---|---|
| 1 | 桌面端点停止**不是马上终止** | 桌面 `stopTurn()` 因 `activeRunId` 恒非空 ⇒ 实际走 `stop_graph_run`，该路径只 `request_stop`：**不派发** `turn:stop_requested`（C2）、**不启动**宽限监督（C5）⇒ 只能等阻塞调用自然返回（可远超 2s） | `stop_graph_run` 锁内只读快照 → 出锁复用 `stop_turn`（C2 同步一条 `turn:stop_requested` + C5 宽限监督 2000ms/250ms）→ 回填 graph 控制结果；顺带消除「锁内 `load_execution_checkpoint` 重入 `graph_runs` 自死锁」隐患 |
| 2 | 停止时**多出一条绿色**提示 | `turn:cancelled` 把「你已停止本轮 · 发送新消息可继续」写入 `sessionNotice` ⇒ `info` 模式 ⇒ `emerald` + `CheckCircle2` | 终态改由 `agentRuntimeStatus` 的 `stopped` 模式承载：lucide **Pause** 暂停图标 + stone 中性色（绿系零出现）、无 shimmer、无耗时；`sessionNotice` 不再承载停止文案 |
| 3 | 停止时仍出现**错误消息** | ①工具执行期取消被归 `turn:failed`；②provider 读通道取消码 `provider_stream_cancelled` 冒泡进 failed 分支 ⇒ 错误态 + 错误正文 | 新增 `classify_stream_error`（取消码 **且** 有停止位 ⇒ cancelled）＋ `claim_terminal_status` terminal-once 闸门（同一 turn **恰好一条**终态）；删除 `turn_stream.rs` 中不可达的 followup 取消分支 |
| 4 | 生成错误时**额外的错误信息** | 失败轮写占位正文「本轮执行失败，请查看右侧 trace。」＋消息区「错误详情」折叠面板＋状态行，三处重复 | 失败终态同样由状态行承载（rose + AlertCircle，既有体系内色）；移除占位正文与错误详情面板；wire text 归一化：`"This turn failed."`（零正文兼容锁）不落正文，`正文+失败后缀` 只留正文；后台会话分支同步生效 |

**未做（非目标/后续 backlog）**：`turn_sync.rs` 非流式轮 `aborted→failed`；非命令类工具的 handler 执行期不可中断；子代理 120s 同步等待与 hook 子进程不观测 turn 取消；`runtime.ts:4843` 失败轮用 wire 4096B 截断正文覆盖已流式正文（既有行为）。

## 2. 门禁物理收据（全部机器 Exit Code）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 红相（冻结） | vitest（wave64 + 3 迁移件） | EXIT 1（15 红，均为契约目标判据） |
| 红相（Rust） | `cargo test --test stop_graph_run_wave64` | EXIT 101（R1 事件序列 `[]`；R2-2 实际 `turn:failed`） |
| 红相（R3 追加） | 同上 | EXIT 101（实测 `…turn:stop_requested, turn:cancelled, turn:cancelled` —— 两条 cancelled） |
| 前端绿相 | `npx vitest run tests/acceptance/wave64 tests/stop-stop-turn.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts tests/runtime-status.spec.ts tests/workspace-runtime-status-integration.spec.ts tests/workspace-turn-item-layout.spec.ts` | EXIT 0（9 files / **229 passed** / 7 skipped） |
| 前端类型 | `npm run typecheck` | EXIT 0 |
| 前端全量 | `npx vitest run tests/` | EXIT 0（143 files / 1347 passed） |
| Rust 绿相 | `cargo test --test stop_graph_run_wave64 --test stop_contract_wave60 --test stop_escalation_wave60 --test stop_interruptible_read_wave60` | EXIT 0（7+9+6+3 passed，wave-60 零回归） |
| Rust 检查 | `cargo check --manifest-path crates/pony-agent-core/Cargo.toml` | EXIT 0（零 warning） |
| 真实浏览器 | `npx playwright test tests/e2e/wave64-terminal-status.spec.ts` | EXIT 0（data-mode=stopped、Pause 图标、零绿系、消息区无额外中止消息、**Console Error = 0**） |
| 可观测性 | `audit-diff.sh --base af599ef4` / `check-declaration.sh` | EXIT 0 / PASS(exit 0) |

证据截图：`docs/release/evidence/wave-64/stopped-terminal-status.png`（实测呈「⏸ 已停止」，stone 中性色）。

## 3. 对抗审查（3 路 + delta 复审）

| 审查 | 首轮结论 | 处置 |
|---|---|---|
| L3-A 并发/锁序 | FAIL（1 BLOCKING：宽限后 in-band 二次收口 ⇒ 两条 `turn:cancelled`）+ 4 ADVISORY | 新增 `claim_terminal_status` 闸门（CAS）+ 修 A2/A3/A4/A5，R3 红相用例转绿 |
| L3-B 契约/前端 | FAIL（1 BLOCKING：后台会话 failed 分支仍落占位正文）+ 4 ADVISORY | 新增 `normalizeFailedTurnContent`（前/后台统一）+ 注释/aria/测试假绿收口 |
| L3-P 生产六问 | PASS（7 ADVISORY） | 逐条评估：证据/日志/双终态等由上述两轮整改覆盖；遗留项登记于 §1 未做清单 |
| L3 delta 复审 | FAIL（1 BLOCKING：`prepare_turn` Err 分支漏闸，10 处内联只盖 9 处）+ 1 ADVISORY | 补闸后 10/10 全过闸；复审复验（见 `review-wave64-recheck.json`） |

审查产物：`.dev-team/review-wave64-a.json` / `-b.json` / `-l3p.json` / `-recheck.json`（均经 `dev-review-lint.py` Exit 0）。

## 4. 兼容性与风险登记

- **breaking（已登记 §4 迁移）**：`sessionNotice` 不再承载停止文案；消息区不再渲染「错误详情」面板；`tests/stop-stop-turn.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/runtime-store.spec.ts` 共 5 条断言按契约迁移。
- **additive**：`stop_graph_run` 现在会派发 `turn:stop_requested` / `turn:cancelled`（既有类型，新路径）；`AgentRuntimeMode` 新增 `stopped`。
- **跨语言字面量耦合**：前端失败后缀字面量与 Rust `FAILED_PARTIAL_TEXT_SUFFIX` 逐字一致（复审字节比对 EQUAL），任一侧改字会静默失配 ⇒ 建议后续加同步护栏用例。
- **共享工作区**：本波提交经定向暂存（`git update-index --cacheinfo` 隔离同文件其他会话改动），未越界提交他人未提交改动。
