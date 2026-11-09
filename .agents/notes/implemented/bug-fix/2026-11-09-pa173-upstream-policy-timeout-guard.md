# PA-173 上游调用策略三缺陷：钳制不落盘 / 下界抬到物理可达 / 批量 followup 补重试

Status: implemented

- 日期：2026-11-09
- 关联：ADR 0104「PA-173 修订」/ 本卡红相提交 `1697d296`
- 前序：PA-172（`docs/decisions/0104-...`，本卡修订其边界与钳制语义）
- 现场事故：TUI 会话 `pa_01a1ba70` 于 15:34 连续两轮 `elapsed=1001ms` 超时 + 「上游服务暂时不可用，重试已用尽。」

## 决定

1. **钳制不落盘**：`normalize_storage` 不再回写 `upstream_policy`；钳制只发生在生效策略解析
   `ProviderRegistryStore::upstream_policy()`，并保留 `upstream_policy_clamped` warn（field/provided/clamped_to/bound）。
   `load_view` / `save_view` / `save_view_without_env_sync` 一律按原值往返（`save_view(load_view())` 幂等）。
2. **下界抬到物理可达**：`UPSTREAM_REQUEST_TIMEOUT_MS_MIN` 1000→**15000**、
   `UPSTREAM_DECISION_MAX_ATTEMPTS_MIN` 1→**2**、`UPSTREAM_RETRY_BUDGET_MS_MIN` 1000→**5000**；
   `*_DEFAULT`(45000/5/30000) 与 `*_MAX` 不动。
3. **批量 followup 补齐重试**：`continue_with_tool_results` 的三协议臂统一用
   `retry_provider_followup_timeout_with_policy("followup_sync_batch", …)` 包装（`max_retries = attempts-1`，
   复用 `provider_timeout_backoff_config_for` 退避）；耗尽后原错误上抛，**不加**本地 fallback。
4. **重试不得污染转录**（本卡新增，非 1:1 照抄）：每次 attempt 起点把 `accumulated_messages`
   重置为进入批量 dispatch 时的 pristine 快照 ⇒ 成功 = base + 恰好 1 次追加；失败 = 停在最后一次写入后的转录
   （无回滚，调用方不应把它当干净 base 复用）。
5. **不改文案**：批量补齐重试后「重试已用尽」恢复为事实描述；attempts 下界 2 保证任何合法配置至少一次重试。

## 为什么

- 事故的物理证据：`providers.json` 的 `upstreamPolicy` = `{1000, 1, 1000}`（三个下界）。实测该网关
  2571/2585 请求返回 200、0 个 5xx、p50 2.2s；该模型上游 p50 4.67s、p99 36s；**2292 次真实调用里 0 次能在 1s 内完成**
  ⇒ 1s 预算 + 零重试使健康上游必然 100% 失败。故「上游故障」是伪因，「配置面可产生不可用组合」才是真因。
- 钳制值落盘把「一次越界输入」固化成「用户意图」：`config.rs` 自己注释写着「全 0 默认会被钳成 1000ms/1 次/1000ms（G2 崩）」，
  而实现恰恰在读写共用入口写回该值 —— 事故现场就是这个组合。
- 批量 followup 此前**无任何重试层**（1:1 有），首跳失败即终态，而 UI 仍显示「重试已用尽」——文案与事实相反。
- 加壳重试会新引入转录重复追加（构造器在每次尝试内写回），故必须同时重置 base，否则修一个问题造一个新问题。

## Alternatives considered

- **只把 `providers.json` 改回默认值**：能立即解封，但下界仍允许再次被钳到 1000ms/1 次，事故可复现，否决为唯一手段（只作现场急救，已单独执行）。
- **越界值改为报错（fail-closed）**：配置文件是用户可手改面，一处越界会让整个 provider 面失效（TUI 无法发起任何调用），
  与 ADR 0104 §2 的 fail-safe 目的相反，否决。
- **越界值回落到默认值（而不是钳到边界）**：同样不可用，但会丢掉「用户想调小/调大」的方向信息；
  抬高下界后钳制已不再产生不可用组合，故保留钳制语义更小闭包，否决回落方案。
- **在 `ProviderManager::with_policy` 里也钳制**：会让冻结测试直接构造的 `1000/1/1000` 被改，
  破坏 `pa172_provider_failover_contract` 的 F-c elapsed 有界断言，且重试器职责被污染，否决。
- **给批量路径加本地 fallback（对齐 1:1 的 `or_else`）**：批量无 fallback helper，且「伪造成功」会掩盖上游故障，
  与本卡「文案必须如实」的方向冲突，否决。
- **顺手修 1:1 的同源重复追加缺陷**：1:1 有 `.or_else` 兜底、行为不同，且在 PA-171/PA-172 冻结测试覆盖内，
  超出本卡最小闭包；登记为后续卡（代码注释已标注）。
- **改「重试已用尽」文案**：尝试次数在文案生产点不可得（`SingleHopRetryReport.attempts_total` 被非 report 包装丢弃），
  改文案需把计数上浮到 TUI 层，属另一条闭包；补齐重试后文案已恢复真实，否决。

## 落地与证据

- 红相锚定：`1697d296`（`pa173_upstream_policy_guard_contract.rs` 新增 4 红 + `pa172_..._config_contract.rs` 授权迁移；
  红相收据：D2 `left 1000/right 15000`、D1 `left (1000,1,1000)/right (0,0,0)`、D3 `left 1/right 2` 与 `left 1/right 5`）。
- 绿相（共享工作区）：`cargo check -p pony-agent-core --all-targets` exit 0；
  `pa173` 7 passed、`pa172_upstream_policy_config_contract` 8 passed、`pa172_provider_failover_contract` 11 passed、
  `pa172_budget_anchor_contract` 6 passed（全 exit 0）。
- 绿相（干净 lane `/tmp/pa173-baseline` @ `7d15a0f8` + 本卡 patch，排除他 lane 的 splash 未提交改动）：
  `pa173` 7、`pa172` config 8、`pa172` failover 11、`pa172` budget 6、`pa171_error_copy_contract` 89、
  `pa171_error_copy_adversarial` 89、`pa171_goal_round_failure_contract` 7、`pa171_provider_rate_limit_cooldown_contract` 5 —— 全 exit 0。
- **变异测试**（证明新断言有判别力）：临时注释掉三处 `*accumulated_messages = base_messages.clone();` ⇒
  `d3_batch_retry_does_not_duplicate_transcript` 失败 `left: 7 / right: 4`，exit 101；恢复后 sha256 与变异前一致、测试转绿。
- 门禁：`audit-diff.sh --base 7d15a0f8` exit 0；`check-declaration.sh` exit 0（PASS）；`npm run version:check` PASSED。
- 已知遗留（未在本卡修，已登记）：1:1 followup 重试会重复追加转录（同源缺陷，有 `.or_else` 兜底）。
