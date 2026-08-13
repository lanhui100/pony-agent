# PA-078 对话文件附件入口与文件类型注册表

## Basic Info

- ID: PA-078
- Status: Done
- Priority: P0
- Complexity: B
- Owner: @orchestrator
- Created At: 2026-08-08
- Updated At: 2026-08-08
- OpenSpec Change: `add-chat-file-attachment-entry`（3 路对抗审核已通过，2026-08-09）
- Spec 状态: 通过（proposal/design/spec/tasks 已按采纳意见修订）

## Background

对话中加入文件的能力目前只有后端地基、没有前端入口：`TurnInputImage` → `submitTurn({images})` 链路已通（`src/stores/runtime.ts:3627`），`workspace_read_document`（Firecrawl anydoc）与 `view_image` 工具已存在（`tools.rs:82`），附件生命周期（`AttachmentAsset`）已完整。但 composer 无"加入文件"按钮，用户无法把文件带进对话，模型无法用既有文件处理工具处理它们。

## Goal

在对话输入区增加"别针"附件入口：选择文件 → 类型校验（白名单注册表）→ 按类型路由到既有处理工具（图片→多模态输入 / 文档→read_document / 文本→直接读取），不支持的格式明确提示而非静默失败；类型注册表以 `mime → handler` 数据驱动，保证未来可扩展。

## Scope

- 前端：`WorkspaceComposer.vue` 别针按钮 + 文件选择 + 待发送附件列表（预览/移除）
- 类型白名单注册表（`mime → handler` 映射，数据驱动、可扩展）
- 工具链路由：图片 → 多模态输入；文档 → `workspace_read_document`；文本 → 直接读取注入
- 外部文件（workspace 之外）导入策略：复制到受控目录再处理
- 前端单测 + `npm run verify`

## Non-Goals

- 不做拖拽上传、粘贴图片（后续迭代）
- 不做大文件分片、进度条（限制单文件大小上限即可）
- 不修改 provider 协议与消息格式
- 不实现 workspace 多项目（PA-079/080/081 承接）

## Acceptance Criteria

1. 别针按钮在 composer 可见，点击可打开系统文件选择器。
2. 支持的类型按注册表路由：图片 → `images` 多模态输入（上限 3）；文本 → 内容注入（≤64 KiB，截断标注）；二进制文档 → 引用附着（path + MIME），模型走 `workspace_read_document`。
3. 不支持的扩展名/MIME 明确提示"暂不支持"，不进入发送队列。
4. 附件可预览、可移除；图片随消息进入既有附件生命周期（AttachmentAsset），文档/文本以引用（路径 + MIME）附着于消息、payload 经 workspace 路径解析。
5. 类型注册表由单一数据源（`mime → handler`）驱动，新增类型只改注册表。
6. 前端 vitest、`npm run build`、`npm run cargo:check:shared` 全绿。

## Review Plan

- @consultant：可扩展性设计、与既有附件/图片链路的边界
- @code-reviewer：类型校验安全、路径处理、与 runtime store 集成的正确性
- @tester：支持/不支持矩阵、移除/重复添加、发送链路回归

## Current Progress

- 3 路对抗审核（@consultant / @code-reviewer / @tester，2026-08-09）已完成，findings 全部采纳；spec/proposal/design/tasks 已修订。详见 `02_REVIEWS/2026-08-09-pa078-081-spec-review.md`。
- **实现完成（2026-08-09）**：`file-attachments.ts` 类型注册表/导入、宿主 `import_attachment` + `get_workspace_root`（`attachment_import.rs`，5 单测）、runtime store `pendingAttachments` + `submitTurn` 集成、WorkspaceComposer 别针/附件条、messages 附件摘要。
- **实现后 3 路对抗审核通过（2026-08-09）**：无未解决 P0/P1；幽灵资产守卫、workspaceId 显式拒绝、二分截断、宿主 relativePath、图片 mime 嗅探修正等已修复并有回归测试。详见 `02_REVIEWS/2026-08-09-pa078-implementation-review.md`。
- 验证：前端 vitest 377 通过、`npm run build` 通过、`cargo:check:shared` 通过、core lib 740 全绿、OpenSpec validate 通过。

## Next Action

- 收口：任务状态流转 Done（已完成）；OpenSpec change 归档（如需）；PA-079（workspace 数据模型）启动实现。

## Blockers

- 无

## Resume Hint

- 实现参考 `openspec/changes/add-chat-file-attachment-entry/tasks.md`（已勾选完成）；实现后审核记录 `02_REVIEWS/2026-08-09-pa078-implementation-review.md`；代码入口 `src/lib/runtime/file-attachments.ts`、`src/components/chat/WorkspaceComposer.vue`、`src/stores/runtime.ts` 的 `submitTurn`、`crates/pony-agent-core/src/agent/attachment_import.rs`。
