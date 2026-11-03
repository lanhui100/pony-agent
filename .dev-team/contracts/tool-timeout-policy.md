# 契约矩阵：Run 工具超时策略（tool-timeout-policy）· v2

> 依据：桌面端 trace `pa_01a1a134` 诊断 P0-2 + DSH 超时设计（`/home/dm/deepseek-harness` @ de5d733a18）。
> v2 修订（Lead 裁决，2026-11-04）：撤销原 CT-6（指纹区分 timeoutMs），CT-5（hint 文案）延后波次二。
> 修订依据：侦察证实现有冻结验收测试 `wave9_adversarial_test.rs:55/:255`、`harness_optimizations_test.rs:215`
> 明确要求「仅 timeoutMs 不同 ⇒ 指纹相同」，且这是刻意的反重试风暴设计（模型不得靠加大 timeout 绕过熔断）。

## 波次一（本波实施）

| ID | 契约 | 验收方式（机器） |
| :--- | :--- | :--- |
| CT-1 | `DEFAULT_RUN_TIMEOUT_MS = 120_000`、`MAX_RUN_TIMEOUT_MS = 600_000` | 单测断言两常量 |
| CT-2 | 删除子串启发式（`tools.rs:2862-2877`）：未传 `timeoutMs` 时任何命令一律 `DEFAULT_RUN_TIMEOUT_MS` | `resolve_run_timeout_ms({},"cargo build") == 120_000`；`({},"echo hi") == 120_000`；`({},"git checkout x") == 120_000` |
| CT-3 | 显式 `timeoutMs` 夹紧：`<1→1`、`>600_000→600_000`、非数字/缺失→120_000 | 三条断言 |
| CT-4 | 工具结果 evidence：超时路径 `timedOut: true`；成功路径 `timedOut: false` + 生效 `timeoutMs`；既有键不删不改名 | JSON 键断言 |

可测接缝：`pub fn resolve_run_timeout_ms(arguments: &serde_json::Value, command: &str) -> u64`（`agent/tools.rs`）。

## 波次二（本波不做，待排期）

| ID | 契约 | 阻塞原因 |
| :--- | :--- | :--- |
| CT-5 | 超时 hint 增加「后台执行 job_start」与「显式更大 timeoutMs（上限 600000）」 | 撞 `wave9_green_acceptance_test.rs:143` 逐字断言 → 需契约演进 + 冻结测试迁移提交 |
| CT-6′ | 真·promote（超时把运行中命令转后台 job） | 侦察结论：Run 进程所有权在 `ProcessManager`（`process.rs:306`）、`poll` 为「取走即清空」（`:869-872`）、`kill_after` 无 disarm API（`:433-443`）、取消 watcher 随函数返回 drop；需 jobs.rs 适配层 + 生命周期重构，非局部补丁 |

## 非目标

不改 `job_*` 工具语义、不改错误码 `timeout`/`non_zero_exit`、不改前端渲染、不动 `ProcessBackend` trait。

## 风险

- 默认 10s→120s：明显挂死的短命令等待变长，由 turn 墙钟预算（`runtime/limits.rs:28` 900s）兜底。
- `tools.rs:11257` 等 src 内单测若依赖旧默认值，由 executor 同步修正（实现层单测，非冻结验收层）。

## 验收门禁

1. `cargo test -p pony-agent-core --test tool_timeout_policy_test` 全绿。
2. `cargo test -p pony-agent-core` 无新增失败（对照基线回执）。
3. `cargo clippy -p pony-agent-core` 无新增 warning（改动文件范围）。
4. L3 子代理复核 diff：无夹带重构、无改写冻结验收测试。
