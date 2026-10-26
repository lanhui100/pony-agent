# Wave-5 冻结契约 — 会话成本估算与预算软告警

本契约用于约束与指导 Wave-5 会话成本估算（Token 成本计算、数据结构、DTO 传递、OTel 属性映射与预算软告警）。

---

## K1 计价口径（唯一真源）

### 条款
1. **价格表规格**：价格表以**每 1M token 的 USD 单价**（1,000,000 tokens）精确表达，包含三维阶梯：
   - `input_per_mtok`: 未命中文档缓存的输入 token 单价（USD）；
   - `output_per_mtok`: 输出 token 单价（USD）；
   - `cache_read_per_mtok`: 命中 prompt/context cache 读取的 token 单价（USD）。
2. **计算公式**：
   - `input_tokens_uncached = max(input_tokens − cache_read_tokens, 0)`（严格剔除已走 cache 的 token，严禁重复计价）；
   - `input_cost_usd = (input_tokens_uncached × input_per_mtok + cache_read_tokens × cache_read_per_mtok) / 1_000_000`；
   - `output_cost_usd = (output_tokens × output_per_mtok) / 1_000_000`；
   - `total_cost_usd = input_cost_usd + output_cost_usd`。
3. **真实 Usage 先决**：与 `gen_ai.usage.*` 保持严格一致，成本计算**只允许**基于 provider 实际返回的真实 usage 字段（input/output/cache_read），禁止任何基于字数/字符的本地 token 猜测估算。
4. **精度与四舍五入**：所有计算出的 USD 成本值在最终输出前四舍五入保留 **6 位小数**（对应 NFR `cost_precision_decimals = 6`）。

### 边界与异常处理
1. **未知模型处理**：若模型未命中内置价格表（未知模型/新模型），整体**必须返回 `None`**（严禁采用默认兜底价、线性插值或凭空猜测）。
2. **异常数值防御**：若输入或计算过程中出现 `NaN`、`+Infinity`、`-Infinity` 或小于 `0.0` 的负值，整体一律返回 `None`。
3. **零 Token 边界**：当 `input_tokens == 0` 且 `output_tokens == 0` 时，若模型在价格表中，正常返回有限值 `0.000000`。

### 拒绝条件
1. 未命中价格表时返回任何估算值或硬编码非 None 值。
2. 将 `cache_read_tokens` 同时计入 `input_tokens_uncached` 导致双重计费。
3. 输出结果包含超过 6 位小数或非有限浮点数（`NaN`/`Inf`）。

### 验收判据
1. 单元测试覆盖已知主流模型（如 deepseek-chat、gpt-4o、claude-3-5-sonnet 等）的标准计费计算，断言结果精确到 6 位小数。
2. 单元测试覆盖未知模型调用，断言返回 `None`。
3. 单元测试覆盖极端负数/溢出/异常浮点输入，断言安全降级为 `None`。

---

## K2 数据结构与传输契约

### 条款
1. **核心计算结构**：
   ```rust
   #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
   pub struct CostEstimate {
       pub input_cost_usd: f64,
       pub output_cost_usd: f64,
       pub total_cost_usd: f64,
   }
   ```
2. **Turn 终态与事件字段扩展**：
   - `TurnStreamEvent`（流式终态事件）与 `TurnTraceRecord`（持久化与 IPC DTO）新增字段：
     ```rust
     #[serde(rename = "costUsd", skip_serializing_if = "Option::is_none")]
     pub cost_usd: Option<f64>,
     ```
   - 前端接收到的 JSON 属性名为 `costUsd`（camelCase）。
3. **前端消费与展示**：
   - 前端运行时 store 在 turn 结束时消费 `costUsd`，累加至当前会话的总消费金额 `sessionCostUsd`。
   - 若某轮 turn 或全会话无有效计费数据（`costUsd` 为 null/None），前端 UI 对应位置显示为占位符「`—`」。

### 边界
- 多轮交互累加过程中，若其中某轮为 `None`，不阻断其它轮次的聚合；前端明确区分「累计计费已知部分」与「包含未知未计价模型」的提示状态。

### 拒绝条件
1. 字段命名脱离 camelCase 契约（如输出下划线 `cost_usd` 到前端）。
2. 在 `cost_usd` 为 `None` 时输出 `0.0` 假象给前端。

### 验收判据
1. IPC 序列化断言：当 `cost_usd` 为 `Some(0.012345)` 时，JSON 包含 `"costUsd": 0.012345`；当为 `None` 时，JSON 键省略或显式为 null。
2. 前端单测断言：消费 `costUsd` 并正确驱动 UI 面板渲染，未知模型回显「`—`」。

---

## K3 Settings 预算语义（软告警不阻断）

### 条款
1. **配置字段声明**：
   - `settings.json` 与前端 Settings Store 显式新增字段：
     ```typescript
     costBudgetUsd?: number | null;
     ```
     （Rust 对应 `cost_budget_usd: Option<f64>`，serde 命名 `costBudgetUsd`）。
2. **有效性判断**：
   - 字段值为 `None`、`null` 或数值 `≤ 0.0` 时，统一视为**未设置预算**。
3. **软告警不阻断原则**（Non-Blocking Soft Warning）：
   - 当会话累计成本超出 `costBudgetUsd` 时：
     - **允许的操作**：在 UI 界面显式呈现醒目的软告警标识（如黄色告警角标/Tooltip 提示「已超预算限额」），并在核心运行时记录一条 `warn` 级别日志。
     - **严禁的操作**：**绝对禁止**拦截、拒绝执行用户的 Prompt 请求；**绝对禁止**禁用发送按钮或使输入框处于不可用状态。

### 边界
- 预算校验仅针对用户在本地设置的提示门槛，不构成任何物理或金融层面的截断。

### 拒绝条件
1. 会话成本超预算时导致任何请求失败或前端交互不可用。
2. 负数预算配置被判定为已启用预算。

### 验收判据
1. 当配置 `costBudgetUsd = 0.5` 且当前会话累计成本达到 `0.6` 时：
   - UI 正常展示预算超限告警标签；
   - 用户依然可以顺畅输入并发送下一个 Prompt；
   - 控制台或文件日志中能观察到单条超限 warn 日志。

---

## K4 OTel 成本属性映射

### 条款
1. **Span 属性规范**：
   - LLM 调用子 span（`llm.chat`）：属性键名为 `gen_ai.cost.usd`（`f64`，对应单次 LLM 调用的 `total_cost_usd`）。
   - Turn 根 span（`agent.turn`）：属性键名为 `agent.turn.cost.usd`（`f64`，整轮 turn 内所有模型调用的聚合成本）。
2. **守卫与缺失语义**：
   - 成本计算未命中或返回 `None` 时，**不得输出该属性键**（严禁输出 null 或 0.0）。
   - 沿用全局 OTel 隐私门禁（`GatedSampler`）：当 OTel 上报门禁关闭时，成本计算与属性附加均执行零分配短路，不产生任何计算与导出开销。

### 边界
- 工具调用（`tool.call`）span 不输出 `gen_ai.cost.usd`。

### 拒绝条件
1. 在无成本估算或未知模型时向 span 写入 `0.0` 伪造无成本。
2. 门禁关闭状态下触发昂贵的价格计算逻辑。

### 验收判据
1. Wire 级别单测断言：在 mock OTLP 导出时，`llm.chat` span 与 `agent.turn` span 携带正确的双精度浮点数属性。
2. 未知模型场景下断言导出 span 的 attributes 列表中不存在 `gen_ai.cost.usd` 与 `agent.turn.cost.usd`。

---

## K5 NFR 商业基准合规说明

已成功在 `.dev-team/nfr-baseline.json` 的 `observability` 对象内扩展追加：
```json
{
  "observability": {
    "exporter_flush_timeout_ms": 3000,
    "log_file_max_bytes": 524288,
    "setting_precedence": [
      "settings",
      "env",
      "default"
    ],
    "cost_precision_decimals": 6,
    "cost_budget_semantics": "soft_warn"
  }
}
```
该变更完全兼容既有 `dev-nfr-lint.py` 门禁规则（Exit Code 0）。

---

# Wave-5 v1.1 契约澄清修订（Lead 裁决）

> 本节为 Wave-5 v1.1 增补与澄清修订段，保留前文 v1.0 原文作为基线。

## K3.1 预算告警的落点分工（Lead 裁决）

### 条款与架构分工
1. **Core 侧零会话累计状态**：
   - 为避免 Rust core 运行时与前端 Pinia store 各自维护一份会话累计状态而产生状态漂移与不一致，**Rust core 侧严禁建立或持久化会话级累计成本状态**。
2. **Core 侧具体职责**：
   - 提供 `cost::is_budget_exceeded(turn_cost: f64, budget: f64) -> bool` 纯函数判定；
   - 提供 `set_budget_usd(Option<f64>)` / `budget_usd() -> Option<f64>` 预算桥接；
   - 当**单轮成本**触发（使该轮单独触及或超过配置的预算门槛）时，记录一条 warn 级别结构化日志：
     ```json
     log_json("warn", { "turn_id": "...", "turn_cost_usd": 0.12, "budget_usd": 0.10, "message": "turn cost reached or exceeded budget threshold" })
     ```
   - 核心层**绝对不做**跨 turn 的会话级累加与会话状态持久化。
3. **前端 Store 具体职责**：
   - 负责多轮会话级累计计算（遍历或流式累加每轮终态下发的 `costUsd`）；
   - 负责驱动 UI 状态展示软告警标签、超限提示及 Tooltip；
   - 双端口径保持一致：同一预算阈值、同一设置项字段名 `costBudgetUsd`。
4. **不变量保证**：
   - 无论单轮还是会话累计超限，均定位为**软告警**，**绝不阻断**用户 Prompt 提交与交互链路（`canSubmitPrompt`、`isInputDisabled` 绝不受成本/预算状态影响）。

### 验收判据
- 核心层单测断言：单轮成本超预算时成功输出带指定字段的 warn 日志，但 core 内部不存在跨 turn 累加计数器。
- 前端单测断言：多轮累计超预算时仅激活告警 UI 状态，发送按钮状态与可用性保持正常。

---

## K5.1 价格表生命周期与「宁缺勿错」原则（补充）

### 条款
1. **价格表快照与变更规范**：
   - `crates/pony-agent-core/src/agent/pricing.rs`（或相关实现文件）中的 `PRICING_TABLE` 静态常态声明顶部，**必须明文标注快照日期**注释（例如 `// Pricing Snapshot: 2026-10-25`）及计价基准单位（USD per 1M tokens）。
   - 任何针对价格表的更新或新增模型，必须在同一 Git 提交内更新该快照日期注释，并在代码注释或 PR 描述中提供官方/供应商的公开核价来源链接。
2. **「宁缺勿错」刚性原则**：
   - 凡无法公开查证、无法获取确切官方单价的实验性模型或私有模型，**一律不得录入价格表**。
   - 未收录模型整体返回 `None`，前端统一呈现「`—`」。
   - **绝对严禁**为了所谓的“完整性”而主观编造、猜测或填入虚假单价。

