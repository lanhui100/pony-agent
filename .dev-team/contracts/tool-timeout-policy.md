# 契约矩阵：Run 工具超时策略（tool-timeout-policy）· v2

> 依据：桌面端 trace `pa_01a1a134` 诊断 P0-2 + DSH 超时设计（`/home/dm/deepseek-harness` @ de5d733a18）。
> v3 修订（Lead 裁决，2026-11-04）：新增 CT-5/CT-5a（hint 指向后台执行）与 S2 接缝，授权迁移 wave9 冻结断言。
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

## 波次二之先行项：CT-5（v3 新增，**已实施** commit `7675017`）

| ID | 契约 | 验收方式（机器） |
| :--- | :--- | :--- |
| CT-5 | `classify_timeout_hint` 两条分支都必须给出「后台执行」与「上限」两条出路：非网络分支文案为「长任务请改用后台执行 `job_start`（`job_output` 取增量输出、`job_kill` 终止，无超时上限）；或显式传入更大的 timeoutMs（上限 600000）；仍不足时缩短或拆分命令。」；网络分支保留既有网络自检四标记并**追加**同样的后台/上限建议 | 断言 hint 同时含 `job_start`、`job_output`、`600000`；网络分支另含 `网络/认证/代理`、`git ls-remote`、`git config --get remote.origin.url`、`SSH 身份自检` |
| CT-5a | 工具 schema 文案（`tools.rs:5864`）追加后台执行指引，前端副本 `src/lib/runtime/browser-preview.ts:35` 逐字同步 | 两处字符串相等且含 `job_start` |

**可测接缝（新增 S2）**：`pub fn classify_timeout_hint(command: &str) -> String`（`agent/tools.rs`）。

**冻结测试迁移（契约演进授权）**：`crates/pony-agent-core/tests/wave9_green_acceptance_test.rs:21` 的 `DEFAULT_TIMEOUT_HINT`
常量与 `:189` 的逐字断言由 **test-agent** 更新为新文案（Lead 授权，独立提交 `test(freeze)`），并追加 3 条标记断言。

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

## 波次三：CT-P6（promote 开关与兼容路径，v5 裁决）

| ID | 契约 | 验收 |
| :--- | :--- | :--- |
| CT-P6 | promote **默认开启**；开关为进程级 env `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT`，值 `0`/`false` 关闭。**不新增模型可见参数**（避免扩张工具面） | promote 测试默认路径断言 promoted；关闭时断言 error 路径 |
| CT-P6a | 关闭 promote 时，超时语义与波次一完全一致（CT-4 的 error 路径描述**仅在 promote 关闭时成立**）：`error.code=timeout`、无 `promoted`/`jobId`、进程被清理 | 既有 4 个测试文件的超时断言在文件级 pin `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=0` 后全绿 |
| CT-P6b | promote 开启时 `kill_after` 安全网延后 `PROMOTE_KILL_AFTER_GRACE_MS`（1000ms）并在到期时 disarm；关闭时行为不变 | `--lib agent::process` 全绿 + P5 通过 |
| CT-P6c | promote 路径不执行 `kill_if_exists`/`cancel`/`remove_windows_batch_script`；Windows 批处理脚本清理移交 job 生命周期（本波未实现，登记风险） | L3 审查 diff |

**冻结断言迁移（Lead 授权）**：`tool_timeout_policy_test.rs`（ct4 三条）、`harness_optimizations_test.rs:314`、`wave9_adversarial_test.rs:138`、
`wave9_green_acceptance_test.rs:99/:162` 需在**文件级** pin `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=0`（同值幂等，不受并行影响）；
`src/agent/tools.rs:11443` 单测同理由 executor 在用例内 pin。

## 波次四：CT-P7（L3 复核后的收口裁决，v6）

> L3-A/L3-B 双路复核 `8488d05` 均判 **FAIL**（3+3 BLOCKING）。裁决：**promote 改为默认关闭（opt-in）**，
> 在下列防线补齐前不得默认开启；本波同时修两条「低成本真缺陷」。

| ID | 契约 | 验收 |
| :--- | :--- | :--- |
| CT-P7 | `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT` **默认关闭**；`1`/`true`/`on`/`yes` 开启。未识别值 → 关闭（fail-safe） | 默认路径断言 error.code=timeout；显式开启断言 promoted |
| CT-P7a | 先 poll 再判超时：进程已退出（如 `echo hi` + `timeoutMs:1`）必须返回正常成功结果，不得被报成 promoted | 新增用例：已退出进程在极短超时下不得 promoted |
| CT-P7b | promoted 的 RingBuffer 溢出必须置 truncated 标记并透传到 `job_output`，禁止静默丢字节 | 用例：大输出 promoted 后 `stdoutTruncated=true` |

**未闭合的 BLOCKING（不得默认开启的理由，各自登记 backlog）**
- A-3：promoted 的 `ManagedProcess` 条目永不回收（每 run 泄漏 1 条目 + Windows Job 句柄）；`ProcessManager::shutdown` 零生产调用方 ⇒ 设计所称收口不可达。→ `B008`
- A-6：promote 后 `CommandCancelWatcher` 已 drop ⇒ 用户 Stop 停不下来（实测 cancel 后命令跑完）。→ `B009`
- B-1：promote 成功会摘掉 `error.code=timeout`，使两道连续失败熔断、超时指纹归一、非幂等盲重试防护同时失去输入。→ `B010`
- B-2：`timedOut/promoted` 无消费方，UI/trace 按 status 判成「成功完成」；`ok:true` + `exitCode:null` 破坏成功路径不变量。→ `B011`
- B-4：job 层无会话归属（跨会话 list/read/kill）；Windows `.cmd` 残留含命令原文。→ `B012`

## 波次五：CT-P9（B008–B013 闭合后的默认翻转，v7）

> 裁决：B008（条目回收）/ B009（取消接线 + 进程组收口）/ B010（熔断输入）/ B011（前端可见性）/
> B012（会话归属）/ B013（Windows `.cmd` 生命周期清理）全部绿相落地后，promote 的默认值**翻回开启**，
> 语义从「opt-in 预览」变为 **opt-out**：默认就是产品行为，需要旧语义的调用方显式关掉。

| ID | 契约 | 验收 |
| :--- | :--- | :--- |
| CT-P9 | `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT` **默认开启**（opt-out）：env **缺失** ⇒ 开启；`1`/`true`/`on`/`yes`（trim + 大小写不敏感）⇒ 开启；`0`/`false`/`off`/`no` ⇒ 关闭（回到 `error.code=timeout` 的波次一行为，CT-P6a）；**未识别值 ⇒ 回落默认「开启」并打一条 warn**（`[tools] unrecognized PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT=…; falling back to default (promote enabled)`），禁止「开关拼错却静默生效」 | 不设 env ⇒ `promoted=true` + `jobId`（`ct_p9_default_is_promote_on`）；显式 `0` ⇒ `error.code=timeout`（`ct_p4_*`） |

**已知风险（保留）**：Windows 真机 e2e 未验证 —— 本机为 Linux；Windows 侧仅保证
`<ws>/.tmp/pony-agent-run-{sid}.cmd` 的生命周期清理（B013）与 Job Object 语义未改，但真机路径未实跑。
