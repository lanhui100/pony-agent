# Wave-7 可观测性链路复审（新项目上线后，真实轮次驱动）

- 复审时间：2026-10-25（Asia/Shanghai）
- 触发：中等规模新项目 **Usage Governance v1** 落地（账本 + 策略引擎 + 三宿主强制 + 前端面板）
- 方法：**真实 CLI 轮次**（隔离 HOME、真实公网 token）驱动「正常轮 → 受治理阻断轮」两条路径，回查 Tempo / 本地日志 / PonySentry

---

## 0. 结论

新子系统接入后，**链路仍完整且有新增信号**，无新增断链。本轮唯一需要跨仓推进的仍是**上一轮已定位的 PonySentry 服务端缺口**（ingest 丢弃 `trace_id`）。

---

## 1. 本轮新子系统产生的可观测信号（真实收据）

### 1.1 治理阻断轮 —— 本地结构化日志（新）
```
{"kind":"governance_block","level":"warn","message":"usage governance block (new turn rejected)",
 "policy_version":"cfc7b29a7d29","reason":"hard_limit_reached",
 "trace_id":"267b2afe87c8b4e3468584ac9cc0745b","ts":"2026-10-25T08:44:06.026Z","window":"day"}
```
- 单行 JSON、字段结构化（`kind/reason/window/policy_version`）、**`trace_id` 非 null**（Wave-4/5 修复持续生效）；
- 金额与阈值**未**写入日志（符合 G5 隐私条款）。

### 1.2 同一 trace 在 Tempo 的形态（物理验证）
```
GET /api/traces/267b2afe87c8b4e3468584ac9cc0745b
spans: ['governance.decide', 'agent.turn']
governance attrs: [decision, policy_version, reason, unpriced_turns, window]   ← 恰好 5 项白名单
```
- **日志 `trace_id` == Tempo trace_id** ⇒ Loki↔Tempo 互跳桥在真实路径可用（桌面端日志面同理）；
- `governance.decide` 与 `agent.turn` 同 trace 且**只有白名单属性**（无 `limit_usd/current_usd`，无金额泄漏）。

### 1.3 用户可见阻断文案（产品验收判据）
```
✘ [Turn Failed] error=Some("用量治理已阻断本轮：当前已用 $1.000000，上限 $0.000001
（触发 hard_limit_reached）。如需继续，请提高上限或关闭「强制拦截」——
设置 → 用量治理 → 关闭 enforcement（一键出口）。")
```
走既有 `turn:failed` 通道（进程 exit=0），含「当前值 / 上限 / 一键出口」三要素。

### 1.4 账本落盘（CLI 形态）
`<cli-state>/usage-ledger.sqlite` 真实生成；预置 priced 行 + 未计价行共存时，`SUM(COALESCE(cost_usd,0))` 与 `unpriced_turns` 分离统计（阻断判定读到 `1.000000`）。

---

## 2. 复审逐维

| 维度 | 状态 | 证据 |
| :--- | :--- | :--- |
| tracing（CLI 形态） | ✅ CONNECTED | 阻断轮 trace 落 Tempo；正常轮亦落（`rootTraceName=agent.turn`） |
| tracing（桌面形态） | ✅ CONNECTED（沿用 Wave-4/5 收据） | `client.service.name=pony-agent`、`ingest.source=public` |
| 日志 → 本地取证面 | ✅ CONNECTED | 单行 JSON + 真实 trace_id；512KiB 轮转 |
| 日志 → Loki | NA（结构性） | 桌面/CLI 不在容器内，声明已覆盖 |
| issue | ⚠️ 客户端 CONNECTED / 服务端缺口 | 客户端发 `event_id`/`trace_id`/`tags.project`；**ingest 丢弃 `trace_id`**（上轮 curl 隔离实验已证） |
| metrics | NA（结构性） | 无 scrape 面；**未**为巡检造假指标 |
| 新增治理信号 | ✅ 合规 | 1 个 `governance.decide` span + 结构化日志 + 去重 issue；无假计数器 |

---

## 3. 本轮新发现（均已在本轮修掉）

| # | 问题 | 证据 | 处置 |
| :--- | :--- | :--- | :--- |
| N1 | **CLI 形态 enforcement 可绕过**：CLI 用 `FileSessionBackend`（JSON 会话库），账本走 trait 默认实现恒零 ⇒ 阻断永不触发 | 首测：预置 `$1.0` 账本 + `hardLimitUsd=1e-6` 仍未被拦 | `FileSessionBackend` 落同 schema 的 SQLite 账本侧车 `<session_dir>/usage-ledger.sqlite`，复用同一 SQL/公式（无第二份真值） |
| N2 | **同步入口未加治理判定**：`turn_sync` 路径无判定 ⇒ 可绕过 | 代码审查 | 同构接入同一决策入口，Block 走既有 `build_failed_turn_result` |
| N3 | **本地日志文件延迟创建**：仅在首次 warn 时落盘，正常轮看不到日志文件（易误判"日志没生效"） | 首测：正常轮后目录为空 | 记录为**已知行为**（懒创建），在复审报告中显式说明，避免下次误判为断链 |

---

## 4. 仍待跨仓推进（与 Wave-6 结论一致，未变化）

**PonySentry ingest 不持久化 `trace_id`**：客户端已完整发送（本地 sink 逐字节 + 公网 201 created），回读事件 payload 无该键 ⇒ `Issue ↔ Tempo` 互跳的最后一环在服务端 schema。建议 infra PR：ingest 白名单增 `trace_id`（32hex 校验）→ 入库 → API 暴露。

---

## 5. 复审方法（可复现）

```bash
# 隔离 HOME，开启治理强制拦截
TMPH=$(mktemp -d); mkdir -p $TMPH/.config/pony-agent
# settings.json 需用 camelCase 嵌套键（踩坑点，务必照抄）：
#   "usageGovernancePolicy": {"window":"day","softRatio":0.5,"hardLimitUsd":0.000001,"enforcement":true}
HOME=$TMPH XDG_CONFIG_HOME=$TMPH/.config XDG_STATE_HOME=$TMPH/.state \
PONYSENTRY_CLIENT_TOKEN=<cluster> PONYSENTRY_INGEST_URL=https://sentry.ponyjob.top \
PONY_OTEL_TOKEN=<otel-token> OTEL_EXPORTER_OTLP_ENDPOINT=https://otel.ponyjob.top \
./target/debug/ponycli --gateway http://127.0.0.1:1 --api-key dummy --prompt "gov-blocked"

# 回查 Tempo（trace_id 取自本地日志行）
in_probe "wget -qO- 'http://tempo.observability.svc.cluster.local:3200/api/traces/<trace_id>'"
```

> ⚠️ 踩坑记录：settings 里治理字段是 **camelCase 嵌套**（`hardLimitUsd` / `softRatio`），写成 snake_case 不会报错但会被 serde 忽略（`Option` 缺省）⇒ 表现为「enforcement 开了却没拦」。本轮首测即因此误判两次，已记入本报告避免重复。

---

## 6. 附：L3 审查判 FAIL 后的整改（诚实记录）

Wave-7 首次 L3 审查 **FAIL（3 BLOCKING / 8 ADVISORY）**，说明"链路通"不等于"功能对"。三项 BLOCKING 与处置：

| # | 缺陷 | 后果 | 处置 |
| :--- | :--- | :--- | :--- |
| B1 | **设置契约断裂**：前端写 `usagePolicy` + snake_case（`hard_limit_usd`），Rust serde 只认 `usageGovernancePolicy` + `hardLimitUsd` | 配置页改了不落盘、`enforcement` **无法经 UI 开启** ⇒ ADR-0048 的 opt-in 硬限流整条路径不可达 | TS 改 camelCase 并对齐 Rust 键名；补 camelCase 往返测试 |
| B2 | **未计价被伪造成 0 成本**：`input_tokens.unwrap_or(0)` ⇒ 失败/取消轮写成 `priced=1, cost_usd=0.0` | 违反 G1.3；账本成为第二套语义（事件口径是"缺席"） | 抽 `build_usage_ledger_entry`：usage 任一缺失 ⇒ `unpriced`（NULL + priced=0） |
| B3 | **金额进 issue payload**：Block 文案含 `$current/$limit`，经 `emit_turn_failed → capture_turn_failure` 上报 | 用户财务数据泄漏到 PonySentry，违反 G5.3 | core 加 `mark_turn_failure_expected`，治理阻断**跳过** issue；前端 `reportError` 同步跳过 |

关键 ADVISORY 也一并修：`governance.unpriced_turns` 恒写 0（telemetry 说谎）→ 传真实值；`unpriced_turns` session+day 重复计数 → 按窗口取值；`day_key` 为 UTC 桶 → 改本地日历日（历史行冻结不重算）；CLI 侧车账本无 WAL/busy_timeout → 单连接 + WAL + 5000ms；`overrideGovernanceBlock` 空逃生舱 → 真实持久化关闭 enforcement；`ui-block.png` 原为启动 splash（非凭证）→ 重截真实拦截横幅。

**结论更新**：整改后 `cargo test --lib` 1149 passed / 1 既有红、vitest 1026 passed、三靶全绿；拦截横幅真实渲染（当前 $0.065 / 硬限额 $0.05 / 本次解除拦截），Console Error 0。

**复审教训**：可观测性链路的"通"与功能的"对"必须分开验收——本轮链路端到端全通（trace/日志/span 都对），但业务契约仍有 3 处开裂。
