# 0031 多模态附件统一受控接入与模型能力按需投影

Status: implemented

## 背景

在 Pony Agent 原有多模态与附件处理链路中，图片附件（`route === "image"`）与文档/文本附件存在割裂：
1. **落盘割裂**：图片仅生成内存 base64 `dataUrl`，不执行 `importAttachment` 受控落盘（`.tmp/imports/`），导致图片附件在前端的 `path` 和 `relativePath` 均为 null；
2. **协议盲传**：请求构建层直接将用户上传的图片 payload（Base64 二进制）原样打包进 `ProviderRequest.images` 与 `native_messages`，不校验当前激活模型是否支持视觉能力（`supportsImageInput`）；
3. **上游不可控报错**：向 DeepSeek、OpenAI o1 等纯文本推理端点发送多模态 Block 或图片参数时，上游服务直接抛出 400 Bad Request（如 "Image input is not supported"），而客户端未做降级处理；
4. **前端心智断层**：在用户选择纯文本模型并拖入图片时，前端 Composer 依然允许作为多模态图片附着，用户缺乏明确的行为预期。

参考 `deepseek-harness` 的资产隔离与投影设计（`request-projection`、`admission`、`transport` 熔断），系统需要建立端到端的多模态资产与能力适配体系。

## 候选方案

### 方案 A：前端入口硬校验阻断（纯前端门禁）
- **做法**：当当前激活模型 `supportsImageInput === false` 时，前端直接禁用图片附件上传，或者在文件选择时提示"当前模型不支持图片并拒绝添加"。
- **落选原因**：
  1. 违反职责单一与 Core 权威原则：模型可以在会话中动态切换，如果用户先选图后换模型，或者子智能体派生调用不同模型，前端门禁将直接失效；
  2. 剥夺了模型的间接工具看图能力：非视觉模型虽然不能直接在系统 Prompt 里查看像素，但可以通过 `view_image`、OCR 等工作区文件工具读取图片文件；如果入口直接阻断，用户完全无法向 Coding 智能体传递截图让其排查。

### 方案 B：Core 协议层静默剥离（简单丢弃）
- **做法**：在 `context.rs` 发现 `!provider.supports_image_input()` 时，直接令 `images = Vec::new()`，不通知模型任何信息。
- **落选原因**：
  1. 造成灾难性的模型幻觉与信息黑洞：用户明确发出“请帮我分析这张报错截图”，模型由于完全看不到图片也看不到任何提示，会陷入“用户在说什么”或胡言乱语；
  2. 缺乏网关层兜底：若有其他入口（如自动化测试、RPC 调用）绕过 `context.rs` 构造 `ProviderRequest`，下游依然可能引发 400 崩溃。

### 方案 C：四级联防体系（统一受控落盘 + 协议按需投影 + 网关熔断防御 + UI 意图对齐）（选中）
- **做法**：
  1. **接入落盘**：图片在获取 `dataUrl` 的同时，受控导入工作区 `.tmp/imports/`，赋予合法的 `path` 与 `relativePath`；
  2. **协议投影**：Core 协议层根据 `provider.supports_image_input()` 动态分支。若为非视觉模型，剥离原始二进制 payload，在用户消息末尾投影出语义化占位句柄（含文件路径与引导词），引导模型调用 `view_image` 等工具；
  3. **网关安全兜底**：`ProviderManager` 在所有入口（`decide_with_tools`、`stream`、`followup`）通过 `sanitize_request` 执行零拷贝防御，彻底剥离非视觉模型的图片参数；
  4. **UI 意图对齐**：前端 Composer 附件 Chip 实时感知当前模型视觉能力，在非视觉模型下标注`工作区引用`并提示预期。
- **入选原因**：既杜绝上游协议崩溃，又完整保留模型通过工作区工具检视图片的工程自由度，兼具健壮性与可用性。

## 决策

1. **前端图片受控落盘统一**：`src/stores/runtime.ts` 在处理 `route === "image"` 时，与普通文档一致调用 `importAttachment` 存入 `.tmp/imports/`，并在构建轮次载荷时为图片赋予优先使用 `relativePath` 的名称。
2. **Core 协议层按需投影**：`crates/pony-agent-core/src/agent/context.rs` 在 `!provider.supports_image_input()` 时：
   - 清空 `ProviderRequest.images`；
   - 在用户输入消息（包括归一化输入与 `native_messages`）注入结构化占位句柄 `[图片附件 `<name>` (<mime>): 当前模型未启用直接视觉通道。若需检查文件请使用 view_image 等工作区工具]`；
   - 杜绝向纯文本模型传输任何 Base64 数据或 `image_url` Content Block。
3. **Provider 网关安全熔断**：`crates/pony-agent-core/src/agent/provider/mod.rs` 引入 `sanitize_request`，在所有执行入口对非视觉模型携带的图片参数进行 fail-safe 剥离并打印警告日志。
4. **前端 Composer 状态指示**：`src/components/chat/WorkspaceComposer.vue` 感知 `currentModel.capabilities.supportsImageInput`，在非视觉模型附着图片时展示`工作区引用`微标签及悬停提示。

## 影响

1. **向后兼容**：支持多模态的视觉模型（如 GPT-4o、Claude 3.5 Sonnet）行为完全不受影响，依然走高效的直接 Base64 多模态通道。
2. **健壮性**：用户在任何非视觉模型（如 DeepSeek-V3/R1）下发送带图消息均不会触发上游 400 崩溃，且模型明确知晓图片存放在工作区的哪个路径。
3. **测试约束**：
   - Rust Core 必须覆盖 `build_request_projects_images_to_placeholders_for_non_vision_model` 与 `sanitize_request_strips_images_for_non_vision_model`；
   - 前端必须覆盖 `tests/file-attachments.spec.ts`（图片导入落盘）与 `tests/workspace-composer-attach.spec.ts`（非视觉工作区引用标签渲染）。
