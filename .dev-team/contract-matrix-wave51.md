# wave-51 契约矩阵（L2-P · 版本单一来源、governance 会话归属、思考/缓存 token 出口）

> Protocol Agent 冻结产物（空桩 / Interface / NFR 唯一编写方）。事实源：`docs/audit/observability-audit-2026-10-27.md` §4 P0-1/P0-2/P0-3 + Gate-1 双路审查裁决。
> 写域：本波我**只**写 `crates/pony-agent-core/src/agent/otel/mod.rs`、`crates/pony-agent-core/src/agent/ponysentry/config.rs`、`.dev-team/`；
> 宿主接线（`src-tauri`）、实现填充与测试冻结分属 Executor / Test Agent。
> 本波空桩纪律：桩体仅 `unimplemented!("wave-51 …")`；冻结断言由 Test Agent 编写，**禁止**本波作者改写测试。
> 版本计数线背景：core crate `0.1.135` 与安装包发布线（`0.1.162`）是两条独立计数线；本波不合并两条计数线，只把「上报取值」改为可注入。

> **契约演进 v2（Lead 裁决）**：C3 范围扩展 —— 根 `agent.turn` span 的 reasoning/cached 聚合纳入本波契约（PA51-C3-T4/T5）。依据：根 span usage 在 `TurnTraceGuard::drop` 内直接 `set_attribute` 补写，不经 `turn_span_attributes` 纯函数；AT2 的 7 键白名单只作用于该纯函数（F12/F13）⇒ 根 span 增键不破冻结断言。
>
> **契约演进 v3（Gate-1 「不放行」修正，本文件为准）**：
> 1. **键名对齐上游 semconv**（V3-1）：常量取值改为 `gen_ai.usage.reasoning.output_tokens` / `gen_ai.usage.cache_read.input_tokens`；写入口径 = `input_tokens` **含**缓存、`output_tokens` **含**思考。旧名 `gen_ai.usage.reasoning_tokens` / `gen_ai.usage.cached_input_tokens` 非标准，**作废**。
> 2. **缓存键协议门控**（V3-1）：仅当上游 `input_tokens` 确实包含缓存命中时才上报 cache 键（OpenAI 协议族）；Anthropic 语义上报会触发 Langfuse 重复减项 ⇒ 本波不上报，登记为已知限制 + follow-up。
> 3. **新增纯函数桩**（V3-2）：`llm_usage_detail_attributes` 供值级断言；T3 改为「纯函数值级 + wire 键名存在性」。
> 4. **调用点接线冻结**（V3-3）：PA51-C3-T6 钉住 `turn_flow.rs` 4 个调用点，并禁止扩 `LlmSpanInput` 字段（冻结结构体字面量会 E0063）。
> 5. **优先级冲突修正**（V3-4）：OTel 三级与 PonySentry 三级分列，不再把 `env:APP_RELEASE` 混进 OTel 链。
> 6. **OT-8e 冻结锚点授权**（V3-5）：见 §7.2。
> 7. ADVISORY 登记（V3-6/7/8/9）：版本注入防御断言 PA51-C1-T4、CLI/TUI 已知限制、`with_release` 与 `from_env` 的 trim 差异 PA51-C1-T5、根 span 聚合沿用新键名。
>
> **契约演进 v4（Gate-1 第 2 轮 PASS 后的最小澄清；不改任何代码与桩，v4 之后契约冻结）**：只澄清三条判定的**作用域与判定接缝**，不改变任何黑盒期望。
> - **V4-1（§1 T4 作用域）**：脏值防御规则**只**作用于「显式入参 `service_version`」与「`PonySentryConfig::with_release`」；env `PONY_OTEL_SERVICE_VERSION` **保持既有 trim-only 语义**（不新增控制字符/长度过滤）⇒ 既有 env 契约不被收窄。
> - **V4-2（§3 T4/T5 断言接缝）**：授权测试侧在 `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs` 内**自写最小 protobuf varint 解码器**（该文件由 Test Agent 新建），用于断言根 span 聚合**数值**（Σ / 0）；不新增依赖、不往 `src/` 添加测试辅助代码。T3 结论不变（纯函数值级 + wire 只断键名）。
> - **V4-3（§3 T6 判定口径）**：T6 以「wire 键存在/缺失 + `turn_flow.rs` 4 处静态计数」判定；协议门控**不强制**纯函数接缝（实现体内联分支即可，不要求新增导出）。
>
> 其余（Rust 常量名保留、T6 未知协议 fail-closed、`LlmSpanInput` 禁扩）**不变**。

---

## 0. 事实锚点（path:line，v3 基线）

| # | 事实 | 锚点 |
| :-- | :-- | :-- |
| F1 | OTel `service.version` 回落取 core crate 版本（`CARGO_PKG_VERSION`），与安装包版本脱钩 | `crates/pony-agent-core/src/agent/otel/mod.rs:569`（`resolve_resource_attributes` 内） |
| F2 | resource 归因解析只有 env 一档覆盖，**没有**「调用方显式传版本」的入参通道 | `crates/pony-agent-core/src/agent/otel/mod.rs:560`、`:175`（`ENV_PONY_OTEL_SERVICE_VERSION` 定义） |
| F3 | 装配入口只收 `OtelConfig`，宿主机拿不到传版本的口子 | `crates/pony-agent-core/src/agent/otel/mod.rs:625`（`pub fn init`） |
| F4 | PonySentry `release` 两处取 core crate 版本（`Default` 与 `from_env` 回落） | `crates/pony-agent-core/src/agent/ponysentry/config.rs:42`、`:119` |
| F5 | PonySentry 配置无「外部注入 release」构造器；宿主只能经 `from_settings` 间接受限覆盖 | `crates/pony-agent-core/src/agent/ponysentry/config.rs:141`、`crates/pony-agent-core/src/agent/ponysentry/mod.rs:34`、`:41` |
| F6 | 发布流水线只烧 `PONY_RELEASE_BUILD`，未注入 `PONY_OTEL_SERVICE_VERSION` / `APP_RELEASE` | `.github/workflows/release.yml:41` |
| F7 | `governance.decide` span 只写 `gen_ai.operation.name`，未下沉会话归属 | `crates/pony-agent-core/src/agent/otel/mod.rs:1744`（`start_governance_decide_span`），调用点 `crates/pony-agent-core/src/agent/usage_governance.rs:355` |
| F8 | 同类子 span 已下沉会话归属（llm.chat / tool.call 各调用一次） | `crates/pony-agent-core/src/agent/otel/mod.rs:1609`、`:1822`；实现 `:2341`（`push_active_session_attribute`） |
| F9 | `TokenUsage` 已解析思考 token 与缓存命中输入 token | `crates/pony-agent-core/src/agent/provider/mod.rs:310`（`cache_hit_input_tokens`）、`:313`（`reasoning_tokens`） |
| F10 | `llm.chat` 属性表只有 input/output/context/speed/cost，无 reasoning/cache 键 | `crates/pony-agent-core/src/agent/otel/mod.rs:41`、`:44`、`:65`、`:74` |
| F11 | `record_usage` 只写 input/output；turn 聚合同样只吃这两项 | `crates/pony-agent-core/src/agent/otel/mod.rs:1638`、`:2221`；调用点 `crates/pony-agent-core/src/agent/turn_flow.rs:2400`、`:2442`、`:2483`、`:2537` |
| F12 | 根 span 的 usage 属性**不经** `turn_span_attributes` 纯函数，而是在 `TurnTraceGuard::drop` 内直接 `set_attribute` 补写（聚合读点 `take_active_turn_usage`） | `crates/pony-agent-core/src/agent/otel/mod.rs:1504`（`impl Drop for TurnTraceGuard`）、`:1553-1569`（usage 写点）、`:2246`（`take_active_turn_usage`）、`:2221`（累加器） |
| F13 | 冻结用例 AT2 的 7 键白名单只约束纯函数 `turn_span_attributes` 的输出；根 span 侧只有「存在性」断言 ⇒ 根 span 增键不破冻结断言 | `crates/pony-agent-core/tests/otel_trace_adversarial.rs:119-127`（`AT2_ALLOWED_TURN_KEYS`）、`:404-417`（唯一应用点）；`crates/pony-agent-core/tests/wave49_observability_gaps.rs:289-294`（仅 `wire.contains(...)`） |
| F14 | 上游 semconv 标准键名（v3 权威口径）：`gen_ai.usage.cache_read.input_tokens`「SHOULD be included in `gen_ai.usage.input_tokens`」；`gen_ai.usage.reasoning.output_tokens`「SHOULD be included in `gen_ai.usage.output_tokens`」；`gen_ai.usage.input_tokens`「SHOULD include all types of input tokens, including cached tokens」 | OTel GenAI registry `docs/registry/attributes/gen-ai.md`（footnote 42 / 47 / 49）：<https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md> |
| F15 | Langfuse ingest 只识别 `cache_read.input_tokens` / `reasoning.output_tokens`，并从 input 减 cache_read、从 output 减 reasoning（避免重复计费）⇒ 若底层 `input_tokens` 不含缓存而上报了 cache 键，会被减成偏小值 | Langfuse `packages/shared/src/server/otel/OtelIngestionProcessor.ts::extractGenericGenAiUsageDetails`（Gate-1 审查 B1 外部核验） |
| F16 | 本仓已有协议语义分流先例：`Anthropic` 的 `input_tokens` **不含** cache、`OpenAi`/`Unknown` 的 prompt_tokens **含** cache；调用点可拿 `provider.protocol_label()` | `crates/pony-agent-core/src/agent/cost.rs:52-56`（语义注释）、`:71`（`ProviderCostProtocol::from_label`）；`crates/pony-agent-core/src/agent/turn_flow.rs:2372`、`:2408`（调用点上下文） |
| F17 | `turn_flow.rs` 有 4 个 `record_usage` 调用点，均形如 `llm_span.record_usage(usage.input_tokens, usage.output_tokens)`；`LlmSpanInput` 存在 2 处**冻结结构体字面量**（新增字段会 E0063） | `crates/pony-agent-core/src/agent/turn_flow.rs:2400`、`:2442`、`:2483`、`:2537`；`crates/pony-agent-core/tests/otel_trace_migration_test.rs:447`、`:496` |
| F18 | OT-8e 静态扫描只认 `otel::init(`，宿主改用新入口会必红（已获 Lead 授权扩展） | `crates/pony-agent-core/tests/otel_trace_migration_test.rs:1142`（`source.contains("otel::init(")`）、`:1171`（失败信息文案） |

---

## 1. PA51-C1 · 版本单一来源（`service.version` / PonySentry `release` 可注入）

**桩落点**：`crates/pony-agent-core/src/agent/otel/mod.rs:609`（`init_with_service_version`）；`crates/pony-agent-core/src/agent/ponysentry/config.rs:141`（`with_release`）。

**两条独立优先级链（v3 修正 V3-4，禁止混写）**
- OTel `service.version`：显式入参 > env `PONY_OTEL_SERVICE_VERSION` > `env!("CARGO_PKG_VERSION")`（**不含** `APP_RELEASE`）。
- PonySentry `release`：`with_release` 显式入参 > env `APP_RELEASE` > `env!("CARGO_PKG_VERSION")`。

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F1/F2/F3：`service.version` 只能取 core crate 版本或进程 env，宿主无法按安装包版本声明 | 显式入参非空 ⇒ resource `service.version` 恒等于该值（trim 后），且**压过** env；与 `CARGO_PKG_VERSION` 不同值以证优先级 | 回环 OTLP sink：`init_with_service_version(config, Some("9.9.9"))` + 造 span + `force_flush` → body 的 resource 段含 `service.version` 且值为 `9.9.9` | **PA51-C1-T1** |
| F2：空白值语义必须与既有 env 覆盖一致（否则空白串会把版本写成空） | 显式入参空白 ⇒ 落 env（trim 后非空时）；env 亦空 ⇒ 落 `CARGO_PKG_VERSION`；任何一级都不得写出空串 | 同上 sink，三组入参（空白 / 空白+env 非空 / 空白+env 空白）各断言一次 | **PA51-C1-T2** |
| F4/F5：PonySentry `release` 与 OTel 版本无法由宿主统一注入 | `with_release(Some(v))`：`v` 合法 ⇒ `release = v.trim()`；`None`/纯空白/不合规 ⇒ **保持原值**（不覆盖、不改其它字段、无 env 读取、无 IO） | 纯构造器单测：`PonySentryConfig::default().with_release(...)` 三态直测 | **PA51-C1-T3** |
| v3（V3-6；**v4-V4-1 收窄作用域**）：版本注入是宿主可控输入，必须防脏值（换行/控制字符/超长串会污染 resource 与 Langfuse release 维度） | **作用域（v4）**：合法化规则**只**作用于 ① 显式入参 `service_version`、② `PonySentryConfig::with_release`；env `PONY_OTEL_SERVICE_VERSION` **不受**该规则约束，保持既有 trim-only 语义（不新增控制字符过滤、不新增长度上限、不做其它改写）。规则本身（仅对上述两处生效）：① trim；② 拒绝任何 `char::is_control()` 控制字符（`\n` `\r` `\t` NUL 等）；③ trim 后字节长度 ≤ **64**；任一条不合规 ⇒ **显式入参**视为未设置并下落 env（env 仍 trim-only）；**`with_release`** 保持原值；两者均**不得**写入非法值、不得 panic | ① 入参侧：`init_with_service_version(config, Some("1.2.3\n"))`、`Some("bad\u{0}ver")`、`Some(&"9".repeat(65))` ⇒ 均落入 env（或 `CARGO_PKG_VERSION`；`Some("  1.2.3  ")` ⇒ `"1.2.3"`）；② 构造器侧：`with_release(Some("1.2.3\n"))`、`Some(&"9".repeat(65))` ⇒ 原值不变；③ **env 反证组**：env `PONY_OTEL_SERVICE_VERSION` 设为含控制字符/超长值时**按既有 trim-only 处理**（不得被新规则拒绝） | **PA51-C1-T4** |
| v3（V3-8）：`with_release` 写入 trim 值，而 env `APP_RELEASE` 路径保留原值 —— 两处口径差异必须被钉住，否则后续会有人「顺手统一」改坏既有 env 语义 | ① `with_release(Some("  1.2.3  "))` ⇒ `release == "1.2.3"`；② env `APP_RELEASE="  1.2.3  "` 时 `PonySentryConfig::from_env().release == "  1.2.3  "`（**保持原值不 trim**，既有语义）；③ 二者共存时 `with_release` 优先覆盖 | 纯构造器 + env 单测（`APP_RELEASE` 用 env 守卫串行化）：三条各断言一次 | **PA51-C1-T5** |

**兼容约束**：`init(config)` 契约不变（等价 `init_with_service_version(config, None)`）；`OtelConfig` 字段与既有构造点不变；不新增环境变量。

---

## 2. PA51-C2 · `governance.decide` 会话归属（Langfuse 会话过滤 9/9）

**桩落点**：本波**不加**新公开签名 —— 该缺陷的修复面是既有 `start_governance_decide_span`（`otel/mod.rs:1744`）内部补 `push_active_session_attribute`，归 Executor；此处只冻结可观测期望。

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F7/F8：`governance.decide` 只写 `gen_ai.operation.name`，缺 `langfuse.session.id` ⇒ 审计 trace 9 span 仅 8 命中会话过滤，Langfuse 不可见治理决策 | 存在活跃 turn 且 session 非空 ⇒ `governance.decide` span 必须输出 `langfuse.session.id`，其值与同 turn 的 `agent.turn` / `llm.chat` / `tool.call` **逐字相同** | 回环 OTLP sink：`begin_turn_trace(session) → start_governance_decide_span + record_verdict → force_flush`，按 span 名筛 `governance.decide` 断言属性 | **PA51-C2-T1** |
| F7：无会话上下文时不得用空串占位（否则会话过滤仍会误命中空 key） | 无活跃 turn / session 为空 ⇒ `governance.decide` **不得输出** `langfuse.session.id` | 同上 sink，不启 turn 只开治理 span，断言该键缺失 | **PA51-C2-T2** |

**不变式**：治理 span 属性白名单（G5.1）不得扩张 —— 本缺陷只复用既有会话键，不新增金额/阈值类动态属性。

---

## 3. PA51-C3 · 思考 token / 缓存命中 token 的 OTel 出口（v3 改键名 + 协议门控）

**桩落点**
- 常量：`crates/pony-agent-core/src/agent/otel/mod.rs:51`（`ATTR_GEN_AI_USAGE_REASONING_TOKENS` = `gen_ai.usage.reasoning.output_tokens`）、`:59`（`ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS` = `gen_ai.usage.cache_read.input_tokens`）。
- 纯函数：`crates/pony-agent-core/src/agent/otel/mod.rs:1128`（`llm_usage_detail_attributes(reasoning_tokens, cached_input_tokens) -> Vec<(String, AttrValue)>`，空桩）。
- 方法：`crates/pony-agent-core/src/agent/otel/mod.rs:1658`（`LlmSpanGuard::record_usage_details`，空桩；内部须复用上面的纯函数）。

**写入口径（v3 冻结，与 semconv 一致 —— F14）**：`gen_ai.usage.input_tokens` **含**缓存命中 token；`gen_ai.usage.output_tokens` **含**思考 token；两个新键是它们的**子集读数**，禁止在成本/用量侧二次相加。

| 缺陷锚点 | 黑盒期望 | 可测接缝 | 冻结断言 |
| :-- | :-- | :-- | :-- |
| F9/F10/F11/F14：`TokenUsage` 有 reasoning/cache 字段，但 `llm.chat` 只有 input/output 出口，思考与缓存命中既不进 Langfuse usage 也不进根 span 聚合 | 有值才产出：`Some(v)` ⇒ 纯函数分别产出 `gen_ai.usage.reasoning.output_tokens` / `gen_ai.usage.cache_read.input_tokens`（`AttrValue::I64`）；两键互相独立（可只出现其一） | **纯函数值级断言**（不依赖 wire varint 解析）：直调 `llm_usage_detail_attributes(...)` 断言键值对与 `AttrValue::I64` 类型；wire 侧只断言两**键名**在 body 中出现 | **PA51-C3-T1** |
| F10：0 与「缺失」必须可区分（否则思考档位为 0 的调用会被误判为未采集） | `Some(0)` ⇒ **必须产出 0**；`None` ⇒ **不得产出该键**（零空串/零占位） | 纯函数：三组输入（both None / `Some(0)+Some(0)` / 混合）各断言一次 | **PA51-C3-T2** |
| —（v3）：属性值为 `i64`，`u64` 直转会溢出成负数；wire 上的 varint 不可反解，故须在纯函数层判定 | 超出 `i64::MAX` 的 `u64` ⇒ 饱和为 `i64::MAX`；不得 panic、不得出现负值、不得截断为其它数 | 纯函数：传入 `u64::MAX` 断言 `AttrValue::I64(9223372036854775807)`；wire 只断言键名存在（不断言数值） | **PA51-C3-T3** |
| F11/F12（v2）：根 `agent.turn` span 只聚合 input/output，思考与缓存命中在根 span 完全缺失 ⇒ 整轮用量/成本口径系统性偏小 | 同一 turn 内 N 个子 `llm.chat` 各调 `record_usage_details(Some(a_i), Some(b_i))` 后，根 `agent.turn` span 导出 `gen_ai.usage.reasoning.output_tokens = Σa_i`、`gen_ai.usage.cache_read.input_tokens = Σb_i`（整型、饱和）；**仅当至少一个子 span 提供有效值时才输出**对应键（两键各自独立判定） | 回环 OTLP sink：`begin_turn_trace` → 2~3 个 `start_llm_span` 各 `record_usage_details` → drop 根守卫 → `force_flush`；断言 `agent.turn` 两键**数值**——**v4 授权**测试侧在 `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs` 内自写最小 protobuf varint 解码器（不新增依赖、不往 `src/` 加辅助代码） | **PA51-C3-T4** |
| F12（v2）：聚合「无值」与「0」必须可区分，否则根 span 会用 0 伪造「已采集」 | ① 所有子 span 都传 `None` ⇒ 根 span 两键**均不出现**（禁止 0 占位）；② 子 span 传 `Some(0)` ⇒ 根聚合值 = 0 且**必须输出**该键 | 同上 sink（同一套 v4 授权 varint 解码器）：两组场景（all-None / `Some(0)+Some(0)`）各断言一次根 span 两键的有无与取值 | **PA51-C3-T5** |
| v3（V3-1/V3-3, F15/F16/F17）：字段有值不等于**可上报** —— cache 键在 Anthropic 语义下会被 Langfuse 重复减项；且 4 个调用点分散，漏接即静默丢数据 | ① `turn_flow.rs` 的 **4 个** `record_usage` 调用点旁必须各调一次 `record_usage_details(usage.reasoning_tokens, <门控后 cache>)`，共 **4 处**（grep 计数 = 4，不得 3/5）；② **协议门控**：`provider.protocol_label()` 属 OpenAI 协议族（`openai-completions` / `openai-responses`）⇒ 传 `usage.cache_hit_input_tokens`；`anthropic-messages` ⇒ 传 `None`（见 §4 已知限制）；无法识别的协议标签 ⇒ 传 `None`（**fail-closed**，宁可少报也不让 Langfuse 减项）；③ `reasoning_tokens` **不受**协议门控（思考 token 在两类协议下都计入 output）；④ **禁止**给 `LlmSpanInput` 增字段（`tests/otel_trace_migration_test.rs:447`、`:496` 是冻结结构体字面量，增字段即 E0063） | 静态断言：`grep -c "record_usage_details(" crates/pony-agent-core/src/agent/turn_flow.rs` = 4；`grep -c "record_usage(" …` = 4 且每处同一代码块内出现 `record_usage_details`；**v4-V4-3 判定口径**：门控正确性以「wire 键存在/缺失」判定（OpenAI 协议一组 ⇒ cache 键存在；Anthropic 协议一组 ⇒ cache 键缺失），**不强制**协议门控留纯函数接缝（实现体内联分支即可） | **PA51-C3-T6** |

**不变式**：既有 `gen_ai.usage.input_tokens` / `output_tokens` / `context_window` / `context_ratio` / `gen_ai.speed.*` / `gen_ai.cost.usd` 的键名、出现条件与语义零变更。
**v2/v3 冻结边界**：根 span 新增键**只允许**上述两个固定 usage 键（v3 新名）；`turn_span_attributes` 纯函数的 7 键冻结集（`AT2_ALLOWED_TURN_KEYS`）与属性条数上界**保持不变**（聚合写在 `TurnTraceGuard::drop`，不经过该纯函数 —— F12/F13）。`LlmSpanInput` 结构体形状冻结（F17）。

---

## 4. Non-Goals / 已知限制（本波非目标）

1. 不合并 core crate 与安装包两条版本计数线；只提供注入通道与优先级契约。
2. 不改 `OtelConfig` 公共字段、既有构造点与 `init` 签名；不加依赖、不加网络 IO、不加环境变量。
3. 不改治理 span 属性白名单（G5.1），不把金额/阈值写进 `governance.decide`。
4. 根 span 聚合纳入本波：**仅**新增 `gen_ai.usage.reasoning.output_tokens` / `gen_ai.usage.cache_read.input_tokens` 两个固定 usage 键的整轮聚合；不扩其它键、不改 `turn_span_attributes` 纯函数的 7 键冻结集（`AT2_ALLOWED_TURN_KEYS`），不新增聚合器公开 API（累加器属私有实现细节）。
5. 不动审计报告 P1-4 / P1-5 / P1-6（`gen_ai.system` 显示名、Langfuse 成本、思考树内容采集）。
6. 不碰 `src-tauri`（宿主装配接线、发布流水线 env 注入归 Executor）；不改 `crates/pony-agent-core/tests/**`（断言冻结归 Test Agent）。

**已知限制（本波不做，登记留痕）**

| # | 限制 | 原因 | 后续 |
| :-- | :-- | :-- | :-- |
| L1（V3-1） | **Anthropic 协议不上报 cache 键** | 其 `input_tokens` 不含 cache（F16），而 Langfuse 会无条件从 input 减 `cache_read.input_tokens`（F15）⇒ 上报会把输入用量减成偏小值。若确需上报，必须先把 `input_tokens` 归一为「含缓存」口径，属跨模块语义变更 | follow-up：协议侧归一（`input_tokens += cache_read` 的语义改造）+ 单测后再放开 |
| L2（V3-7） | **CLI / TUI 仍报 core crate 版本** | 安装包版本只有桌面端宿主（Tauri 装配点）拿得到；CLI/TUI 无发布清单来源 ⇒ 只能落 `CARGO_PKG_VERSION` | 由 Lead 在 ADR / `docs/observability.yaml` 声明中写明「版本维度按形态分口径」 |
| L3 | 未知协议标签不上报 cache 键（fail-closed） | 与 `cost.rs` 的 `Unknown → OpenAI` 计价取舍**不同侧**：计价宁可少收，遥测若误报会直接污染 Langfuse 用量口径 | 若确认为 OpenAI 兼容上游，请在接入点显式标注协议标签 |
| L4 | 不改既有 env 语义（`APP_RELEASE` 不 trim） | 改动会波及 CLI/TUI 既有行为，超出本波最小闭包 | 见 PA51-C1-T5：差异已用断言钉住，不做统一 |

---

## 5. NFR 映射（详见 `.dev-team/nfr-baseline-wave51.json`）

| NFR 键 | 取值/判据 | 覆盖 |
| :-- | :-- | :-- |
| `external_call_timeout_ms` | 3000ms（继承 wave-46，未收紧） | 全波 |
| `retry` | `max_attempts=1`、`backoff_ms=500`、`backoff_cap_ms=1000`（继承） | 全波 |
| `concurrency_lock` | `mutex`（继承；OTel 全局态仍走既有 `lock_state`，不新增锁） | 全波 |
| `logging` | `format=json`、`trace_id=true`、字段名 `trace_id`（继承） | 全波 |
| `resource_release` | 继承 wave-46；本波实际释放面 = `otel_provider_flush_and_shutdown_on_exit`（旧行为不变） | 全波 |
| `wave51_observability_semantics.config_selfcheck` | `OtelConfig` 公共结构/构造点不变；`init(config)` 等价 `init(config, None)`；空白=未设置；无新 env/依赖/网络 IO；`LlmSpanInput` 结构体不变 | C1 / C3 |
| `…pa_51_c1_single_version_source` | OTel 三级：`explicit_argument` > `env:PONY_OTEL_SERVICE_VERSION` > `CARGO_PKG_VERSION`；trim + 空白下落 | C1 |
| `…pa_51_c1_single_version_source.ponysentry_release_precedence` | PonySentry 三级：`explicit_with_release` > `env:APP_RELEASE` > `CARGO_PKG_VERSION`（与 OTel 链**分列**，V3-4） | C1 |
| `…pa_51_c1_single_version_source.injection_validation` | **作用域（v4）**：只作用显式入参 `service_version` 与 `with_release`；env `PONY_OTEL_SERVICE_VERSION` 保持既有 trim-only。规则：trim → 拒控制字符 → 长度 ≤ 64 → 不合规视为未设置（入参下落 env / `with_release` 保持原值） | C1（PA51-C1-T4） |
| `…pa_51_c1_single_version_source.with_release_vs_env_trim` | `with_release` 写 trim 值；`env:APP_RELEASE` 保留原值；既有 env 语义不改 | C1（PA51-C1-T5） |
| `…pa_51_c2_governance_session_attribution` | 有活跃会话即输出 `langfuse.session.id` 且与同 turn 兄弟 span 同值；无会话不输出；会话过滤 9/9 | C2 |
| `…pa_51_c3_reasoning_and_cache_tokens` | 键名对齐 semconv；有值才写、0 有效、`u64→i64` 饱和（`i64::MAX`）；值级断言走纯函数 `llm_usage_detail_attributes`；既有 usage 键不变 | C3 |
| `…pa_51_c3_reasoning_and_cache_tokens.cache_report_gating` | 仅 OpenAI 协议族（`openai-completions`/`openai-responses`）上报 cache 键；`anthropic-messages` / 未知标签 ⇒ 不上报（fail-closed）；`reasoning_tokens` 不受门控。**v4 判定口径**：以 wire 键存在/缺失判定，不强制纯函数接缝 | C3（PA51-C3-T6） |
| `…pa_51_c3_reasoning_and_cache_tokens.call_sites` | `turn_flow.rs` 4 处调用点齐全（grep = 4）；禁止扩 `LlmSpanInput` 字段 | C3（PA51-C3-T6） |
| `…pa_51_c3_reasoning_and_cache_tokens.root_turn_aggregation` | 根 `agent.turn` 导出两键 = 本轮各 `llm.chat` 有效值之和（整型、饱和）；至少一个子 span 有值才输出 **v4 断言接缝**：测试侧自写最小 varint 解码器（`tests/observability_reporting_fixes_test.rs`，不新增依赖） | C3（PA51-C3-T4） |
| `…pa_51_c3_reasoning_and_cache_tokens.root_turn_absence` | 全部子 span 为 `None` ⇒ 根 span 两键**均不出现**（禁止 0 占位）；`Some(0)` ⇒ 根聚合为 0 且必须输出 | C3（PA51-C3-T5） |

---

## 6. 本波落地的冻结接口清单（仅签名，无实现）

| 落点 | 签名 | 状态 |
| :-- | :-- | :-- |
| `crates/pony-agent-core/src/agent/otel/mod.rs:609` | `pub fn init_with_service_version(config: OtelConfig, service_version: Option<&str>) -> Result<(), String>` | 空桩 `unimplemented!()` |
| `crates/pony-agent-core/src/agent/otel/mod.rs:1128` | `pub fn llm_usage_detail_attributes(reasoning_tokens: Option<u64>, cached_input_tokens: Option<u64>) -> Vec<(String, AttrValue)>` | 空桩 `unimplemented!()`（v3 新增） |
| `crates/pony-agent-core/src/agent/otel/mod.rs:1658` | `impl LlmSpanGuard { pub fn record_usage_details(&mut self, reasoning_tokens: Option<u64>, cached_input_tokens: Option<u64>) }` | 空桩 `unimplemented!()` |
| `crates/pony-agent-core/src/agent/otel/mod.rs:51` | `pub const ATTR_GEN_AI_USAGE_REASONING_TOKENS: &str = "gen_ai.usage.reasoning.output_tokens";` | 常量（v3 改值） |
| `crates/pony-agent-core/src/agent/otel/mod.rs:59` | `pub const ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";` | 常量（v3 改值） |
| `crates/pony-agent-core/src/agent/ponysentry/config.rs:141` | `pub fn with_release(self, release: Option<&str>) -> Self` | 空桩 `unimplemented!()` |

> 门禁：`cargo check -p pony-agent-core` 必须 Exit 0（空桩可编译即契约成立）。
> 实现期红线：桩体不得保留任何业务逻辑；`unimplemented!()` 由 Executor 按上表语义替换，不得改动签名。

### 6.1 v2 增量（PA51-C3-T4/T5）的接口结论：**零新增桩、零签名变更**

| 事项 | 裁决 | 依据 |
| :-- | :-- | :-- |
| 是否需要新的公开聚合 API | **不需要** | 根 span 聚合写在既有 `TurnTraceGuard::drop`（`crates/pony-agent-core/src/agent/otel/mod.rs:1553-1569` 同段落），累加/读取沿用私有 `accumulate_active_turn_usage`(:2221) / `take_active_turn_usage`(:2246) 形态扩展，属私有实现细节 |
| 是否改动既有桩签名 | **不改** | T4/T5 只消费 `record_usage_details` 的两参语义 |
| 冻结测试是否需要改 | **不需要**（见 §7.1） | F13：AT2 7 键白名单只作用于纯函数；wave49 只做存在性断言 |
| 现有 `input_tokens`/`output_tokens` 聚合行为 | **零变更**（含 `0` 不输出键的既有行为） | `take_active_turn_usage` 的 `> 0` 判据不在本波扩用，新键按 T4/T5 独立判定 |

### 6.2 v3 增量（PA51-C3-T1/T3/T6）的接口结论

| 事项 | 裁决 | 依据 |
| :-- | :-- | :-- |
| 新增 1 个公开纯函数桩 | **需要**：`llm_usage_detail_attributes`（:`1128`） | wire 上的属性值是 protobuf varint，测试无法可靠反解 ⇒ 值级/饱和断言必须走纯函数（V3-2） |
| 常量取值变更 | **是**（v1 → v3）：`gen_ai.usage.reasoning.output_tokens` / `gen_ai.usage.cache_read.input_tokens` | semconv 标准名 + Langfuse 只识别这两个（F14/F15） |
| `record_usage_details` 签名 | **不变**（仍两参 `Option<u64>`） | 协议门控放在**调用点**（`turn_flow.rs`），不污染遥测守卫接口 |
| 新增协议门控辅助 API | **不需要** | 调用点已有 `provider.protocol_label()`（`turn_flow.rs:2408` 等）与 `ProviderCostProtocol::from_label`（`cost.rs:71`），可直接复用 |
| `LlmSpanInput` 扩字段 | **禁止** | F17：冻结结构体字面量 `tests/otel_trace_migration_test.rs:447`、`:496` 会 E0063 |
| 新增环境变量 | **禁止** | 与「无新 env」自检项一致；版本注入走显式入参 |

---

## 7. 冻结测试影响判定（v2 + v3）

### 7.1 无需修改的冻结断言

| 冻结用例 | 断言对象 | 为何不破 |
| :-- | :-- | :-- |
| `crates/pony-agent-core/tests/otel_trace_adversarial.rs:119-127` | 常量 `AT2_ALLOWED_TURN_KEYS`（7 键，字面量） | 只作为「合法键集」入参 |
| `crates/pony-agent-core/tests/otel_trace_adversarial.rs:404-417`（及 `:469-475`、`:499-503`） | **纯函数** `otel::turn_span_attributes(&input)` 的键数与键集 | 根 span 聚合写在 `TurnTraceGuard::drop` 的 `set_attribute`，不经过该纯函数 ⇒ 纯函数输出不变 |
| `crates/pony-agent-core/tests/wave49_observability_gaps.rs:289-294` | wire 中 `gen_ai.usage.input_tokens` / `output_tokens` **存在性** | 存在性断言对新增键不敏感（无键数上界断言） |
| `crates/pony-agent-core/tests/otel_trace_migration_test.rs:402-427`、`:485-486`、`:510-511` | 纯函数 `turn_span_attributes` / `llm_span_attributes` 的键值 | 入参为纯函数而非导出 span；本波不改这两个纯函数的键集 |
| `crates/pony-agent-core/tests/otel_trace_migration_test.rs:447`、`:496` | `LlmSpanInput` 结构体字面量 | 本波**禁止**扩该结构体字段（PA51-C3-T6 ④）⇒ 不产生 E0063 |

### 7.2 OT-8e 冻结锚点授权（V3-5，Lead 裁决留痕）

- **事实**：`crates/pony-agent-core/tests/otel_trace_migration_test.rs:1142` 的 OT-8e 静态扫描为 `if source.contains("otel::init(")`；宿主改用 `otel::init_with_service_version(` 后该扫描会找不到装配点而**必红**（失败信息在 `:1171`）。
- **Lead 裁决**：显式授权 **Test Agent**（且仅 Test Agent）把该静态扫描扩为「`otel::init(` **或** `otel::init_with_service_version(`」，即同一判定同时接受两条入口。
- **授权边界**：
  1. 只改 `:1142` 的一处匹配条件（可加一个 `||` 分支），`:1171` 的失败信息文案可同步补入口名，**其余断言一律不动**（包括 `RunEvent::Exit` + `otel::shutdown()` 退出路径扫描、扫描面文件存在性断言 `:1130-1135`）。
  2. 该修改是**唯一**被授权触碰的冻结测试面；除此之外本波禁止改任何 `tests/**`。
  3. 若 Test Agent 认为需要更大改动，须回 Lead 重新裁决，不得自行扩大。
- **责任划分**：Protocol（本文件）只登记授权；Executor 改宿主入口；Test Agent 改该扫描；三者不可互相代做。

### 7.3 v4 授权的测试侧动作（与「改冻结测试」区分）

| 动作 | 授权对象 | 性质 | 说明 |
| :-- | :-- | :-- | :-- |
| 新建 `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs` 并在其中自写最小 protobuf varint 解码器（V4-2） | **Test Agent** | **新增测试文件**（非改冻结断言） | 用于断言根 span 聚合数值（Σ / 0）；不新增依赖、不往 `src/` 加测试辅助代码、不改任何既有测试文件 |
| 把 OT-8e 扫描条件扩为「`otel::init(` 或 `otel::init_with_service_version(`」（V3-5，详见 §7.2） | **Test Agent** | **改一处冻结条件**（Lead 已批准） | 仅 `tests/otel_trace_migration_test.rs:1142` 一处匹配条件（+ `:1171` 文案），其余断言不动 |
| 宿主改用新装配入口、向 `record_usage_details` 接线（T6） | **Executor** | 实现（非测试） | 不属测试面 |

结论：**冻结断言的修改面仅 §7.2 授权的一处扫描条件**；v4 允许 Test Agent **新增**测试文件承载数值断言（§7.3），其余既有 `tests/**` 一律只读。若实现后出现其它红，属实现侧语义偏差，禁止改测试放行。
