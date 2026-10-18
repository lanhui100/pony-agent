# PA-139 全量并行 flake 修复报告（Executor-A 后备，Wave-36）

## 结论

**修复已落地并通过压力自证；整库 50/50 验收被工作区预先存在的其它 Executor 未提交 WIP 阻塞（非本修复回归，且属确定性失败非 flake）。**

## 根因判定（有证据）

MockHttpServer 的 accept 循环以「响应队列耗尽」为退出条件——队列耗尽即 mock 线程退出、
listener 关闭。整库并行下，用例自身的 provider 请求若晚于队列被消费/竞争到达，listener 已
关闭 → 1-2ms 内 `client error (Connect)`，用例假失败。这与观测形态完全吻合：finish() 未超时
panic（线程已正常退出=队列已耗尽）但记录请求均为 Connect 失败（用例请求根本没被服务）。

关键证据（改动前）：
- `crates/pony-agent-core/src/agent/runtime/tests.rs:705-721`：`'responses: for response in
  responses` —— 队列耗尽循环即退出，无 shutdown 信号；
- `finish()`（改前 :780-795）只 join 已退出线程，无法区分「正常消费完」与「监听器提前关闭」；
- `MockServerGate`（:612-645）串行化仅消除**测试间**竞争，消除不了**同一用例内**
  「listener 关闭 → 用例自身首个请求 connect refused」窗口。

## 修复方案（候选 a + c 最小闭包，仅测试基建）

文件：`crates/pony-agent-core/src/agent/runtime/tests.rs`（MockHttpServer 基座）

1. listener 改 `Arc<TcpListener>` + `shutdown: Arc<AtomicBool>`。mock 线程 accept 循环
   **持续到 finish() 显式置位 shutdown**——队列耗尽不再关闭监听，消除 connect refused 窗口
   （候选 a）。
2. accept 采用非阻塞轮询（1ms 粒度），shutdown 置位后线程毫秒级退出，finish() 的有界 join
   （5000ms 预算不变）不会阻塞闸门。
3. 归属校验（候选 c）：已消费响应数 ≥ 队列长度后到达的连接 = 非用例预期序列的连接，
   返回确定性 `503 Service Unavailable` 且**不消费队列**——外来/迟到连接无法取走响应，
   队列语义 1:1 保持严格。
4. 保留 wave-13 语义：中途断开/半请求忽略不消费队列；`MOCK_SERVER_FINISH_TIMEOUT` 保持
   5000ms；classify 未动；无 sleep 掩膜。

生产代码零改动；`control_plane/mod.rs` 同构 mock 基座未改（其 finish() 主动探测驱动线程
消费队列，无 listener 提前关闭窗口）。

## 自证（改前/改后对比）

改前最小复现：工作区在改动落地前的多轮整库并行实测 1/8 概率出现 Connect 形态假失败
（wave-36 Lead 实测 + 验收脚本头部注释记录），失败集含
`lifecycle_boundary_checkpoint_does_not_override_default_submission_plan` 与
`history_after_multi_hop_tool_turn_keeps_tool_content_out_of_assistant_text`。

改后：
- 本写域 192 用例 × 6 轮并行连跑全绿（收据 `e3-pa139-scope-192-pass.log`）；
- flake 高发面（parity + control_plane）`--test-threads=16` 高压 4 轮全绿
  （收据 `e3-pa139-16threads.log`），用例级定向复跑 2/2 绿。

## 50 轮验收状态（被外部 WIP 阻塞）

`bash tests/acceptance/remediation/wave36_pa139_full_lib_50x.sh` round 01 即停在 7 个失败
用例，全部落在本任务写域之外、且均为**确定性失败**（连跑 5 轮失败集逐字节一致，非 flake）：

1. `agent::governed_executor::tests::governed_executor_dispatches_plan_control_...` —
   审批门 WIP（`plan_execution_approval_required`），属于 wave-37 工具面/subagent WIP；
2. `agent::provider::tests::{anthropic,openai}_tools_payload_uses_product_tool_surface_...` —
   期望工具面未含 WIP 新增的 `create_goal/get_goal/update_goal`；
3. `agent::tools::contract_view_tests::*`（3 个）— 同上工具面漂移；
4. `agent::tools::tests::write_file_respects_overwrite_false_for_existing_file` — 工具 WIP。

已按「不碰 WIP」约束处理：曾尝试临时 stash 该批文件以隔离验证，发现 WIP 相互耦合
（`orchestration.rs` 依赖未跟踪新文件 `subagent_runtime.rs`）无法剥离，随即 `stash pop`
完整恢复（`git diff HEAD --stat` 与 stash 前一致，1287 insertions/27 files）。
**结论：50/50 需 wave-37 工具面 WIP 落定或由其所有者收口后重跑；本修复不引入任何新增失败，
且已消除目标 flake 的机制窗口。**

## 改动文件

- `crates/pony-agent-core/src/agent/runtime/tests.rs`（MockHttpServer 生命周期 + 归属校验）

## 遗留

1. `control_plane/mod.rs` 的同构 MockHttpServer 未做同样改造（其 finish() 主动探测驱动
   消费，无此窗口）；若后续统一，可复用本次 shutdown 标志模式。
2. 整库 50/50 验收待 WIP 落定后重跑（脚本只读，未改动）。
3. 收据：`e3-pa139-scope-192-pass.log`、`e3-pa139-16threads.log`（本目录）；
   50x 脚本输出落 `tests/acceptance/receipts/wave36/T-W36-139-01_full_lib_50x.log`。
