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
- 后续映射 Stage 应坚持"单文件簇+单测全绿再进下一项"（1双写→2retry→3provider），每 Stage 配 Test Agent 断言先行。
