# Wave-6 冻结契约 — 可观测性与成本计量精细化补齐

本契约用于约束 Wave-6 针对成本计价协议归一、链路跟踪子 Span 会话下沉与结构化日志扩展的精细化治理。

---

## K1.1 计价协议语义归一（正确性缺陷修复）

### 1. 现状与缺陷事实
既有计价公式统一假设 `uncached = max(input − cache_read, 0)`。该公式仅对 **OpenAI 系协议**成立，对 **Anthropic 系协议**存在重大漏计缺陷：
- **OpenAI 系协议**：返回的 `prompt_tokens` 已**包含**命中缓存的 tokens。因此未命中缓存的部分为 `uncached = max(prompt_tokens − cached_tokens, 0)`。
- **Anthropic 系协议**：返回的 `input_tokens` **不包含** `cache_read_input_tokens` 与 `cache_creation_input_tokens`。若强行减去 `cache_read`，在例如 `input_tokens = 100, cache_read = 5000` 时，会导致 `uncached = max(100 - 5000, 0) = 0`，导致基础输入价格被彻底漏计（计算为 0）。

### 2. 契约条款与归一规范
1. **输入参数**：计价入口（纯函数）必须显式接收「协议语义类别」（例如枚举 `ProviderProtocol::OpenAi` 与 `ProviderProtocol::Anthropic`，或等价协议标签）。
2. **归一计算规则**：
   - **OpenAI 协议族**（含 OpenAI、DeepSeek、Azure、Ollama 等）：
     ```text
     uncached_input_tokens = max(input_tokens - cache_read_tokens, 0)
     ```
   - **Anthropic 协议族**（含 Anthropic Claude 原生协议、Bedrock Claude 等）：
     ```text
     uncached_input_tokens = input_tokens
     ```
     （`cache_read_tokens` 独立按 `cache_read_per_mtok` 计费，两部分相加为总输入成本）。
3. **未知协议回退与风险说明**：
   - 若遇到未指定或未识别的协议类别，**默认按 OpenAI 系语义处理**（保守计价策略，避免多收费）。
   - **风险周知**：使用 OpenAI 默认兜底若实际对接了 Anthropic 兼容模型，可能导致 cache 场景下轻微少计成本，因此所有接入点（如 `turn_flow.rs`）必须显式传递准确的 `provider_protocol`。
4. **函数纯度**：归一处理逻辑必须为无副作用的纯函数，完全独立于全局状态，支持独立单元测试。

### 3. 拒绝条件
1. 计价入口未区分协议语义，仍对所有 provider 执行无差别减法。
2. 对 Anthropic 协议执行 `input_tokens - cache_read_tokens` 导致计算出漏计的错误结果。

### 4. 验收判据
1. 单元测试针对相同的一组用量参数 `(input = 100, cache_read = 5000, output = 0)` 进行断言：
   - 在 Anthropic 语义下：未命中文档输入计费为 `input_per_mtok × 100 / 1_000_000`（基础输入不为 0）；
   - 在 OpenAI 语义下：未命中文档输入计费为 `0`（`max(100 - 5000, 0) = 0`）。
2. 单元测试覆盖未知协议，断言其默认回退到 OpenAI 语义。

---

## K4.1 `langfuse.session.id` 属性下沉到子 Span

### 1. 契约条款
1. **Span 继承覆盖面**：
   - `llm.chat`（模型调用子 span）与 `tool.call`（工具执行子 span）必须显式携带属性：
     ```text
     langfuse.session.id = <当前活跃 turn 的 session_id>
     ```
2. **生命周期与空值契约**：
   - `langfuse.session.id` 取自当前活跃 turn 所属会话（Session）。
   - 若当前无活跃 turn、无关联 session 或 session_id 为空/空白字符，**绝对不输出该属性键**（不得写入空字符串或伪造值）。
3. **设计考量**：
   - 保证 Langfuse 在按 Observation 维度进行独立聚合、单独导出或下钻分析时，不会丢失会话上下文归属；且对既有 Trace 级按 Session 归集完全兼容无破坏。

### 2. 拒绝条件
1. 在生成 `llm.chat` 或 `tool.call` span 时遗漏 `langfuse.session.id` 属性。
2. 在无活跃会话时输出 `langfuse.session.id: ""` 或 `"null"`。

### 3. 验收判据
1. Wire 级别（OTLP protobuf/JSON payload）单测断言：在存在活跃 turn 会话时，导出的 `llm.chat` 与 `tool.call` span 的 attributes 列表中均包含 `langfuse.session.id`，且值与所属会话 ID 严格一致。
2. 单元测试断言：在无活跃 turn 上下文或脱离会话的环境下生成独立工具/模型 span 时，导出的 attributes 列表中**不存在** `langfuse.session.id` 键。

---

## K6 结构化日志字段（Extras 支持）

### 1. 契约条款
1. **日志 API 扩展**：
   - 核心日志函数 `log_json` 扩展支持可选的结构化键值对（`extras: Option<&[(&str, serde_json::Value)]>` 或支持序列化结构体的重载函数 `log_json_with_fields`）。
   - 附加的结构化字段必须平铺序列化在同一行 JSON 的根级别对象中，例如：
     ```json
     {
       "timestamp": "2026-10-25T15:00:00.000Z",
       "level": "warn",
       "message": "turn cost reached or exceeded budget threshold",
       "trace_id": "0123456789abcdef0123456789abcdef",
       "turn_id": "turn-1234",
       "turn_cost_usd": 0.0075,
       "budget_usd": 0.000001
     }
     ```
2. **向后兼容性**：
   - 既有 `log_json(level, message)` 基础签名保持完全向后兼容；在未提供 extras 时，输出格式与字段集合与既有规范保持 100% 一致。
3. **成本预算告警（K3.1）日志整改**：
   - 成本预算超限的 warn 日志**严禁**将数值以字符串拼接形式塞入 `message` 文本（杜绝无法被日志收集器指标化的非结构化文本）。
   - 成本告警日志必须显式包含数值型的 `turn_cost_usd`（f64）与 `budget_usd`（f64），以及字符串型的 `turn_id` 作为顶级结构化字段。

### 2. 拒绝条件
1. 成本告警日志输出时把 `turn_cost_usd` 或 `budget_usd` 序列化为字符串类型而非数值类型。
2. 破坏现有不带 extras 的 `log_json` 输出格式。

### 3. 验收判据
1. 单元测试断言：调用无 extras 的 `log_json`，JSON 包含原有核心字段（level, message, trace_id, timestamp）。
2. 单元测试断言：调用携带成本告警 extras 的 `log_json`，输出的 JSON 行中包含数值类型的 `turn_cost_usd` 与 `budget_usd` 字段，且 `message` 为干净的常量描述信息。
