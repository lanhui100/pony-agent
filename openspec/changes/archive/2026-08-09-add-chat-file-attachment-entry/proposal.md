# add-chat-file-attachment-entry

## Background

对话中加入文件的能力目前只有后端地基、没有前端入口：`TurnInputImage` → `submitTurn({ images })` 链路已通（`src/stores/runtime.ts:3627`），`workspace_read_document`（anydoc 转换）与 `view_image` 工具已注册进 governed executor（`tools.rs:82`），附件生命周期（`AttachmentAsset`）已完整。但 composer 无"加入文件"按钮：用户无法把本地文件带进对话，模型无法用既有文件处理工具处理它们；不支持的格式也没有显式限制。

3 路对抗审核（@consultant / @code-reviewer / @tester，2026-08-09）确认的关键约束：

- 后端图片上限为 `MAX_TURN_IMAGES = 3`（`runtime/mod.rs:244`）；原设计引用的"既有 4 张上限"为虚构，前端必须与后端常量对齐。
- 宿主侧不存在 `get_workspace_root`，也没有对话框/文件写入插件；WebView 的 `<input type="file">` 只能拿到无绝对路径的 `File`。**"外部文件导入复制"必须在宿主侧完成（新增 host command），前端无法把字节复制到磁盘。**
- `AttachmentAsset.relative_path` 是相对 `attachment_root` 解析的（`session.rs:748`），后端仅自动注册图片 asset（`session.rs` 附件资产注册逻辑）。因此文档/文本附件不能声称"进入既有 AttachmentAsset 生命周期"；本卡只对图片保持该合同，文档/文本以"引用（路径 + MIME）"形式附着。

## Goals

- 在对话输入区（`WorkspaceComposer.vue`）增加"别针"附件入口：选择文件 → 类型校验 → 加入待发送附件列表（预览/移除）。
- 建立数据驱动的文件类型白名单注册表（`mime → handler` 映射），支持/不支持一目了然，新增类型只改注册表；spec.md 中的注册表为规范表，design.md 逐字复用。
- 按类型路由：图片 → 多模态输入（`images`）+ `view_image`；文本可提取格式（md/txt/json/csv 及常见源码）→ 内容注入（上限 64 KiB，超出截断标注）；二进制文档（pdf/docx/pptx/xlsx）→ 引用附着（路径 + MIME，不注入内容），由模型调用 `workspace_read_document` 处理。
- 不支持的格式明确提示"暂不支持"，不静默失败、不进发送队列。
- 外部文件（workspace 之外）由宿主 `import_attachment` 复制到受控目录（`<workspace_root>/.tmp/imports/`，与 PA-080 的受控 tmp 布局逐字一致；fallback 为稳定非 PID 的 `temp_dir()/pony-agent/`）再处理，为 PA-080 权限边界预留接缝。

## Non-goals

- 不实现拖拽上传、粘贴图片（后续迭代）。
- 不实现大文件分片、上传进度条（单文件大小上限内同步处理）。
- 不修改 provider 协议与消息格式（复用 `TurnInputImage` 与 attachments 消息合同）。
- 不实现 workspace 多项目（PA-079/080/081 承接）。
- 不实现授权审批 UI（PA-080 承接审批语义，本卡只做导入复制）。
- 不实现二进制文档的本地解析转换（由工具面 `workspace_read_document` 负责；本卡只携带引用）。

## Scope

- `src/lib/runtime/file-attachments.ts`（新）：类型白名单注册表 + 文件读取/校验/去重/内容截断。
- 宿主 command（新）：`import_attachment { name, bytes, mime, workspaceId? } -> { path }`（宿主按 `workspaceId` 解析目标 root，写入 `<workspace_root>/.tmp/imports/`，返回 canonical 路径；`name` 做净化防逃逸）+ `get_workspace_root`（最小面；亦可复用 PA-079 `workspace_list` 的根路径读取）。
- `src/components/chat/WorkspaceComposer.vue`：别针按钮、文件选择（浏览器 `<input type="file">` 读 bytes）、待发送附件条（预览/移除/错误态）、空消息+附件可发送。
- `src/stores/runtime.ts`：`pendingAttachments` 状态、`submitTurn` 附件透传（图片走 `images`；文本走内容注入 + 附件元数据；文档走引用元数据）。
- `src/lib/runtime/messages.ts`：附件摘要展示（复用/扩展 `buildDisplayedUserMessage`）；附件-only 发送的自动摘要消息。
- 单元测试：类型矩阵、导入路径、消息构建、发送后清空、会话切换清空。

## Risks

- 宿主接缝缺失：`import_attachment` 与 `get_workspace_root` 为本卡新增 host command，属本卡范围（本卡无前置 blocker，必须自持）。浏览器模式无宿主可写 → 策略：每个被选文件按"外部"处理，仅内存引用（图片 dataUrl、文本内容注入），`import_attachment` 跳过，`path=null`。
- 浏览器/WebView 环境差异：浏览器预览模式与 Tauri 模式都统一走 `<input type="file">` 读取 bytes；Tauri 模式将 bytes 交宿主写入，浏览器模式仅内存引用。能力探测降级。
- MIME 来源与伪造：浏览器模式用 `File.type`；Tauri 模式由扩展名派生 MIME；冲突规则"扩展名优先"。图片进入 dataUrl 前做 magic-byte 嗅探（与后端 `image_artifact.rs` 一致），防类型伪造。
- 大文件撑爆上下文：文本注入内容上限 64 KiB（超出截断并标注"内容已截断"）；单文件上限按注册表 `maxBytes`。二进制文档不注入内容，无上下文风险。
- 图片过大/数量：前端上限 3 张（与后端 `MAX_TURN_IMAGES = 3` 对齐）；dataUrl 总长度受后端 24 MiB 上限约束，超出明确提示。
- 导入目录与 PA-080 边界一致性：布局字符串 `<workspace_root>/.tmp/`（`imports/` 为其子目录）与 PA-080 的受控 tmp 逐字一致；fallback 用稳定非 PID 目录，重启不换路径。
- 多 workspace 导入目标：`import_attachment` 必须携带 `workspaceId`，宿主按激活 workspace 解析目标 root；否则激活 workspace B 时文件复制进 root A，后续按会话 root 判定失配。
- 导入 name 注入：宿主侧净化 `name`（拒绝路径分隔符/`..`），canonicalize 目标并校验位于导入目录内，防绕过 PA-080 边界。
- 浏览器模式二进制文档：`document` 路由拒绝并提示"预览模式暂不支持"，避免 `path=null` 时模型收到空引用的静默失效。
- ADR-0007 兼容性：文本内容注入位于用户消息尾部（请求稳定前缀 = system/tools/history），不影响缓存前缀；若未来改为系统级注入需重新评估。

## Validation

- 单元测试覆盖：支持/不支持类型矩阵（含 MIME 冲突、无扩展名、大小写扩展名、空文件）、外部文件导入（成功/失败）、附件移除/重复添加、图片第 4 张被拒、内容截断、空消息+附件发送（自动摘要）、发送后清空、会话切换清空。
- `npm run test:unit`、`npm run build`、`npm run cargo:check:shared` 通过。
