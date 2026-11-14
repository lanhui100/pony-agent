# B149-P0-T 测试设计要点（test-agent，只读预研产出）

状态：task-1（空桩冻结）in_progress，task-2 未 claim。真源 docs/architecture/acp-client-integration.md §3.2 / §4 P0 / §6。

## 冻结文件（P0 就绪后写）
- `crates/pony-agent-core/tests/acp_p0_acceptance.rs`：蓝军 7 项（round-trip / init 往返 / 入站请求 / 入站通知 / stderr 有界 / 相对 cwd 拒绝 / config 回落）
- `crates/pony-agent-core/tests/acp_p0_adversarial.rs`：L2-AT 3 类（畸形行阈值、超时、并发 id 不串包）
- `crates/pony-agent-core/tests/fixtures/acp_fake_agent.sh`：newline-delimited JSON-RPC fake agent 模板

## fake agent 方案
沿用 `tests/conformance.rs` 惯例：运行期内联生成脚本到 TempRoot + chmod 可执行位；env 注入 `ACP_FAKE_MALFORMED=N` / `ACP_FAKE_DELAY_MS=` / `ACP_FAKE_CRASH=1` / `ACP_FAKE_VERSION=2`。

## 确定性约束
- 畸形行：固定"连续 N 行非法"计数，阈值源 `.dev-team/nfr-baseline.json`（task-1 负责补 ACP 键）。
- 超时：`delay = baseline_timeout + margin`，同一真源取值；键缺失即用例 fail，禁写死兜底。
- 并发：固定 id 集合 + `std::sync::Barrier` + 固定轮次，禁 `sleep`/概率。

## 红线自检
无 `#[ignore]`、无空断言、无吞异常；红相原始输出落 `.dev-team/acp-p0-red.log`；只写 tests/ 与 .dev-team/。
