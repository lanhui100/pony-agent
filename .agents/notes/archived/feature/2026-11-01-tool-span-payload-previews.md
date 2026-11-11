# Agent Note: 工具 span 采集脱敏截断的入参/结果预览

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0061-tool-span-payload-previews.md` 之归档快照，仅供溯源。

## 背景

桌面端 trace 复盘长期缺内容：`tool.call` span 只有 `gen_ai.tool.name` / `agent.tool.status|duration_ms|error`，
Langfuse 侧 TOOL 观察的 input/output 恒为空。真实后果是本轮最关键的失败现场（`workspace_run_command`
的参数与结果、`non_zero_exit(101)` 的 stdout/stderr、Run 到期 promote 的 `timedOut/promoted/jobId`）
在 trace 上不可复盘——审计只能靠 token 数与耗时反推，无法回答「模型到底让 shell 跑了什么、返回了什么」。

同时，一旦把入参/结果原样写入 span，就形成一个**真实的凭据外泄面**：命令行里出现 `export TOKEN=sk-…`、
`curl -H 'authorization: Bearer …'`、家目录绝对路径都是常态。实测还发现既有 `ponysentry::sanitize`
只覆盖「键值 / Bearer / Basic」三种形态，**裸 token**（无键名、无 scheme 前缀的 `sk-…`、`ghp_…`、
`xoxb-…`、`AKIA…`、`AIza…`）会原样通过。

## 候选方案

- **方案 A：不采集，保持现状**。落选：trace 无内容正是本次要修的问题，P0-1 的核心诉求就是「可复盘的最小切片」。
- **方案 B：采集全量（不截断、只脱敏）**。落选：一条 `workspace_read_file` 结果可达数百 KB，
  会把 OTLP 批次撑爆（单属性上限、Langfuse 侧渲染、成本三重代价），且与既有 `spill` 外置策略重复。
- **方案 C：只采集长度/哈希等元信息**。落选：复盘价值几乎为零——「命令是什么」恰恰是必需信息。
- **方案 D（采纳）：默认开启、脱敏 + 2KB 截断的预览**。`ToolSpanInput` 增 `args_preview` /
  `result_preview`，映射到 `langfuse.observation.input|output`；统一走既有 `push_str`（自动 sanitize）。
- **方案 E：默认关闭，靠 env 显式打开**。落选：可观测性的默认值决定多数用户看到什么；一个需要手动开启的
  诊断能力在真实排障里等于不存在（对比 `promote` 的 opt-out 翻转，同一取向）。

## 决策

1. **字段与键**：`ToolSpanInput.args_preview` / `result_preview: Option<String>` → 采集开启且为 `Some` 时
   写 `langfuse.observation.input` / `langfuse.observation.output`；`None` **不写键**（禁止空串占位）。
2. **上限 2KB/字段**：截断按 `char_indices` 边界回退，**不切出非法 UTF-8**；超限追加 `…[truncated]`
   （标记计入 2048 字节预算，故属性长度有硬上界）。
3. **顺序固定为「先脱敏 → 再截断 → `push_str`」**：先脱敏保证截断不会把凭据切成「残缺但仍可识别」的碎片；
   `push_str` 内部的再次 sanitize 对已脱敏文本是幂等的（替换标记不命中任何规则），因此不会把长度顶回上限之上。
4. **开关**：`PONY_OTEL_CAPTURE_TOOL_PAYLOADS` 默认开启（opt-out）；`0/false/off/no` 关闭；
   未识别值回落默认并 warn。关闭只影响这两个键，既有 span 属性一律不变（关 payload ≠ 关 span）。
5. **补裸 token 脱敏面**（`ponysentry::sanitizer`）：新增 `sk-…`（含 `sk-ant-`/`sk-proj-`）、
   `gh[pous]_…`、`xox[abposr]-…`、`AKIA…`、`AIza…` 五类前缀形态，统一替换为 `[REDACTED_SECRET]`。
   取向是「宁可多脱敏」：这些前缀在正常业务文本中极少出现，误伤代价远小于漏报。

## 影响

- **用户可见行为**：Langfuse / Tempo 上 TOOL 观察首次具备 input/output（≤2KB 预览），复盘不再依赖猜测。
- **隐私边界**：不外发原文，只外发「脱敏 + 截断」预览；2KB 上限既是成本闸也是泄露面闸。裸 token 规则
  同时惠及所有既有字符串属性（错误信息、tool_name 等）与 PonySentry 上报路径。
- **性能**：每工具调用最多多两次正则扫描（sanitize）与一次截断，量级为微秒；仅在采集开启时发生。
- **兼容性**：`ToolSpanInput` 新增两个字段属**结构体形状变更**——外部构造点需补 `args_preview: None,
  result_preview: None`（语义不变）。既有冻结验收里「结构体零变更」的表述由本 ADR 接替。
- **验收**：`crates/pony-agent-core/tests/otel_tool_payload_test.rs` 5 例（键映射 / 2KB+截断标记 /
  敏感串脱敏 / `None` 不写键 / 开关关闭），`sanitizer.rs` 内新增裸 token 单测（11 形态 + 4 个不误伤样例）。

