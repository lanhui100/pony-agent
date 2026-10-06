# Dev Team Meta-Retrospective — 0021 统一错误码注册表（骨架阶段）

## 1. 通信拓扑与信噪比（Topology & Noise）
- 拓扑：Lead + 独立 Test Agent（验证+C1-C7契约冻结）+ Executor（注册表骨架实施），经 team_task 看板（task-1→task-2→task-3）与 write_scopes 隔离并行写域，未发生脏写。
- Test Agent 只读侦察+只写 `.dev-team/error-contract.md`，Executor 只写 `error_code.rs`+`mod.rs` 接线，Lead 只写 ADR 与看板，信噪比高。
- 严苛 JSON 协议本次未触发 Reviewer 路（骨架变更小、机器门禁全绿即放行），符合爆炸半径分级。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）
- 机器门禁：`cargo check -p pony-agent-core` exit 0；`agent::error_code` 3 单测、`agent::retry` 36 单测、`agent::dispatcher*` 51 单测全绿；`timeout` 线码冻结与三形状兼容无穿透。
- Test Agent 红相价值：证伪 1 条（失败 turn `fallback_reason=None` 只有自由文本），加重 2 条（三形状、401/500 无细分），阻止了"直接全链强类型重写"的高爆炸方案。
- 误杀控制：`tracked_count` dead_code 告警经定位确认为既有文件引入，未误判为本次回归失败。

## 3. 分工契约与隔离有效性（Contract Isolation）
- 写域隔离有效：`.dev-team/`、`docs/decisions/proposed/`、`crates/.../error_code.rs` 三簇互不重叠；Executor 在 Lead claim task-2 后才落盘骨架，无抢写。
- 契约先行有效：C1-C7 黑盒矩阵冻结后，Executor 实施零返工；映射 Stage1-3（双写/retry收编/provider分支）得以按依赖序拆分为 task-4/5/6 独立验收。
- 残留：`handler_failure` 未注册码直通、`classify(&str)` 子串桥仍在，属已登记的双写期债务，不视为隔离失效。

## 4. 元协议迭代建议（Self-Evolving Protocol）
- `npm run cargo:check:shared` 经 powershell 分发在 Linux 下不可用，门禁应文档化直调 `cargo check/test -p` 降级路径。
- `cargo test` 多过滤名不支持多 TESTNAME，回归指令应拆分为单过滤多次调用。
## 5. Stage1-3 收敛追加（本轮）
- Stage1 双写：7 文件 kind+code 全写侧收敛，B4 code-wins + B5 纯字符串保文本锁定；agent::tools 6 失败经 stash 基线对照确认预存。
- Stage2 收编：retry 七张码表为唯一真源，classify 遗留桥语义零变更；retry 39 / T2-B 相关 10 全绿；grep 零新增。
- Stage3 分批：包装器五件套纯新增先合（check + retry 39 绿）；openai_sse 占位接线已 revert，调用点拆 task-7（需真实 parsed_bytes/elapsed + tool_call 签名断言先行）。
- 写锁纪律：Executor 越界一次（control_plane 顺手改）已 revert；并行 workspace 主线改动未混入本轮提交。
