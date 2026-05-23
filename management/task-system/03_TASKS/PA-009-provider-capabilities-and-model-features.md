# PA-009 完善 provider 能力配置（思考、多模态、上下文与模型能力）

## 状态
- Status: `In Progress`
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

## 本轮验证
- `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` 通过
- `npm run test:unit` 通过（含 provider 相关前端工作台回归）
- `npm run build` 通过
- `npm run verify` 通过

## 下一步动作
- 把 capability 默认值从启发式判断继续抽成更明确的模型能力目录
- 明确哪些字段属于“用户可配置”，哪些字段属于“模型能力声明”
- 把 `supportsImageInput / contextWindowTokens` 真正接进 runtime 的输入编排与限制逻辑
- 继续收敛不同 provider 对 reasoning / multimodal 的内部抽象

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
