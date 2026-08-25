# PA-100 回合工具错误保真与文件分段翻页修复

- Task ID: PA-100
- 标题: Read/gather 暴露 startLine 分页参数；timeline 记录真实工具错误；429 感知退避
- 状态: Done（2026-08-25，双轮 spec 审核 + 双 code review 通过，必须修复项全部落实）
- 复杂度: B（跨模块：tools.rs / dispatcher_composites.rs / trace_timeline.rs / provider::mod.rs / trace.ts / browser-preview.ts）
- 负责人: orchestrator + implementation（本会话）；审核：spec reviewer×2、code reviewer×2（独立 subagent）
- 创建时间: 2026-08-25
- Next Action: 无（可提交）。Resume Hint：若需回滚，按任务卡"Code Review"节的 F1/F2/F3 分组原子还原，禁止逐文件 checkout（工作树混有多任务变更）

## 背景（故障还原）

一次真实 turn（用户问 "tauri_adapter.rs是什么？"，provider=商汤 deepseek-v4-flash）出现连锁失败：

1. 模型调用 `Read`(→`workspace_gather_context`) 读到文件前 80 行，结果里的嵌套 plan 显示子调用参数含 `"startLine": 1`；模型于是模仿该参数名发起第二次 Read 并传 `"startLine": 80`。但模型侧 schema（tools.rs 中 `TOOL_WORKSPACE_GATHER_CONTEXT` 定义）没有 `startLine`，dispatcher 的 JSON-Schema 校验（additionalProperties:false）直接拒绝 → `invalid_arguments` → invocation_failed。
2. 该错误确实作为 tool result 回传进历史（回路本身没断），但下一跳 provider followup 撞上商汤 429 "inference tpm exhausted"（code 429001）。`retry_provider_timeout` 的重试预算为 5 次尝试 / ≤约8s 总退避，远小于分钟级 TPM 窗口 → 重试全部失败。
3. 失败后走 `local_tool_followup_fallback_response` 本地兜底，回合以“completed + fallback 文案”收尾，没有可恢复入口。
4. 观测层 bug：`build_persisted_trace_timeline` 对失败工具 hop 写入 `error: Some(parent_tool.description)`（即工具中文描述），trace UI 因此显示「错误: 读取 tauri_adapter.rs 剩余内容」，真实校验错误被掩盖。

## 目标

1. 模型可通过 Read(gather) 显式传 `startLine` 完成大文件分段翻页，schema、执行器、嵌套 plan/arguments 回显一致。
2. 工具失败时 trace timeline 的 error 字段记录结构化真实错误（code+message），不再用描述文本冒充。
3. provider followup/decision 遇 429/rate-limit 类错误时使用更长的退避预算（覆盖 TPM 分钟窗口量级），超时类错误行为保持不变。

## 非目标

- 不改变 fallback 兜底文案的回合语义（completed vs failed 的产品决策另立任务）。
- 不做 Retry-After header 解析（当前错误路径拿不到 header）。
- 不改前端组件（UI 自动受益于后端字段修正）。

## 技术方案

### F1 startLine 端到端透传
- tools.rs：`TOOL_WORKSPACE_GATHER_CONTEXT` input_schema 增加可选 `"startLine"`（integer，最小 1，默认 1）。
- dispatcher_composites.rs：`GatherContextComposite::execute` 解析并 clamp startLine≥1；`gather_single_path` 增加 start_line 参数，segment 子调用与 nested entry arguments、`build_nested_gather_plan`/`build_multi_path_gather_plan` 回显真实值。
- tools.rs 内部旧版 gather 实现（~2666 行区域）同步透传，避免双实现漂移。
- 兼容性：缺省不传时行为与现状完全一致（startLine=1）。

### F2 timeline 错误保真
- trace_timeline.rs：新增 helper，从 parent activity（telemetry 已写入 `error: Option<Value>`，形如 {code,message}）提取错误文本；两个 builder（persisted + progress）中 4 处 `Some(parent_tool.description.clone())` 全部替换；无结构化错误时回退 description（维持现状兜底）。

### F3 rate-limit 感知退避
- provider/mod.rs `retry_provider_timeout`：失败分类命中 rate-limit 特征（"429"/"rate limit"/"rate_limit"/"tpm"）时，后续尝试切换到 RL 退避配置（initial 3s、×2、cap 15s、budget 60s），尝试次数上限不变；timeout 类维持现配置（500ms×2、cap 8s、budget 30s）。

## 风险与回滚

- F1 属纯增量参数，风险低；回滚即还原 schema 与两处 handler。
- F2 仅观测字段，回滚无副作用。
- F3 会拉长 429 场景的回合耗时（最多 ~40s 阻塞重试），需在 spec 审核 中确认可接受；回滚即恢复原配置常量。

## 测试计划

- cargo test -p pony-agent-core dispatcher_composites（startLine 透传断言：nested arguments / plan steps）
- cargo test -p pony-agent-core tools::tests（legacy gather startLine）
- cargo test -p pony-agent-core runtime（timeline 失败 hop error 字段断言）
- cargo test -p pony-agent-core provider（retry_provider_timeout 对 rate-limit 错误采用长退避、对 timeout 维持短退避）

## 测试证据（2026-08-25 实施 + 双轮 review 后复验）

| 套件 | 结果 |
|---|---|
| `cargo check -p pony-agent-core --all-targets` | 通过 |
| `cargo test -p pony-agent-core --lib`（全量，review 修复后） | **913 passed / 0 failed** |
| gather_context 过滤组（含 6 个新用例） | 全部通过 |
| persisted_timeline 组（4 个新用例）+ activity_error_text 截断边界 + progress builder parity | 全部通过 |
| provider retry/rate-limit 组（8 个新用例，FakeSleeper：切换调度/结转/hint 阻断/mixed/decision 短退避） | 全部通过 |
| `cargo test --manifest-path src-tauri\Cargo.toml --test tool_router_regression` | **15 passed / 0 failed** |
| `pnpm exec vue-tsc --noEmit` | 通过（trace.ts / browser-preview.ts 变更） |
| `pnpm exec vitest run tests/trace-projection.spec.ts` | **未能运行**：沙箱环境 vite 配置加载 spawn EPERM + @tailwindcss/oxide 原生绑定不可加载——非本次改动引入；前端行为由 vue-tsc 与 review 消费方逐点审计覆盖 |

未运行项：clippy（项目未配置强制门禁）、全量前端 vitest（同上环境限制）。

## 验收标准

1. Read 传 `{path, startLine:80}` 不再 invalid_arguments，返回第 80 行起内容；不传 startLine 行为不变。
2. 构造失败工具调用的 turn，其持久化 timeline 条目 error 含真实 code/message。
3. 单测证明 rate-limit 错误触发长退避配置且 timeout 配置不受影响。
4. 全部定向测试通过；cargo check 无新告警。

## 审核记录

### Spec Reviewer A（code-reviewer-a 视角）：有条件通过

采纳并裁决：
- P1-1/P1-2（预算结转、指数基数）→ 采纳：切换 RL 配置时重建 RetryBudget 并结转 elapsed_ms；退避指数以配置切换点为基准重置。
- P1-3（无取消通道、~40s 低估）→ 部分采纳：同步 followup 路径无取消通道为既有限制，本次不扩栈改造；~~RL 退避改保守（initial 2s、×2、cap 8s、budget 45s，sleep 上界 ≤30s + 各次请求实际耗时）~~【**已被推翻改案**（code-review B P1-1 对齐记录）：2s/45s 方案 sleep 总和 ~22-30s，无法覆盖分钟级 TPM 窗口，与 Spec Reviewer B"调度须覆盖 ≥60s"的结论冲突；最终定案 initial 20s、cap 40s、budget 90s（sleep 上界 60s），见"实施期裁决补充"第 3 条。残余风险：最坏 wall-clock = 60s sleep + 各次 followup 的 provider 超时耗时；多跳回合按跳重置预算、无回合级上限（code-review B P2-4 记档）】
- P1-4（startLine 与搜索模式优先级）→ 采纳：显式 startLine 一旦提供即全模式生效；仅当未提供且 search 模式时自动定位；抽共享解析函数供 composite/legacy 双实现复用。
- P2-1 → 采纳：只替换 error 字段位（397/436/621/655），不触碰 text 字段（393/617）。
- P2-2 → 采纳：gather 子调用失败导致父聚合 partial 时父条目保持非错误态定义为预期行为（summary.firstError 已承载细节），补断言测试。
- P2-3 → 采纳：rate-limit 关键词判定收敛到 retry.rs 单一来源（新增 is_rate_limited，含 tpm/too many requests/quota），provider 循环基于 classify 结果 + 该判定选配置。
- P2-4 → 采纳：skipped_paths 条目 arguments 同步回显 startLine；schema description 注明多路径下逐路径生效。
- P3-1（prompt-cache 一次性失效）→ 接受，记录。
- P3-2 → clamp 逻辑走共享函数，逐字对齐。
- P3-3 → helper 同时覆盖 aborted 状态的错误提取（不改 state 判定）。

AC 修订：AC1 补短文件 line_out_of_range→partial 断言；AC2 扩为双 builder + 无 error 回退用例；AC3 改为可证伪的延迟序列/预算耗尽原因断言；新增 AC5（partial 父条目行为）。

### 实施期裁决补充（spec 偏差记录，code-reviewer-a 必须修复项驱动）

1. **F2 兜底策略变更**：spec 原文"无结构化错误时回退 description"在实施审核中被推翻——最终实现为返回 None（不回退）。理由：同条目 text 字段已承载描述文本，error 回退同一字符串会复现「错误栏显示描述」的假象。测试 `persisted_timeline_leaves_error_none_when_no_structured_error_present` 钉住。本条为任务卡契约源与 spec 正文冲突时的权威裁定。
2. **混合报文分类**：报文同时含 timeout 与 rate-limit 特征时按 rate-limit 处理（长退避对两类瞬时故障都安全），偏离 reviewer B 建议的"混合走短退避"反例方向；由 `mixed_timeout_and_rate_limit_message_takes_rate_limit_schedule_in_followup` 钉住。
3. **RL budget 口径**：budget 仅计 sleep 时间（record_attempt），不含请求自身执行耗时；90s 指退避睡眠上限，最坏 wall-clock = 60s sleep + 3 次 followup 各自的 provider 超时耗时。取消通道缺失为同步路径既有限制，记档为残余风险。
4. **parity 近似方式**：双实现一致性用"两侧字面断言 + 共享解析函数"近似，未建跨栈 parity harness（治理栈+真实 FS 的组合夹具不存在）；已知既有差异：composite 子调用 arguments 含 GATHER_CHILD_DESCRIPTION 而 legacy 无（module doc 已声明 mirror 语义）。
5. **EOF 越界契约**：实测父聚合 status=partial、errorCount=1、summary.firstError.code=line_out_of_range、顶层 ToolResult.status 保持 ok——src-tauri 回归用例钉住。

### Code Review（A/B 双 reviewer：均"有条件通过"，必须修复项全部落实）

- **A 必须修复**：①F2 兜底策略变更落卡 → 已记入"实施期裁决补充"第 1 条；②progress builder 的 stale "test-only" 注释更正 → 已改为"turn_stream 生产仍在调用，两 builder 字段语义须一致"
- **B 必须修复 P1-1**：早期 P1-3 裁决（2s/45s）与实现（20s/90s）冲突 → 已在原条目标注推翻改案与理由（见上）
- **B 必须修复 P1-2**：流式增量已发出后撞限流会白睡 20s+40s 且 sync fallback 再 60s → 修复：`retry_provider_scoped_with_sleeper` 增加 `no_retry_hint` 通道（sleep 前检查、阻断 RL 切换）；`followup_stream` 传入共享 AtomicBool `streamed_any_delta`；哨兵措辞去掉 "timeout" 特征（classify 判不可重试，decision_stream 的 ≤7.5s 既有浪费同步归零）。新测试 `followup_stream_hint_stops_retries_without_rate_limit_backoff` 钉住
- **B 核心发现采纳**：前端第三副本 `trace.ts:345` 同步为真实结构化错误提取（`extractActivityErrorText`，与 Rust 同语义：kind/code/string 三级、300 字截断、无则 null）；`browser-preview.ts` Read schema 补 startLine
- **A 建议采纳**：is_rate_limit_error 消费前提注释（classify 可重试才被消费）、provider_rate_limit_backoff_config 口径注释（budget 仅计 sleep）、双份 build_multi_path_gather_plan 交叉引用注释
- **A 缺失测试①-⑤全部补齐**：RL 结转（61_500ms 断言）、300 字截断边界（CJK 按字符计）、progress builder parity、{startLine,lineCount} 推断门、legacy 多路径回显
- **记档残余风险**："429"/"tpm" 子串误判最多多等 ~67s（Followup only）；浮点/越界 startLine 静默 clamp 回退（沿用 lineCount 松散契约，schema 不加 minimum——事故创伤下拒绝式校验更危险）；legacy join-fail 分支回显无直接测试（panic 注入困难）；多跳回合 RL 预算按跳重置
