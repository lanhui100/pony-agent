# ADR-0048: 用量治理硬限流的判定落点与 fail-open 策略

Status: implemented

- 日期：2026-10-25

## 背景

Wave-5 的契约（`.dev-team/contracts/wave-5-cost.md` §K3.1）明确规定：core 侧不建会话累计状态，超限只做软告警、**绝不阻断请求**。该裁决当时为的是避免与前端 store 形成双份会话状态。

Usage Governance v1 需要「硬限流」能力，但这与 K3.1 的"绝不阻断"直接冲突。同时架构勘察发现：配额判定若放在 Tauri 层，CLI/TUI 两个宿主可绕过（三宿主都直连 core）。

## 决策

1. **判定落点收敛到 core 的 turn 启动入口**（`turn_stream` 的 prepare 之前），三宿主一致生效。
2. **core 仍不持有内存会话累计**：唯一真源是 SQLite `usage_ledger`，判定为「一次无状态 SELECT SUM」，语义上仍是 stateless 读，不违背 K3.1 的初衷（避免双份状态）。
3. **硬限流为 opt-in**（`enforcement` 默认 false）；未开启时永不 Block，仅保留 `would_block` 诊断字段。
4. **fail-open**：策略解析失败、账本不可读、聚合异常 ⇒ 判为 Allow，并置 `governance_degraded=true`、写结构化 warn 日志、按 `(kind, policy_version)` 去重上报 issue。
5. **绝不中断进行中的 turn**：只在「新 turn 发起」时判定。

**对 K3.1 的收窄（部分 supersede）**：K3.1 中"无论单轮还是会话累计超限均为软告警，绝不阻断"一句，仅对**未开启 enforcement 的默认配置**继续成立；开启 enforcement 后允许对"新 turn 发起"阻断。K3.1 的「core 不建会话累计状态」保持不变。

## 理由

- **误伤比放行更贵**：单用户桌面场景下，误拦等于应用"发不出消息"，且没有客服/回滚通道；放行最坏是多花钱，而账本存在可事后对账（有补偿面）。
- **可绕过等于没做**：判定若留在 Tauri，CLI/TUI 直接绕过；收敛到 core 才有一致语义。
- **真源下移**：会话累计从"前端 store"迁移到 DB，避免重启丢失与多窗口不一致；同时保持 core 无内存累计，避免两套状态。

## 影响

- 新增 `usage_ledger` 表（纯增量迁移，不 bump `EVENT_SCHEMA_VERSION`，旧二进制可安全忽略）。
- `turn_stream` 启动路径新增一次只读查询（需保证在 DB 异常时 fail-open，不阻塞 turn）。
- `wave-5-cost.md` K3.1 需追加 supersede 注记；`docs/obs-config` 无关。
- 可观测性：新增 `governance.decide` span 与 `governance_degraded` 结构化日志；**不新增**任何指标计数器或 `/metrics`（该交付形态无常驻 scrape 面）。
- 已识别既有缺陷（本 ADR 不处理，单开工单）：`turn_persist.rs` 的 trace 成本用 OpenAI 口径，与协议感知口径可能同轮不等；账本只认协议感知口径。
