# Usage Governance v1 系统架构蓝图

> 本文件是本子系统唯一系统蓝图（planning 层）。`openspec` design.md 只承载单个 backlog 事项的内部细化，不替代本蓝图。

**状态**: approved
**日期**: 2026-10-25
**架构负责人**: Lead（dev-team Phase 0 参谋团三视角合议：产品/架构/工程风险）

交付形态：桌面（Tauri）/ CLI / TUI 三宿主共用 `pony-agent-core`。关联 ADR：[0048](../decisions/0048-usage-governance-enforcement.md)（硬限流口径收窄，部分 supersede `wave-5-cost.md` K3.1）。

---

## 1. 目标与产品边界

### 1.1 目标（v1 必须交付）
1. **看得见的账**：会话累计成本为主线真源；按天/按模型为派生视图；持久化、重启不丢、重放不重。
2. **会话级刹车**：软告警（默认开，已有）+ 硬限流（**opt-in，默认关**）；只拦「新 turn 发起」，绝不打断进行中的 turn。
3. **未计价透明**：`cost=None` 的轮次与计价轮次并列呈现（`$X + N 轮未计价`），聚合不静默按 0 求和。
4. **失败即放行**：策略解析/账本读取异常 ⇒ 放行 + 降级标记 + 去重 issue（fail-open）。

### 1.2 Non-Goals（结构性理由，不是"以后再说"）
| 非目标 | 结构性理由 |
| :--- | :--- |
| 团队/多席位共享配额 | 桌面端无身份体系与同步通道，做出来只能是伪团队 |
| 与上游账单对账/发票级精度 | 缓存计费、批处理、退款使自算永不齐；承诺对账即失信 |
| 自动切模型/自动降级省钱 | 隐式改变输出质量、不可预期，属策略层 v2 |
| QPS/并发/令牌桶限流 | 单用户桌面瓶颈是花费而非吞吐 |
| 自定义单价录入 | 会让"成本"变成用户输入，口径不可审计 |
| 跨设备/跨项目汇总 | 无同步通道 |
| 伪造 `/metrics` 或计数器 | 交付形态无常驻 scrape 面；为让巡检变绿造假被明令拒绝 |

---

## 2. 系统拓扑与模块契约

### 2.1 数据流
```
turn 结束（turn_flow / turn_persist）
   └─ 同步事务内 append usage_ledger（失败重试一次 + 落 spool，绝不阻塞 turn 返回）
turn 启动（turn_stream::prepare_turn 之前）
   └─ governance::decide(policy, ledger_snapshot) -> Allow | Warn | Block{reason}
        ├─ Allow  → 正常执行
        ├─ Warn   → 正常执行 + 结构化日志 + UI 角标
        ├─ Block  → 复用既有 early-return 失败通道，文案含"当前值/上限/一键出口"
        └─ 异常   → Allow + governance_degraded=true + warn + 去重 issue
查询（Tauri command）
   └─ usage_governance_summary(window)（扩展既有 load_model_monitor_summary，加 cost/day 维度）
```

### 2.2 模块契约（v1 冻结）
| 名称 | 形态 | 语义 |
| :--- | :--- | :--- |
| `UsagePolicy` | `{ window: "session"\|"day", soft_ratio?: f64, hard_limit_usd?: f64, enforcement: bool }` | `enforcement=false` ⇒ 永不 Block |
| `DecideInput` | `{ session_id, day_key, policy, snapshot: { session_cost, day_cost, unpriced_turns } }` | 纯数据，可单测/回放 |
| `Decision` | `Allow` / `Warn{ratio}` / `Block{reason,current,limit}` | 单调：usage 增大不得 Block→Allow |
| `usage_governance_summary` | `{ session, days[], unpriced_turns }` | 只读；缺数据返回空而非错误 |

### 2.3 分层约束
- 配额判定**必须在 core**（三宿主直连 core：`src-tauri/src/lib.rs`、`bin/tui.rs`、`bin/cli.rs`）；只挡 Tauri 等于没挡。
- core 仍**不持有内存会话累计**；唯一真源在 DB，判定为「一次无状态 SELECT SUM」。该收窄见 ADR 0048。

---

## 3. 核心数据模型与状态流转

### 3.1 账本表 `usage_ledger`（append-only，纯增量迁移）
| 列 | 语义 |
| :--- | :--- |
| `session_id` / `turn_id` | 自然键，`UNIQUE(session_id, turn_id)`（turn_id 非全局唯一） |
| `recorded_at_ms` | **首写冻结**的 UTC 毫秒（不用 trace.updated_at，会被重写） |
| `day_key` | 首写冻结的本地日历日 `YYYY-MM-DD`（按 turn **结束**时刻归属） |
| `provider_name` / `model` / `protocol` | 原串保留，不归一（计价表为精确匹配） |
| `input_tokens` / `output_tokens` / `cache_read_tokens` | provider 实际值，缺失为 NULL |
| `cost_usd` | **可 NULL**；未知模型必须 NULL，严禁存 0 |
| `priced` | `0/1` 冗余标志，便于统计未计价轮次 |

**幂等**：`INSERT ... ON CONFLICT(session_id,turn_id) DO UPDATE`；窗口累计是 `SELECT SUM` 而非计数器 ⇒ 重启不丢、重放不重。

### 3.2 口径规则（冲突时以此为准）
1. 跨天：按每轮结束时刻归入本地日历日，不整体归起始日。
2. 未计价：`SUM(COALESCE(cost_usd,0))` 只是"已知成本下限"；UI 必须并列 `unpriced_turns` 并加 `≥` 前缀，全未计价显示 `—`。
3. 阈值：`usage >= threshold` 即触及（与 `cost::is_budget_exceeded` 统一）；`None`/`<=0` 视为未设置。
4. 协议感知：账本只认 `estimate_cost_with_protocol`；`turn_persist.rs` 的 OpenAI 口径为已知缺陷（单开工单，账本不复用）。

### 3.3 决策状态机
`Allow --usage↑--> Warn --usage↑--> Block`（单调，不可逆）；`enforcement=false` 时 `Warn/Block` 一律降为 `Allow`（但保留 `would_block` 诊断字段供 UI 提示）。

---

## 4. 关键技术选型与 ADR

| 议题 | 选型 | 理由 |
| :--- | :--- | :--- |
| 账本落点 | 纯 SQLite append-only 表 | trace blob 每会话仅留 24 条（回溯性缩小）；trace 异步队列明示"允许丢失"；内存聚合重启即丢 |
| 判定位置 | core turn 入口 | 三宿主共用，前端可绕过；避免"只挡桌面" |
| 失败策略 | fail-open（放行 + 降级标记） | 单用户桌面，误伤＝应用像坏了且无补偿面；放行有账本可对账 |
| 迁移方式 | `CREATE TABLE IF NOT EXISTS` + 新索引；不 bump schema 版本 | 旧二进制忽略新表 ⇒ 回滚安全；bump 会让旧库打不开 |
| 聚合复用 | 扩展既有 `load_model_monitor_summary` | 避免第二套聚合视图分叉 |

**ADR**：[0048-usage-governance-enforcement.md](../decisions/0048-usage-governance-enforcement.md)（硬限流口径、fail-open、supersede K3.1 的"绝不阻断"部分）。

---

## 5. 任务分解映射

| Backlog | 内容 | 依赖 | 档位 | 验收判据（机器可判） |
| :--- | :--- | :--- | :--- | :--- |
| B001 | 账本表 + append + 窗口聚合查询 | — | L2 | 幂等（连写 3 次=1 行且时间冻结）；未知模型 `cost IS NULL AND priced=0`；重开库 SUM 一致 |
| B002 | 策略纯函数 + settings 契约（Rust/TS） | — | L1 + L2-AT | 黄金表三态；单调性；阈值 `>=`；`enforcement=false` 恒 Allow；NaN/负值拒绝 |
| B003 | 决策接入 turn 入口 + fail-open + 查询 IPC | B001,B002 | L2 + L2-AT | 三态快照；DB 不可读 ⇒ Allow + `governance_degraded` + 去重 issue；命令 JSON schema 快照 |
| B004 | 只读用量面板 + 拦截提示 UI | B003 | L2（出拦截 UI 升 L3 浏览器） | 面板数字=手工按轮汇总（含未计价轮数一致）；无 NaN/Infinity 文本 |

**过程门禁**：每项独立红相锚定 → 实施 → L3 审查 → 合并；`cargo test -p pony-agent-core --lib` 失败数恒为 1（既有 `config.rs:1616`）；`.dev-team/evidence/wave15/` 测试后零改动。
