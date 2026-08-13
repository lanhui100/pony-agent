# Tasks

- [x] 新建 `src/lib/runtime/file-attachments.ts`：类型注册表 `ATTACHMENT_TYPE_REGISTRY`（spec.md 规范表逐字复用）+ `resolveAttachmentRoute`（扩展名优先、MIME 兜底）+ `readFileAsArrayBuffer`/`bytesToDataUrl`（图片 magic-byte 嗅探）+ `importAttachment`（可注入 `{ root, importDir, workspaceId }`）+ `truncateTextByBytes` 内容截断 + `pickFiles` 可注入。
- [x] 宿主 command：`import_attachment { name, bytesB64, mimeType, workspaceId? } -> { path }`（`attachment_import.rs`：name 净化、写 `<workspace_root>/.tmp/imports/`、fallback `temp_dir()/pony-agent/`）+ `get_workspace_root`；注册进 control plane 与 Tauri `invoke_handler`。
- [x] `WorkspaceComposer.vue`：别针按钮（`data-testid="workspace-attach-button"`）、`pickFiles()` 文件选择、待发送附件条（预览/移除/错误态 chip）、空消息+附件可发送、图片第 4 张拒绝、附件 notice 提示。
- [x] `runtime.ts`：`pendingAttachments` 状态 + `set/clear/removePendingAttachments` + `addPendingAttachments(files)`（类型解析/上限/去重/魔数嗅探/导入）；`submitTurn` 扩展：图片 → `images`，文本 → 内容注入 + 附件元数据，文档 → 引用元数据；附件-only 自动摘要；发送后清空；`switchSession`/`createSession` 清空。
- [x] `messages.ts`：`buildAttachmentMessageBlocks` / `buildProviderUserMessageWithAttachments` / `buildDisplayedUserMessageWithAttachments` / `buildAttachmentMetas` 附件摘要与消息构建。
- [x] 单元测试：`tests/file-attachments.spec.ts` 28 例（类型矩阵/MIME 冲突/无扩展名/大小写/魔数嗅探/截断/去重/图片上限/浏览器二进制文档拒绝/导入成功失败/大小上限/清除动作/消息构建）。
- [x] `npm run test:unit`（363 通过）、`npm run build`（通过）、`npm run cargo:check:shared`（通过）、core lib 738 全绿（含新增 attachment_import 5 例）。

## Validation Notes

- 3 路对抗审核（2026-08-09）已采纳：宿主侧导入复制（A1）、文本/二进制文档路由拆分（A2）、图片上限对齐 3（B3）、MIME 来源与伪造（R-12/T-11）、附件-only 摘要与生命周期清空（R-13/R-14）、受控 tmp 布局与 PA-080 逐字一致（T-4/C-15）、`import_attachment` 携带 `workspaceId`（N-1）、name 净化（N-3）、浏览器模式二进制文档拒绝（N-3）等。
- 实现完成（2026-08-09）：全部任务完成并通过全量验证。
- 实现后 3 路对抗审核（2026-08-09）已采纳：后端 `merge_session_attachment_assets` 守卫不物化 doc/text 幽灵资产（F-1 + 回归测试）、`workspace_id` 本轮显式拒绝非默认值（F-2/T-2，PA-079 承接注册表解析）、`truncateTextByBytes` 二分（F-3）、宿主返回 `relativePath`（F-4/T-1）、override 移出 Tauri IPC（F-6）、空文本跳过导入（T-8）、`mimeMatches` 支持 `.*` 通配（T-9）、`resetSessionRuntimeState`/`createSession` 清空（F-9/T-5）、补 submitTurn 端到端/会话切换清空/Composer 组件测试（T-4/T-5/T-6）。
- 记录为后续候选：当前轮 doc/text attachment metas 仅存 localStorage（F-5，host session-patch 后补）、`.tmp/imports/` 保留期与清理策略（F-7）、`get_workspace_root` 为最小接缝、PA-079 的 `workspace_list` 将取代其消费方（F-8）。
