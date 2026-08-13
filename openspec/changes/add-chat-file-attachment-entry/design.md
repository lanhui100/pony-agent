# Design

## Decision Summary

在既有"图片多模态输入 + 附件生命周期"地基上加前端入口与类型注册表，全程不触碰 provider 协议。以下设计已按 3 路对抗审核（2026-08-09）意见修订。

1. 新增 `src/lib/runtime/file-attachments.ts`：类型白名单注册表（单一真相源，spec.md 规范表）+ 文件读取/校验/去重/内容截断。
2. 新增宿主 command：`import_attachment { name, bytes, mime } -> { path }`（复制到 `<workspace_root>/.tmp/imports/`，不存在则 `create_dir_all`；fallback `temp_dir()/pony-agent/`）+ `get_workspace_root`（最小面）。**外部文件导入复制在宿主侧完成，前端不做磁盘 IO。**
3. `WorkspaceComposer.vue` 增加别针按钮与待发送附件条（预览/移除/错误态）；`runtime.ts` 增加 `pendingAttachments`；`submitTurn` 时图片走既有 `images` 路径（前端上限 3，对齐后端 `MAX_TURN_IMAGES`），文本走内容注入（64 KiB 上限 + 截断标注），二进制文档走引用元数据（path + MIME，不注入内容）。
4. 附件-only 发送生成自动摘要消息；发送后清空 `pendingAttachments`；`switchSession`/新建/撤销取消时清空。
5. 浏览器模式：无宿主可写 → 每个被选文件按"外部"处理，仅内存引用（图片 dataUrl、文本内容注入），`import_attachment` 跳过，`path=null`。

## Chosen Direction

### 1. 类型注册表 `file-attachments.ts`（与 spec.md 规范表一致）

```ts
type AttachmentRoute = "image" | "text" | "document";
type AttachmentHandlerSpec = {
  route: AttachmentRoute;
  mimeTypes: string[];        // 白名单 MIME（精确值或 text/* 通配）
  extensions: string[];       // 白名单扩展名（不含点，小写）
  maxBytes: number;           // 单文件上限
  injectMaxBytes: number;     // 注入消息的内容上限（仅 text 路由生效；0 = 不注入）
};

const ATTACHMENT_TYPE_REGISTRY: AttachmentHandlerSpec[] = [
  // 图片 → 多模态输入（dataUrl），后端上限 3 张
  { route: "image", mimeTypes: ["image/png","image/jpeg","image/webp","image/gif"],
    extensions: ["png","jpg","jpeg","webp","gif"], maxBytes: 8 * 1024 * 1024, injectMaxBytes: 0 },
  // 文本可提取格式 → 内容注入（截断标注）
  { route: "text", mimeTypes: ["text/*","application/json"],
    extensions: ["md","txt","json","ts","tsx","js","py","rs","vue","css","html","csv"],
    maxBytes: 1 * 1024 * 1024, injectMaxBytes: 64 * 1024 },
  // 二进制文档 → 引用附着（path + MIME），模型经 workspace_read_document 处理
  { route: "document", mimeTypes: ["application/pdf","application/vnd.openxmlformats-officedocument.*"],
    extensions: ["pdf","docx","pptx","xlsx"], maxBytes: 20 * 1024 * 1024, injectMaxBytes: 0 },
];
```

- `resolveAttachmentRoute(name, mime) -> { route, spec } | null`：**扩展名优先、MIME 兜底**；扩展名与 MIME 冲突时扩展名胜出；均未命中返回 null（提示"暂不支持该文件类型"）。
- MIME 来源：统一经 `<input type="file">` 读取 bytes，MIME 取 `File.type`（WebView 提供）；`File.type` 为空或不可用时按扩展名派生；冲突规则扩展名胜出。
- 未来加类型 = 加一行注册项，不改路由逻辑。

### 2. 文件读取与导入

- `readFileAsDataUrl(file)`：图片转 `TurnInputImage`（既有合同）；进入前做 **magic-byte 嗅探**（png/jpeg/webp/gif 文件头），与后端 `image_artifact.rs` 一致，防 MIME 伪造。
- `importAttachment(name, bytes, mime, workspaceId): Promise<{ path, relativePath }>`：
  - Tauri 模式：调宿主 `import_attachment { name, bytes, mime, workspaceId? }`，宿主把提交的 bytes 写入默认 workspace 的 `<workspace_root>/.tmp/imports/`（不存在则 `create_dir_all`），返回 canonical `path` + 相对 workspace root 的 `relativePath`（`.tmp/imports/<name>`，fallback 目录时为 null）；复制失败 → 结构化错误（复用 `{ code, message }` 信封），前端明确提示。
  - **`workspaceId` 本轮仅接受默认**：非默认 workspace 由宿主显式拒绝（`import_attachment_unsupported_workspace`），避免"参数存在但静默无效"；PA-079 注册表落地后改由宿主按注册表解析目标 root。
  - **`name` 净化（宿主侧信任边界）**：拒绝含路径分隔符（`/`、`\`）或 `..` 组件的 name；canonicalize 目标并校验其位于导入目录内；否则结构化错误，不写任何字节。
  - 前端无法判断源文件是否在 workspace 内（WebView `File` 无路径），故宿主对任何传入文件统一执行导入复制；"源在 workspace 内 / 外"的区分由宿主判定，前端一律使用返回的 path/relativePath。
  - **空文本（0 字节）跳过导入**：前端对空文本不调 `import_attachment`（宿主拒绝空字节），path/relativePath 为 null、内容为空——与浏览器模式行为一致。
  - 浏览器模式：返回 `{ path: null, relativePath: null }`（无宿主可写），文件仅内存引用。
- 可注入接缝（供测试）：`importAttachment(file, { root, importDir, workspaceId })` 与 `pickFiles()` 均可注入。
- 大小预算：超过 `spec.maxBytes` 直接拒绝并提示；文本注入超过 `injectMaxBytes` 截断并标注。

### 3. 前端交互（WorkspaceComposer）

- 别针按钮（`Paperclip` icon，lucide）置于 composer 工具行（provider 选择旁），`data-testid="workspace-attach-button"`。
- 待发送附件条：位于 textarea 与工具行之间，横向 chip：名称 + 大小 + 移除按钮（`data-testid="workspace-attachment-chip-{index}"` / `workspace-attachment-remove-{index}`）；读取失败态 chip 标错误态、可移除，不阻塞文本发送。
- 空消息 + 附件时主按钮可用（放宽 `primaryActionDisabled`）；附件-only 发送生成自动摘要消息（见 §4）。
- **图片数量上限 3**（与后端 `MAX_TURN_IMAGES` 对齐），第 4 张被拒并提示。
- 同一文件重复添加：按 path+name+size 去重，重复时提示已添加。

### 4. 消息构建

- 图片：走 `submitTurn({ images })`（既有链路，`buildDisplayedUserMessage` 已处理摘要）。
- 文本：读取内容注入用户消息（**用户文本在前、附件块在后**，`[附件: <name>]\n\n<content>` 内容 ≤ 64 KiB，超出截断标注），同时把路径/元数据写入消息 `attachments`（既有 `AttachmentMeta` 合同）。附件块位于用户消息（请求尾部）之内，不影响 ADR-0007 稳定前缀（稳定前缀 = 请求前部的 system/tools/history）；若未来改为系统级注入需重新评估。
- 二进制文档：不注入内容；仅写入 `attachments` 引用元数据（canonical 路径 + MIME），模型通过记录路径调用 `workspace_read_document` 处理。路径经 `buildTurnHistory`（`messages.ts:256-261`）透传后端（非空 `relativePath`）。
- 附件-only（无文本）发送：存在注入内容则用注入内容；否则生成摘要消息（如 `[附件: <name>]` 列表）作为用户消息，确保 `submitTurn` 的空消息守卫（`runtime.ts:3635`）不拦截。
- 生命周期：发送后清空 `pendingAttachments`；`switchSession`/新建会话/撤销取消时清空；撤销（undo）不恢复附件。

## Edge Cases

- MIME 伪造/冲突：图片做 magic-byte 嗅探；扩展名与 MIME 冲突时扩展名胜出。
- 空文件（0 字节）：文本按空内容处理；图片拒绝（无效图片）。
- 无扩展名文件：仅当 MIME 命中白名单时接受，否则拒绝。
- 读取失败（权限/文件被删）：chips 标错误态，可移除；不阻塞文本发送。
- 导入目标不可写：宿主返回结构化错误，前端提示，不进发送队列。
- 导入 name 含路径分隔符/`..`：宿主拒绝并返回结构化错误，不写任何字节（防逃逸导入目录）。
- 浏览器模式二进制文档：`document` 路由拒绝并提示"预览模式暂不支持二进制文档"（避免 path=null 时模型收到空引用的静默失效）；text/image 仍仅内存可用。
- 会话切换残留：`pendingAttachments` 在 `switchSession`/新建/取消时清空，防跨会话携带。
- 图片总量：单张 ≤ 8 MiB，且所有图片 dataUrl 总长度 ≤ 后端 24 MiB 上限，超出拒绝并提示。
