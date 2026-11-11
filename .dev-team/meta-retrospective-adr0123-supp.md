# Meta Retrospective — wave adr0123-supplement（2026-11-12）

## 波次事实（物理收据）

| 项 | 值 |
|---|---|
| 基线 | `d5fa1673`（ADR 0123 已实施：`27da736d`） |
| 交付提交 | `a5634316` fix(context): 中断轮上下文闭合（ADR 0123 补充） |
| 范围 | 4 项修正（C/A/D/B）+ ADR Supplement + 6 例验收测试 |
| 代码行 | 5 文件 +143/−32；新增测试 378 行 |
| 红相 | `adr0123_supplement_test` 在 `git archive HEAD` 纯净树 exit 101（E0432 未定义新符号） |
| 绿相 | 同测试 exit 0（6 passed / 0 failed / 0 ignored） |
| 回归 | `multi_turn_tool_context_test` exit 0（3 passed，与 `.dev-team/regression-baseline-wave-adr0123-supp.json` 逐项一致） |
| 编译 | `cargo check -p pony-agent-core --features test-support` exit 0（提交前复跑同样 0） |
| 门禁 | 提交钩子：adr-verify 通过 / 归档封印 117 件一致 / backlog-lint PASS；`audit-diff.sh --base` exit 0；`check-declaration.sh` = UNKNOWN(3)（巡检证据不可得，需人工补证） |
| 看板 | task-1/2/3 全部 completed，无残留 in_progress |

## 1. 通信拓扑与信噪比

- 有效机制：Lead 先落 `contract-matrix-adr0123-supplement.md`（含 A1/D1/C1 签名与不变量），三个执行/测试角色回执统一用「函数 + 行号 + 真实退出码」对账，零过程吹捧。
- 噪声点：exec-context 首轮用 `| tail -20` 观测测试，因 lib 二进制结果块被截断而误报「0 tests」——**管道截断会伪造收据**，必须显式 `--lib`/指定 test 名并保留退出码。
- 越界请求 1 次：exec-turn 需改 `session/mod.rs` re-export（写域外），由 Lead 以「公共接线唯一组装」例外执行并回告，未放开其写域。

## 2. 门禁穿透与误杀率

- 真实拦截：若无 C1 修复，T-C 的「tool 消息数 == tool_calls 总数」与「无孤儿 tool」双断言必红；测试刻意加 `PROBE_HEAD_0` 消失断言，防止实现「删光消息」骗过配对断言。
- 红相在共享工作区被并发实现「抢先变绿」（A1/D1 先落地），Test Agent 改用 `git archive HEAD` 纯净树 + 独立 target 取得 exit 101——**共享工作区下红相必须用干净树取证**，建议固化为纪律。
- 存量红灯 2 例（control_plane 冻结红测、jobs 并发抖动）由 Executor 用纯净树复现归因，未计入本波回归。

## 3. 分工契约与隔离有效性

- 写域互斥成立：`git status` 显示本波团队只碰 6 个文件，其余约 20 项改动全部来自并行的另一 TUI 会话（`.dev-team/*`、`backlog/*`、`scripts/*`、`docs/architecture/*`），提交时用定向 `git add` 排除。
- 分权边界守住：拒绝 Executor「在自己实现文件内补验收断言」的提议，验收断言只留在冻结的 `tests/adr0123_supplement_test.rs`。
- 风险：并发的另一会话在 02:18–03:20 自行 `git add -A`/commit（`0685803b`、`a6a2396d`），把本波 `.dev-team/` 产物一并带走。**多会话共享仓库必须「定向暂存 + 波次前缀」**，否则提交归属不可追溯。

## 4. 元协议迭代建议

1. **收据完整性**：门禁命令统一 `> log 2>&1; echo exit=$?; tail -n 20 log`，禁止裸 `| tail` 收尾。
2. **红相取证环境**：共享工作区场景下红相必须在 `git archive <base>` 独立树跑，否则并发实现让红相永远拿不到非零退出码。
3. **并发提交探测**：进入波次前探测「近 30 分钟是否有其他会话提交」，有则本波一律 `git add <显式路径>` + 提交信息带波次标识。
4. **ADR 双真源**：本波只改 implemented 修订版（archived 冻结版按封印不动），建议 `write-adr` 增「implemented 与 archived 标题/章节漂移」检测。
5. **残留待办**：`turn_stream.rs:30-37` 的 `claim_failed_terminal` suppressed 分支存在同类「终态已收口但无持久化」口子，概率低于本波场景，应登记 backlog 并补一条「0 步取消/失败轮 history 必有 user 消息且 turn 数恰为 1」的断言。
