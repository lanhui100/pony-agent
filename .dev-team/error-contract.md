# 统一错误码注册表 — 验证报告 + 黑盒验收契约矩阵（冻结版）

> 作者：Test Agent（L2-T，只读侦察 + 只写本文件）。业务代码零修改。
> 侦察范围：`crates/pony-agent-core/src/agent/{retry,dispatcher,tools,capability_bridge,hooks,turn_flow}.rs`、
> `runtime/{limits,tool_recovery,trace_timeline}.rs`、`provider/{mod,openai_sse,responses_api,model_catalog}.rs`、
> `docs/architecture/runtime.md`、`docs/decisions/`。

## §0 验证报告：Lead 诊断逐条确认/证伪（附文件:行号）

| # | Lead 诊断 | 结论 | 证据 |
|---|---|---|---|
| 1 | 请求级 retry.rs 有 FailureKind + StreamState + RetryBudget | **确认** | `retry.rs:128-133` FailureKind 4变体；`retry.rs:76-82` StreamState 5态；`retry.rs:204-210` RetryBudget；`retry.rs:377-418` decide() 组合三者 |
| 2 | 工具级 dispatcher.rs DispatchError{code,message} | **确认** | `dispatcher.rs:345-349`；`into_outcome` 写 `error.code`（`dispatcher.rs:368-384`） |
| 3 | tools.rs ToolError{kind,message,retryable}、tool_error_from_output | **确认** | `tools.rs:303-314` ToolError；`tools.rs:6498-6523` 解析时 `kind` OR `code` 兜底（6505-6507） |
| 4 | capability_bridge.rs CapabilityFailureKind 6类 | **确认** | `capability_bridge.rs:74-81` 6变体；另有 SkillFailureLayer 6层（`114-121`），两套并存、映射在 `1086-1096` |
| 5 | limits.rs 熔断、tool_recovery.rs 本地收口 | **确认** | `limits.rs:25-72` 四个 `build_*_error` 全为无码中文 format! 字符串；`tool_recovery.rs:424-479` 本地 fallback 构造 `provider_source="provider_followup_recovery_local_fallback"` + `fallback_reason=Some("tool_followup_recovery_failed:…")`（466-469），纯字符串 |
| 6 | turn_flow.rs build_failed_turn_result + fallback_reason | **部分证伪** | `build_failed_turn_result_with_hooks`（`turn_flow.rs:423-470`）置 `provider_source/mode="failed"` 但 **`fallback_reason: None`**（452）；失败原因只进 `emit_stream_failed` 的 `error: Some(String)`（511）。即：失败 turn 无码、无 fallback_reason，只有自由文本 |
| 7 | trace_timeline.rs 三形状兼容 | **确认** | `trace_timeline.rs:117-123` 注释明写三种形状；`132-143` 同时读 `kind`/`code`/纯字符串；取不到时返回 None 且刻意不回退 description |
| 8 | hooks.rs HookFailurePolicy | **确认** | `hooks.rs:601-605` Ignore/Degrade/FailTurn；`runtime/turn_runner.rs:73-82` FailTurn 熔断为 `fail_turn_error: String`，其余继续——hook 失败坍缩为字符串 |
| 9 | 全 Result<T,String> | **确认** | Provider trait 4方法全 `Result<_,String>`（`provider/mod.rs:332,338,349,359`）；`PreDispatchHook::rewrite`（`dispatcher.rs:228`）、`CompositeToolHandler::execute`（`dispatcher.rs:251`）、hook executors（`hooks.rs:955,969,988,996`）、`fetch_model_ids`（`model_catalog.rs:15`）、SSE 解析（`openai_sse.rs` 6处 / `responses_api.rs` 12处 / `mod.rs` 58处 `Result<_,String>`）。结构化上下文在边界丢失 |
| 10 | kind vs code 双形状 | **确认 + 加重** | 实为**三形状**：ToolError 用 `kind`（`tools.rs:305`）+ 错误输出写 `code`（`tools.rs:6486`）+ timeline 还容忍纯字符串。`tool_error_from_output` 双读是事后桥接，非统一 |
| 11 | 无线码注册表/ADR | **确认** | `docs/decisions/` 仅 0001-0020，无错误分类 ADR；`dispatcher.rs:340-343` 注释明示 code 刻意为自由 String；`handler_failure`（`dispatcher.rs:391-405`）把任意 `code:` 前缀提升为码——未注册码全链路直通 |
| 12 | 无 LLM provider 上游系统分类 | **确认** | 见 §0.1 |
| 13 | 无协议不匹配系统分类 | **确认** | 见 §0.2 |

### §0.1 上游错误覆盖现状（`retry.rs:314-371` 关键词子串分类，无类型化状态码）

| 上游类别 | 现状 | 证据 |
|---|---|---|
| timeout | ✅ TransientRetryable | `retry.rs:316-324` |
| 429 / rate limit | ✅ TransientRetryable（+ `is_rate_limit_error` 长退避 `467-476`） | `retry.rs:325-329`；消费前提 `452-462` |
| 502/503/504/408 | ✅ TransientRetryable | `retry.rs:330-338` |
| 连接重置/拒连/DNS | ✅ TransientRetryable（仅字面 `dns` 子串） | `retry.rs:339-346` |
| 400/401/403/404/422/407/413 | ⚠️ 一律 NonRetryable，无细分 | `retry.rs:347-358`：**401/403 认证失败与 400 参数错误不可区分**；无 `auth`/`quota`/`payment` 独立类目 |
| 500/501/505+ 其它 5xx | ❌ 未覆盖 → 默认 NonRetryable Abort | `retry.rs:368-370` 默认分支 |
| quota-only（无 429 伴随） | ❌ Abort 不重试（已知接受残余：混合场景白烧 ~60s） | `retry.rs:452-462` 文档 + `provider/mod.rs:6006` 测试锁定 |
| context 超限 | ⚠️ RequiresRequestMutation（仅 `context too large/context_length/max_tokens/payload too large` 四词） | `retry.rs:359-367` |
| 网络层类型化 | ❌ `format_request_error`（`mod.rs:2063-2099`）产 `type=timeout+connect…` flags，但下游只收到格式化**字符串**，`classify` 重新子串匹配，无类型直通 | `mod.rs:2063` vs `retry.rs:314` 签名 `classify(&self, err: &str)` |

### §0.2 协议不匹配覆盖现状（全为无码 `String`，无系统分类）

| 协议不匹配类别 | 现状 | 证据 |
|---|---|---|
| SSE 解析失败 | ❌ 各解析器独立 `Err(String)`，无协议错误码 | `openai_sse.rs:99,111,298,305`；`responses_api.rs:91,103-104`；anthropic SSE 六处（`mod.rs:2139,2170,2214,2250,2297,2309,2327`，含 1MB 行缓冲上限） |
| tool_call schema 不匹配 | ❌ 静默降级/裸字符串 | `extract_openai_tool_call`（`mod.rs:2730-2756`）坏 arguments JSON **静默 default `{}`**（2743-2748）；anthropic 多 tool_call 唯一裸错（`2885-2894`）；responses 空输出裸错（`responses_api.rs:798`）；`dropped_function_calls>0` 仅 `provider_log`（`mod.rs:1098-1103,1221`） |
| model catalog 未知模型/坏目录 | ❌ 格式化字符串，无稳定码 | `model_catalog.rs:33-40` 状态码拼进消息；`74-81,85-88` 解析失败裸错；无 unknown-model 选择期错误码 |
| reasoning effort 不兼容 | ❌ 被静默抹掉而非分类报错 | `with_openai_request_options`（`mod.rs:2396-2422`）reasoning 模型直接删 `temperature`；responses 侧附加 effort（`responses_api.rs:666-667`）；误配只产生上游 400，无本地码 |
| 事件版本不匹配 | ❌ 无版本门禁，未知事件名直通 | `turn_event_version()` 固定 `"turn-event-v1"`（`turn_flow.rs:784-786`）；`resolve_canonical_event_type` 通配 `_ => name.replace(':','.')`（`780`） |

## §1 黑盒验收契约矩阵（未来注册表必须满足的可验证标准）

> 黑盒定义：仅观测 `ToolOutcome.output` JSON（`error.{code,message}`）、`TurnResult`/`TurnStreamEvent`（`phase/error/fallback_reason`）、timeline 条目 `error` 文本。不碰内部类型。

### C1 错误码命名规范
- [ ] C1.1 码形：`^[a-z][a-z0-9_]{1,63}$`（小写 snake，全局唯一，长度 ≤64）。
- [ ] C1.2 分层前缀：`upstream_*`（LLM/出网上游）/ `protocol_*`（SSE/schema/catalog/effort/事件版本）/ `tool_*`（调度执行）/ `turn_*`（turn 终态）/ `hook_*`（hook 失败）。未知来源一律 `unknown:*` 原文保留，不得丢弃。
- [ ] C1.3 `handler_failure` 式 `code: message` 提升只接受注册表白名单码；未注册前缀不得提升为码（必须落 `handler_error` + 原文进 message），黑盒验证：输出 `error.code=="handler_error"`。
- [ ] C1.4 message 保持人类可读（含中文现状），code 与自然语言解耦：同一 code 在不同 message 下稳定出现。

### C2 kind/code 统一
- [ ] C2.1 全链路单一字段名：`error.code`（淘汰 `kind` 写路径；读路径永久保留 `kind`-or-`code` 兼容，参考 `tools.rs:6505-6507`）。
- [ ] C2.2 `ToolError.kind` 与 `DispatchError.code` 同一注册表取值；timeline `error` 文本形如 `{code}: {message}`（延续 `trace_timeline.rs:138` 格式）。
- [ ] C2.3 纯字符串 error 形状被消灭或显式标记：任何仍为 `Value::String` 的 error 必须以 `unknown:` 码包装，不得裸透（冻结 `trace_timeline.rs:133` 分支的消亡条件）。
- [ ] C2.4 `CapabilityFailureKind`/`SkillFailureLayer` 映射到注册表码（`capability_*` / `skill_*` 前缀），`resolve_invocation` 失败输出携带码而非仅中文句。

### C3 provider 上游类目（逐项黑盒可触发验证）
- [ ] C3.1 `upstream_timeout` / `upstream_rate_limited`（429）/ `upstream_server_error`（5xx 全段，含 500/501/505）/ `upstream_auth`（401/403/invalid_api_key）/ `upstream_quota`（quota-only 独立于 429，可观测 Abort 不烧退避）/ `upstream_context_overflow` / `upstream_network`（connect/reset/DNS）/ `upstream_bad_request`（400/404/422 参数类）。
- [ ] C3.2 分类输入为类型化上游事实（HTTP status / reqwest flags），禁止对渲染后字符串二次子串匹配（冻结 `classify(&str)` 签名的消亡条件）。
- [ ] C3.3 `Retry-After` 与码联动可观测：429 携带码时 delay 行为与文档一致（延续 `retry.rs:283-292` 语义）。

### C4 协议不匹配类目
- [ ] C4.1 `protocol_sse_parse`（openai/responses/anthropic 三路 SSE 解析失败同码，endpoint+parsed_bytes 进 message/evidence）。
- [ ] C4.2 `protocol_tool_call_schema`（arguments 非 JSON、anthropic 多 tool_call、responses 空输出 + `dropped_function_calls>0` 升级为可观测警告而非仅日志；消灭静默 default `{}`）。
- [ ] C4.3 `protocol_model_catalog`（非 JSON 目录、无 ID 目录、上游状态码）+ `protocol_unknown_model`（选择期未知模型）。
- [ ] C4.4 `protocol_reasoning_effort`（effort/temperature 与模型能力冲突时本地报错，而非静默删字段后吃上游 400）。
- [ ] C4.5 `protocol_event_version`（未知事件名/版本 mismatch 显式码，替代 `name.replace(':','.')` 直通）。

### C5 turn 级与 hook 失败码
- [ ] C5.1 `build_failed_turn_result*` 终态携带 `error.code`（字段或 `fallback_reason` 前缀），`fallback_reason: None` 的失败 turn 不得存在。
- [ ] C5.2 `HookFailurePolicy::FailTurn` 熔断输出 `hook_*` 码（含 hook 名 + hook_point）；Ignore/Degrade 在 trace 留码可查。
- [ ] C5.3 limits 熔断（hop/followup/连续同码失败）输出 `turn_*` 码而非裸中文句；`tool_recovery` 本地收口码以 `turn_recovery_*` 开头。

### C6 向后兼容三形状
- [ ] C6.1 读侧永久兼容 `{kind,message}` / `{code,message}` / 纯字符串（以 `trace_timeline.rs:124-161` 现有单测为回归基线，码统一后单测仍全绿）。
- [ ] C6.2 `docs/architecture/runtime.md:164-168` 的 timeout 收口行为（`error.code` 稳定落 `timeout`）在注册表映射下保持：`upstream_timeout` 的线码即 `timeout`（别名冻结，不得改名）。
- [ ] C6.3 事件版本 `turn-event-v1` 不因加码而 bump；码只出现在 payload 字段，不进 `event_type`/`event_version`。

### C7 可扩展性
- [ ] C7.1 加码 = 注册表加一行 + 单测，无需改调用点分支（调用点只传码构造器；禁止新增 `lower.contains("…")` 式关键词分支——以 grep 零新增为门禁）。
- [ ] C7.2 无无线码 ADR 状态：新增/变更任一码必须同提交附 ADR（`docs/decisions/`），延续本文件 §0 的"文件:行号举证"格式。
- [ ] C7.3 未知码 Fail-Closed：运行时遇到未注册码 → `unknown:*` 包装 + error 日志，永不 panic/静默吞错。
