# P0-1 全链路 Payload 可观测性契约矩阵 (Contract Matrix)

| 规范维度 | 约定定义 | 补充说明 / 约束依据 |
| :--- | :--- | :--- |
| **所属任务** | P0-1 全链路 Payload 可观测性 | 覆盖 `llm.chat` 与 `agent.turn` span 预览、脱敏与截断 |
| **双轨语义规范** | **OTel GenAI 语义轨**：<br>- `gen_ai.prompt`<br>- `gen_ai.completion`<br><br>**Langfuse 语义轨**：<br>- `langfuse.observation.input`<br>- `langfuse.observation.output` | - 既有工具 span 复用 `langfuse.observation.input/output`<br>- `llm.chat` 与 `agent.turn` 对齐 OTel `gen_ai.prompt/completion` 语义，并同步填充 Langfuse 输入输出 |
| **Span 归属映射** | - `agent.turn` 根 span：记录 turn 的用户输入（Prompt）与最终输出（Response）<br>- `llm.chat` 子 span：记录发送给大模型的对话上下文序列/Prompt 与大模型回复内容（Completion）<br>- `tool.call` 子 span：既有 `langfuse.observation.input/output` | 确保在 Tempo、Langfuse 及监控面板上各层级均可追溯 Payload |
| **脱敏策略 (Sanitization)** | **前置强制脱敏**：<br>复用 `crates/pony-agent-core/src/agent/ponysentry/sanitizer.rs`：<br>1. 敏感路径替换为 `[USER_HOME]`；<br>2. 常见鉴权头：`Bearer [REDACTED_SECRET]`、`Basic [REDACTED_SECRET]`；<br>3. 裸 Token：识别 `sk-`、`ghp_`、`xox-`、`AKIA`、`AIza` 等特征前缀替换为 `[REDACTED_SECRET]`；<br>4. JSON 敏感键值（`api_key`, `token`, `password`, `secret` 等）全量脱敏；<br>5. 必须保持**先脱敏、后截断**原则，严禁截断导致凭据切片泄漏。 | 严格保持与既有安全防护体系一致；脱敏算法需保证幂等性 |
| **截断策略 (Truncation)** | **UTF-8 安全字符截断**：<br>- 默认单字段预览上限：`4096` 字节（4KB）或 `2048` 字节（按 span 类型定义，基线上限 4KB）<br>- 截断标记：`…[truncated]`<br>- 算法约束：从 `budget = MAX - len(marker)` 向前回溯至 `text.is_char_boundary(end)`，严禁破坏多字节字符编码 | 避免因字节硬切导致乱码或 JSON 反序列化异常 |
| **开关控制 (Feature Flags)** | **环境变量**：`PONY_AGENT_CAPTURE_LLM_PAYLOADS`<br>**默认行为**：默认开启（Opt-out 机制）<br>**解析规则**：<br>- 关闭值：`0` / `false` / `off` / `no`（大小写不敏感，支持 trim）<br>- 未识别值回落默认开启并打印 warn 日志<br>- 当开关关闭时，不输出 Payload 属性键，已有元数据（token/耗时/model）完全保留 | 与 `PONY_OTEL_CAPTURE_TOOL_PAYLOADS` 控制口径一致 |
| **日志与 Trace 关联** | 所有与 Payload 处理或 span 导出相关的核心日志必须结构化携带 `trace_id` 与 `span_id` | 保障分布式追踪与日志系统的全链路关联 |
