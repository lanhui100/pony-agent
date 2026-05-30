# PA-009 完善 provider 能力配置（思考、多模态、上下文与模型能力）

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
把当前“最小可用”的 provider 配置，推进到更接近真实产品和真实 agent core 需要的状态，让 Pony Agent 能明确表达不同模型与 provider 的能力差异，而不是只保存 `provider / protocol / model / temperature / max_output_tokens`。

## 输出
- provider 能力模型草案
- 模型能力矩阵
- 前后端统一的 provider/model 配置结构
- 第一版思考、多模态、上下文相关配置设计

## 验收标准
- 能清楚区分“模型名”和“模型能力”
- 能表达不同 provider / model 是否支持：
- 推理或思考强度
- 图片输入等多模态能力
- 上下文窗口与输出上限
- 工具调用能力
- 前端配置页与 Rust provider 配置结构不再只停留在最小字段

## 当前进展
- 当前 provider 已支持基本字段：`provider / protocol / model / temperature / max_output_tokens`
- 已有 provider 选择、模型选择、API Key 写入和最小联调主链
- OpenAI 兼容与 Anthropic 主路径已经打通
- provider 侧可观测性已开始补齐：`provider_source / provider_mode / fallback_reason`
- provider registry 已接入统一 `SecretStore`：Provider 配置页保存的新 API Key 会直接写入应用密钥存储，而不是继续依赖环境变量热更新
- 这一轮已经落地第一版能力结构：
- `capabilities + reasoning` 已进入 [provider.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/provider.ts) 和 [config.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/config.rs)
- 当前已能表达：
- `contextWindowTokens`
- `supportsTools`
- `supportsStreaming`
- `supportsImageInput`
- `supportsReasoning`
- `reasoningEffort`
- `reasoningBudgetTokens`
- 前端 `providers` store 已补默认值与兼容归一化
- 配置页已支持新增、编辑和保存这些能力字段
- runtime/request 层已做最小透传：
- OpenAI 兼容：支持推理模型时按需附加 reasoning 配置
- Anthropic：支持推理模型时按需附加 thinking 预算配置
- 多模态和上下文窗口当前先完成建模与配置，不强行接入实际消息格式
- 2026-05-23 这一轮继续完成 provider/runtime 收口：
- provider 相关前端状态已纳入更稳定的 runtime trace 持久化与恢复流程，减少 session 切换时的串状态风险
- `HomeWorkspace` 中 provider/model 选择器、思考强度选择与失败态展示已纳入组件级回归验证
- `npm run verify` 已把 provider 类型、前端构建和 Rust 编译放进同一条固定验证链
- 本轮继续把 capability 从“可存储”推进到“真实输入编排”：
- `contextWindowTokens` 已进入 `context.rs` 的 history/native transcript 预算裁剪逻辑，不再只是配置展示字段
- `supportsImageInput` 已进入 developer/system 上下文提示逻辑：无图模型会显式约束“不要假装看见图片”，有图能力的模型也会被提醒“没有真实图片载荷时仍按纯文本处理”
- token 估算在存在 `native_messages` 时已优先按 provider-native transcript 计算，减少 reasoning provider 下的统计偏差
- 本轮继续把 capability 默认值从分散启发式收口到更明确的目录：
- 前端 `providers` store 已引入集中式 capability catalog
- Rust `config.rs` 已引入集中式 capability catalog
- `infer_capability_preset` 不再散落维护多段 if/else，而是围绕模型事实目录做统一匹配
- 本轮也把 provider 配置保存链路从“env-first”推进到“SecretStore-first”：
- `providers.json` 不再落盘真实 API Key
- `resolve_provider_api_key()` 优先读取 `SecretStore`
- `env` 仅保留兼容 fallback，承接旧配置与手工部署

## 本轮验证
- `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` 通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` 通过（41/41）
- `npm run test:unit` 通过（含 provider 相关前端工作台回归）
- `npm run build` 通过
- `npm run verify` 通过

## 下一步动作
- 明确哪些字段属于“用户可配置”，哪些字段属于“模型能力声明”
- 在已完成文本上下文裁剪与图片提示约束的基础上，继续收敛不同 provider 对 reasoning / multimodal 的内部抽象
- 为未来真实图片载荷接入预留统一输入结构，而不是继续把“图片提及”与“图片载荷”混在一起

## 当前卡点
- 当前预置能力值还是启发式默认，不是权威模型事实
- 如果过早把所有能力字段都做成“用户手填”，会把模型事实和运行时策略混在一起
- 多模态输入和上下文窗口目前还是“可声明、可保存”，尚未进入真实 prompt/message 编排

## 断点续跑提示
继续前先看：
- `src/types/provider.ts`
- `src/stores/providers.ts`
- `src/components/ProviderConfigPage.vue`
- `src-tauri/src/agent/config.rs`
- `src-tauri/src/agent/provider.rs`
- `docs/learning/0011-provider-config-and-env.md`
## 2026-05-23 补充进展
- 前后端都已把 provider 配置继续拆成“模型事实 / 能力声明”和“用户策略”两层语义：
- 前端：`ProviderModelIdentity / ProviderModelCapabilityDeclaration / ProviderModelUserPolicy`
- Rust：同样补齐 capability declaration / user policy 的归一化路径
- capability catalog 已集中到前后端各自的单一目录，减少散落的 if/else 启发式。
- 新增前端回归：
- `tests/providers.store.spec.ts`
- `npm run verify` 通过，说明类型、store、页面保存链路和 Rust 编译链路都已闭环。
## 2026-05-24 补充进展
- 新增 Rust `SecretStore` 抽象，Provider 配置层现在除了“模型能力声明 / 用户策略”之外，也补齐了“敏感凭证不落普通配置”的基础设施边界。
- `provider_registry_regression` 已覆盖 secret 持久化、env fallback、provider 删除时清理 secret，以及测试禁止误用默认真实配置路径等回归场景。
- 本轮已把“能力声明”推进到“真实图片载荷 MVP”：前端可附加图片，`TurnInput` 已携带结构化图片输入，Rust runtime 会把当前轮图片 data URL 送入 OpenAI / Anthropic 请求体。
- 当前范围刻意收在“当前轮真实图片输入”，不承诺跨轮图片记忆；后续这部分已拆到 `PA-011` 继续推进。
