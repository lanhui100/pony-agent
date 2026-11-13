# Agent Note: 上游 HTTP 525 由「永久不可重试」改判为可重试（边缘 5xx ③′ 组收编）

Status: implemented

## Problem

会话 `pa_01a1cf14-3789-7660-b73a-992ff56e1c34` 六轮中三轮失败，全部同一根因：
上游网关 `https://tokens.ponyjob.top/v1/chat/completions` 返回 **HTTP 525（SSL Handshake Failed）**，
响应正文为空，耗时 6–8s。

| turn | 模型 | 结局 | 触发点 |
|---|---|---|---|
| turn-d51afe98 | antigravity/gemini-3.8-flash | failed | 5 次工具调用后回填，525 @8036ms |
| turn-c38cb0b1 | antigravity/gemini-3.8-flash | failed | 3 次工具调用后回填，525 @5964ms |
| turn-2cd915a8 | workbuddy/hy4free | failed | 4 次工具调用后回填，525 @6068ms |

三次失败一律 `decision=Abort { reason: NonRetryable }`：**5 次退避预算从未启动**。

全量日志（`~/.pony-agent/tui-logs/tui-runtime.log`）中 525 的分布表明这不是单一路径的偶发：

| 上游路径 | 525 命中行数 | `scope` |
|---|---:|---|
| `decision_stream` | 3 | Decision |
| `decision` | 3 | Decision |
| `followup_stream` | 11 | Followup |
| `followup_sync_batch` | 5 | Followup |
| `followup_sync_fallback` | 1 | Followup |

即 **Decision 首跳与三条 Followup 路径都会命中**，全部走 `Abort`；其中 `attempt=2/5` 的行
（如 `decision_stream:attempt=2/5`）不是「重试生效」，而是同一跳内不同请求各自计数的独立失败行，
行内仍是 `NonRetryable` 立即 Abort。

根因是 `crates/pony-agent-core/src/agent/retry.rs` 的 ③′ 码表
`UPSTREAM_EDGE_SERVER_ERROR_HINTS`（520/521/522/523/524/527/530）刻意排除 525/526，
理由来自归档 ADR
`.agents/notes/archived/architecture/2026-11-07-provider-transport-and-edge-5xx-retry-classification.md`
决策 2：「源站 TLS 握手失败 / 证书无效属配置错误，重试不会改变结果」。

该理由对 526（`Invalid SSL Certificate`，证书无效/过期/不受信）成立，但对 **525 不成立**：

1. 同一归档 ADR 的实测表里 525 已出现 **12 次**，与 522（36 次）同量级，说明它是**持续性抖动**而非一次性配置事故；
2. 525 的 Cloudflare 语义是「边缘无法与源站完成 TLS 握手」，成因含源站重启、负载抖动、握手超时——与 522「边缘连不上源站」同属
   「边缘没能完成交换」，秒级可自愈；
3. 本次实证跨 gemini / hy4free 两个模型、三个 turn 连撞，而同一网关后续轮次恢复，佐证瞬时性。

连带缺陷（同一分类结论的下游）：

- **不换手**：`failover_eligible` 只对 `server_error` 码 + `rate_limited/quota/timeout/network/auth` 生效，
  525 落 `None` ⇒ 既不在码表也不在换手集合，永不切换 provider；
- **不冷却**：只有 rate-limit/quota 写 `provider_cooldown_entered`，525 无冷却 ⇒ 下一轮仍打同一网关，形成「连撞三次」。

## Decision

1. **525 纳入 ③′**：`UPSTREAM_EDGE_SERVER_ERROR_HINTS` 扩为
   `["520","521","522","523","524","525","527","530"]`，映射 `ErrorCode::UpstreamServerError`（`TransientRetryable`），
   与 522 同组同结论，进入既有 `provider_timeout_backoff_config_for(policy)`
   （500→1000→2000→4000ms，cap 8s，预算 30s，最多 5 次）。
2. **526 维持排除**：`Invalid SSL Certificate` 是确定性配置错误，重试与换到同源站的其他模型都不会改变结果，
   保持 `NonRetryable → Abort`；不为让退避「看起来有效」而吞掉配置类故障。500 亦维持既有冻结。
3. **换手随之放开（自动推导，不新增分支）**：`failover_eligible` 判 `upstream_code_for_hints == UpstreamServerError`，
   525 进码表即自动可换手，无需改 `failover_eligible` 本体；`MAX_FAILOVER_HOPS` 与候选冷却语义不变。
4. **词边界判据不变**：沿用 `hints_match`（数字码前后不得紧邻字母数字），
   `token id 1525` / `elapsed=5250ms` 均不命中，负样本已在单测中钉死。
5. **不改动的既有语义**：`decide` 的流安全闸门（已发可见 delta 即禁止自动重试）、退避参数与 jitter、
   `RATE_LIMIT_LONG_BACKOFF_HINTS`、429/408/500/400/401/403/context-overflow 分组全部冻结。
6. **机械收口**：`agent::retry::tests::edge_5xx_maps_to_server_error_but_origin_tls_stays_abort`
   （525 转正例、526 转负例、补 `1525`/`5250ms` 子串负样本）、
   `adr_0086_regression_classify_retryability`（525 进可重试组、526 进 Abort 组）、
   `tests/pa172_provider_failover_contract.rs::failover_trigger_class_truth_table_is_closed`
   （525 进 eligible、526 进 ineligible）三处真值表同步。
7. **行为层收口（新增，码表归属不足以证明生效）**：`agent::provider::tests` 增两条端到端用例——
   `http_525_enters_retry_loop_and_succeeds_on_second_attempt`（Decision 与 Followup 两个 scope 各跑一遍：
   首试 525 → 必须 sleep 500ms → 第二次请求成功，`attempts_total=2` / `slept_ms=500` / 不切长退避）与
   `http_526_aborts_without_retry`（首跳 Abort，`attempts_total=1` / `slept_ms=0`）。
   这两条走的是生产同一条 `retry_provider_scoped_with_report` 内核，而不是只断言 `classify().is_retryable()`。

## Alternatives considered

- **525 与 526 一起放开（取消排除）**：拒绝——526 是证书无效，属确定性配置错误，重试只会把
  「一次快速失败」拖成「5 次预算 + 换手后的 5 次预算」，且换到同源站的其他模型同样失败；与 ADR 0086 决策 2 的成立部分冲突。
- **只给 525 加重试、不放开换手**：拒绝——525 是网关级（边缘↔源站）故障，同网关换模型毫无收益；
  本次实证恰好跨模型连撞。放开换手是「同一网关整体不可用」时的唯一有效出路，且推导路径零代码。
- **给 525 单独登记 provider 冷却（像 rate-limit 那样）**：拒绝——冷却的语义是「已知窗口内不要打」，
  而 525 没有 `Retry-After` 也无可解析恢复时间，只能拍一个窗口；对瞬时抖动而言，
  短退避（500ms→8s）比固定冷却更快恢复，且冷却会让**健康**候选也被跳过。留作后续若出现长时 525 风暴再评估。
- **把 5xx 全量放开（500–599）**：拒绝——501/505/508/511 属永久性协议错误，526 亦在其列；通配化会让
  「配置故障」伪装成「暂时不可用」，与 ADR 0086 候选方案 A 的落选理由同源。
- **改 `classify` 增加「边缘码 + 源站握手措辞」二级判据**：拒绝——引入第二套关键词表即违反 R1「单一注册表」约束，
  且 525 报文正文常为空（本次三例均无 body），措辞判据在真实报文中不可靠。

## Consequences

- `crates/pony-agent-core/src/agent/retry.rs`：③′ 码表加 `"525"`；③′ 注释由「525/526 排除」改写为
  「525 纳入（实证驱动）/ 526 仍排除」并链入本条；`failover_eligible` 文档注释的码段由 `520-524/527/530` 改为 `520-525/527/530`。
- 行为变更：525 由「一次 Abort」变为「最多 5 次退避 + 可换手」。最坏情形下单个 turn 在 525 上多消耗
  约 30s 预算（退避 500+1000+2000+4000ms 与预算上限取小），换手后再计一份；这是把「必然失败」换成「可能自愈」的代价。
- 已验证：`cargo test -p pony-agent-core --lib agent::retry::tests` 52 passed / 0 failed；
  `cargo test -p pony-agent-core --test pa172_provider_failover_contract` 11 passed / 0 failed；
  `cargo test -p pony-agent-core --lib http_52` 2 passed / 0 failed（新增的两条行为用例）。
- 观测归属：本次只改失败分类判据，不新增对外接口、网络 IO、后台任务或 LLM 调用；
  `docs/observability.yaml` 追加一条归属登记（四维不变，metrics/logs 结构性 N-A）。
- **仍未修的已知缺口**（本轮不做，与 525 分类无关）：turn 级 trace 行缺失导致
  `observation ref backfill skipped: no trace row`；`turn.cancelled` 事件重复上报。
- 诚实边界：525 是否真能在 500ms–8s 量级自愈，取决于源站重启/负载抖动的实际时长；若生产出现
  「5 次打满仍 525」的稳定模式，应按备选 3 重新评估固定冷却窗口，而非继续拉长退避。
