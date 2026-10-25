# Wave-7 冻结契约 — 用量治理子系统（Usage Governance v1）

本契约依据架构蓝图 `docs/architecture/system-design-usage-governance.md` 与 ADR-0048 制定，对用量账本、决策策略、Fail-Open 韧性、汇总查询及可观测性进行端到端契约冻结。

---

## 附：K3.1 Supersede 演进说明（依据 ADR-0048）

1. **核心不变量保全**：
   - 保留 `wave-5-cost.md` 中 K3.1 确立的「Rust core 运行时绝对不建立会话级内存累计状态」原则，所有历史用量与成本统一以 SQLite 持久化账本 `usage_ledger` 为唯一真源。
2. **阻断语义的显式收窄演进**：
   - 本契约对 `wave-5-cost.md` K3.1 中的“绝不阻断”语义依据 ADR-0048 进行演进收窄：
   - 默认状态（`enforcement = false`）下，**严格保持 100% 软告警绝不阻断**；
   - 仅当用户在策略中进行了**显式 opt-in**（`enforcement = true`）且达到硬限额时，治理决策引擎允许在**新 Turn 发起前**执行 `Block`（拒绝创建新 turn）；但在任何情况下，**绝不阻断或中断已在进行中的 Turn**。

---

## G1 账本 `usage_ledger`（Append-Only，纯增量迁移）

### 条款
1. **表结构定义**：
   ```sql
   CREATE TABLE IF NOT EXISTS usage_ledger (
       session_id TEXT NOT NULL,
       turn_id TEXT NOT NULL,
       recorded_at_ms INTEGER NOT NULL,
       day_key TEXT NOT NULL,
       provider_name TEXT NOT NULL,
       model TEXT NOT NULL,
       protocol TEXT NOT NULL,
       input_tokens INTEGER NOT NULL,
       output_tokens INTEGER NOT NULL,
       cache_read_tokens INTEGER NOT NULL,
       cost_usd REAL,
       priced INTEGER NOT NULL,
       PRIMARY KEY (session_id, turn_id)
   );
   CREATE INDEX IF NOT EXISTS idx_usage_ledger_day_key ON usage_ledger(day_key);
   CREATE INDEX IF NOT EXISTS idx_usage_ledger_session_id ON usage_ledger(session_id);
   ```
2. **幂等写入与列冻结规则**：
   - 主键/唯一约束：`UNIQUE(session_id, turn_id)`。
   - 写入语句规范：
     ```sql
     INSERT INTO usage_ledger (
         session_id, turn_id, recorded_at_ms, day_key, provider_name, model, protocol,
         input_tokens, output_tokens, cache_read_tokens, cost_usd, priced
     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(session_id, turn_id) DO UPDATE SET
         provider_name = excluded.provider_name,
         model = excluded.model,
         protocol = excluded.protocol,
         input_tokens = excluded.input_tokens,
         output_tokens = excluded.output_tokens,
         cache_read_tokens = excluded.cache_read_tokens,
         cost_usd = excluded.cost_usd,
         priced = excluded.priced;
     ```
   - **绝对冻结列**：`ON CONFLICT` 更新时，**严禁修改** `recorded_at_ms` 与 `day_key`，确保历史归档与天级别桶划分的时序绝对单调稳定。
3. **未知模型记录规范**：
   - 当模型未命中价格表时，必须显式写入 `cost_usd = NULL` 且 `priced = 0`。
   - **绝对严禁**将未知模型成本存入 `0.0`，杜绝把“未计价”伪造成“零成本免费”。
4. **增量迁移与版本不变量**：
   - 账本初始化采用 `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`。
   - **绝对禁止 bump** 既有全局 `EVENT_SCHEMA_VERSION`，保证纯增量平滑迁移。
5. **故障隔离**：
   - 写入失败时不得阻塞 turn 主流程；提供单次重试，重试依然失败则进入内存/磁盘 spool 待异步重试或记录告警。

### 验收判据
1. 幂等性：针对同一 `(session_id, turn_id)` 连续写入 3 次不同/相同载荷，表中行数恒等于 1，且首次写入的 `recorded_at_ms` 与 `day_key` 保持完全不变。
2. 数据持久性：关闭数据库连接后重新打开，对已计价记录执行 `SUM(cost_usd)` 与写入前计算值完全一致。
3. 未知模型语义：写入未知模型时，SQL 查询断言 `cost_usd IS NULL` 且 `priced = 0`。
4. 容灾隔离：账本写故障被捕获并静默吸收，turn 主流程成功返回。

---

## G2 策略与决策引擎（纯函数，无全局态）

### 条款
1. **数据结构契约**：
   ```rust
   #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
   pub struct UsagePolicy {
       pub window: PolicyWindow,          // "session" | "day"
       pub soft_ratio: Option<f64>,       // 软告警比例，例如 0.8
       pub hard_limit_usd: Option<f64>,   // 硬限额 USD
       pub enforcement: bool,             // 是否物理拦截，默认 false
   }

   #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
   pub struct UsageSnapshot {
       pub session_cost_usd: f64,
       pub day_cost_usd: f64,
       pub unpriced_turns: u64,
   }

   #[derive(Debug, Clone, PartialEq)]
   pub struct DecideInput<'a> {
       pub session_id: &'a str,
       pub day_key: &'a str,
       pub policy: &'a UsagePolicy,
       pub snapshot: &'a UsageSnapshot,
   }

   #[derive(Debug, Clone, PartialEq)]
   pub enum Decision {
       Allow { would_block: bool },
       Warn { ratio: f64, would_block: bool },
       Block { reason: String, current: f64, limit: f64, would_block: bool },
   }
   ```
2. **规则与阈值语义**：
   - **触及判断**：`usage >= threshold` 视为触及（含恰好等于）。
   - **未设置语义**：`soft_ratio` 或 `hard_limit_usd` 为 `None` 或数值 `≤ 0.0` 时视为未配置。
   - **默认配置**：`enforcement` 默认恒为 `false`。
   - **诊断字段 `would_block`**：当且仅当满足硬限额条件（`usage >= hard_limit_usd`）时为 `true`。若 `enforcement == false`，决策返回 `Warn` 或 `Allow`，但其 `would_block` 依然置为 `true`，供前端 UI 呈现“已超限但未开启拦截模式”的诊断提示。
   - **单调性约束**：在相同策略下，随着 snapshot 中用量成本增大，决策结果状态转移必须单调：`Allow -> Warn -> Block`，绝对禁止发生 `Block -> Allow` 的非预期回跳。
   - **脏数据防御**：若 policy 存在 `NaN`、`Infinity` 或负值字段，决策引擎判定策略非法，**安全降级为 `Allow { would_block: false }`** 并标记 `governance_degraded = true`。

### 验收判据（黄金测试表）
覆盖包含但不限于以下 8 组经典测试用例：
1. **未设置限制**：`hard_limit = None, soft_ratio = None` → `Allow { would_block: false }`。
2. **低于软阈值**：`limit = 10.0, soft = 0.8, cost = 7.0` → `Allow { would_block: false }`。
3. **恰好触及软阈值**：`limit = 10.0, soft = 0.8, cost = 8.0` → `Warn { ratio: 0.8, would_block: false }`。
4. **处于软硬之间**：`limit = 10.0, soft = 0.8, cost = 9.5` → `Warn { ratio: 0.95, would_block: false }`。
5. **触及硬限额但未开启强制 (enforcement=false)**：`limit = 10.0, cost = 10.0, enforcement = false` → `Warn { ratio: 1.0, would_block: true }`（不阻断）。
6. **远超硬限额但未开启强制 (enforcement=false)**：`limit = 10.0, cost = 15.0, enforcement = false` → `Warn { ratio: 1.5, would_block: true }`（不阻断）。
7. **触及硬限额且开启强制 (enforcement=true)**：`limit = 10.0, cost = 10.0, enforcement = true` → `Block { current: 10.0, limit: 10.0, would_block: true }`。
8. **脏数据/非法策略 (NaN/负值)**：`limit = NaN` → `Allow { would_block: false }`（degraded）。
9. **单调性穷举验证**：测试从 0.0 逐步递增 cost 至 20.0，决策状态机单调推进。

*脚注：`Warn { would_block: true }` 出现在 `enforcement = false` 且 `usage >= hard_limit` 时（如上述用例 #5 与 #6），语义属于向前端透出的“预告拦截”，**绝不阻断**当前与后续 turn 的执行。*

---

## G3 Fail-Open 韧性契约

### 条款
1. **静默放行原则（Fail-Open）**：
   - 当遇到 SQLite 账本不可读、文件锁竞争超时、策略 JSON 解析失败、用量聚合 SQL 异常等任何底层治理故障时：
   - 决策必须短路回退为 **`Decision::Allow { would_block: false }`**；
   - 显式设置返回标记 `governance_degraded = true`；
   - 输出结构化 warn 日志记录错误明细；
   - 通过 PonySentry 捕获并按 `(kind, policy_version)` 生成去重 fingerprint 的 Issue。
2. **零运行时中断**：
   - **绝对禁止**因为治理子系统故障导致 Turn 启动失败或请求中断。
   - **绝对禁止**在 Turn 执行中途（LLM 生成中、工具执行中）中断会话。

### 验收判据
1. 单元测试模拟 SQLite 数据库只读/文件损坏或断开连接，调用决策接口断言返回 `Decision::Allow` 且 `governance_degraded == true`。
2. 单测验证治理故障触发的 Issue 具有预期的 `(kind, policy_version)` 去重指纹。

---

## G4 汇总查询面 `usage_governance_summary`

### 条款
1. **接口返回形状契约**：
   ```typescript
   export interface UsageGovernanceSummary {
     session: {
       cost_usd: number;
       unpriced_turns: number;
     };
     days: Array<{
       day_key: string;       // "YYYY-MM-DD"
       cost_usd: number;
       unpriced_turns: number;
       turns: number;
     }>;
     policy_version: string;
     governance_degraded: boolean;
   }
   ```
2. **边界与只读约束**：
   - 纯只读查询，无写锁开销。
   - 若账本无任何记录，返回 `cost_usd = 0.0`、`days = []`，**禁止返回错误**。
   - 所有返回的浮点金额必须保留 6 位小数（符合 `cost_precision_decimals = 6`）。

### 验收判据
1. 空库查询返回合法的零值结构体，不抛异常。
2. 写入 2 条已计价与 1 条未计价记录，查询断言 `unpriced_turns == 1`，`cost_usd` 为 2 条已计价记录之和。

---

## G5 自身可观测性防线

### 条款
1. **Span 约束**：
   - 仅在宏观决策调度入口建立 span：`governance.decide`。
   - 属性白名单：
     - `governance.window`
     - `governance.decision`
     - `governance.reason`
     - `governance.policy_version`
     - `governance.unpriced_turns`
   - **严禁为内部无 IO 纯函数添加 span**（杜绝无意义 span 爆炸）。
2. **结构化日志**：
   - 仅在以下明确事件点触发结构化日志：账本 append 失败、表迁移异常、决策结果为 `warn` 或 `block`、fail-open 降级事件。
3. **PonySentry 故障 Issue 隔离**：
   - 仅用于上报治理子系统自身内部的致命故障（如 DB 损坏、迁移失败）；
   - `fingerprint` 必须按 `(kind, policy_version)` 严格计算收敛；
   - **严禁**把具体的动态数字（如金额、阈值、token 数）写入 issue payload / fingerprint（避免 issue 爆炸）。
4. **防熵红线**：
   - **绝对禁止**伪造 `/metrics` 导出接口。
   - **绝对禁止**新增无代码依据的虚假指标计数器。
   - **绝对禁止**引入不存在的 `enforcement_point`。

### 验收判据
1. 决策执行时导出的 `governance.decide` span 正确包含声明的 5 项标准属性。
2. 故障触发时，多次重复失败归入同一 Sentry Issue 指纹。
