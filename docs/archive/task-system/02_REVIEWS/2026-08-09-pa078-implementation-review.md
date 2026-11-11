# PA-078 实现后审核（3 路对抗）

- 日期：2026-08-09
- 阶段：实现后审核 → 验证收口
- 审核对象：PA-078 实现（`file-attachments.ts` / `messages.ts` / `runtime.ts` / `WorkspaceComposer.vue` / `attachment_import.rs` / control plane + tauri command / 测试）
- 方式：3 路对抗审核子智能体（@consultant / @code-reviewer / @tester）独立审核实现代码与 spec 基准

## 阵容与结论

| 角色 | 结论 | 关键发现（P0/P1） | 处置 |
| --- | --- | --- | --- |
| `@consultant` | 有条件通过 | F-1（doc/text 幽灵资产）、F-2（workspaceId 无 owner）、F-3（截断 O(n²)）、F-4（fallback relativePath） | 全部采纳并修复 |
| `@code-reviewer` | 有条件通过 | I-1（截断 O(n²)，与 F-3 同源）、I-2（图片 mime 未按嗅探修正）、I-3（fallback relativePath，与 F-4 同源） | 全部采纳并修复 |
| `@tester` | 有条件通过 | T-1（fallback relativePath）、T-2（workspaceId 静默丢弃）、T-3（spec Import fails 矛盾）、T-4/T-5/T-6（submitTurn 端到端/会话切换清空/Composer 组件零测试）、T-7（host 单测并行 flaky）、T-8（空文本双模式不一致） | 全部采纳并修复 |

三路均无 P0；判定均为"有条件通过"，无未解决 P0/P1 后收口。

## 采纳记录

### P1（全部修复 + 回归测试）

- **F-1 / 幽灵资产**：后端 `merge_session_attachment_assets` 加 `is_asset_eligible_attachment` 守卫——跳过无 asset_id 且 relative_path 非 `<session_id>/` 布局的引用（doc/text 的 `.tmp/imports/...` 不再物化为 `MissingPayload` asset）；图片资产保留。回归测试 `doc_text_reference_attachments_do_not_materialize_phantom_assets`。
- **F-2 / T-2 / workspaceId**：`import_attachment` 本轮显式拒绝非默认 workspaceId（`import_attachment_unsupported_workspace`），避免"参数存在但静默无效"；Rust 测试；PA-079 已承接"注册表 root 解析"owner（proposal/tasks）。
- **F-3 / I-1 / 截断**：`truncateTextByBytes` 改二分查找截断点，消除 O(n²) 主线程冻结。
- **F-4 / T-1 / I-3 / fallback relativePath**：宿主 `ImportAttachmentResult` 返回 `relative_path: Option<String>`（root 下 `.tmp/imports/<name>`，fallback 为 None）；前端 `ImportResult.relativePath` 直接采用宿主值，不再硬编码。
- **T-3 / spec Import fails**：spec 场景同步为"文件以错误态进入 pending 列表，可移除且不参与发送"。
- **T-4**：补 submitTurn 附件端到端测试（浏览器模式，含图片+文本、附件-only 自动摘要）。
- **T-5**：补会话切换/新建清空测试（`createSession` 清空提前到 guard 前，覆盖空消息 no-op 场景）。
- **T-6**：新增 `tests/workspace-composer-attach.spec.ts`（5 例：按钮渲染/打开选择器/chip/remove/notice/空消息+附件可发送，mock `pickFiles`）。
- **T-7**：host 单测改用唯一临时目录，消除并行 flaky。
- **T-8 / I-9 / 空文本**：前端对 0 字节文本跳过导入（path/relativePath 为 null、内容为空），双模式一致；测试覆盖。
- **I-2 / 图片 mime**：`mimeForSniffed` 由魔数嗅探结果派生 dataUrl mime（"PNG 内容改名 .jpg" 不再产出错误 mime）；测试覆盖。

### P2（采纳，含部分文档化）

去重键并入 `lastModified`（I-4/T-10）；失败 chip 不占图片上限、不参与去重（I-5）；在途导入与 submitTurn 竞态守卫 `if (isSubmitting) break`（I-6）；用户文本在前、附件块在后（I-10，design 同步）；`mimeMatches` 支持 `.*` 通配（T-9）；空图片拒绝测试（T-10）；`root`/`import_dir` override 移出 Tauri IPC（F-6）；`resetSessionRuntimeState` 清空（F-9）；`get_workspace_root` 保留为接缝、PA-079 的 `workspace_list` 取代消费方（F-8）；`.tmp/imports/` 保留期与清理、当前轮 metas 仅存 localStorage、bytesB64 大文件 IPC、跨轮文本内容保留记录为后续候选（F-5/F-7/I-7/I-8）。

### P3（采纳）

后端存储用 trim 后的 name（I-11）；`bytesToBase64` subarray 防御（I-12）；图片 `sizeBytes` 单位不一致记录为既有行为（I-13）。

## 验证

| 门 | 结果 |
| --- | --- |
| vue-tsc | ✅ 无错误 |
| 前端 vitest | ✅ 377 通过（10 skipped） |
| `npm run build` | ✅ |
| `cargo:check:shared` | ✅ |
| core lib | ✅ 740 通过 |
| OpenSpec validate（add-chat-file-attachment-entry / workspace-data-model-and-registry） | ✅ valid |

## 结论

**PASS** — PA-078 实现通过 3 路对抗审核，无未解决 P0/P1。实现与修订后 spec 一致，跨卡接缝（受控 tmp 布局、workspaceId、AttachmentAsset 收窄）已落实或明确 owner/后续候选。可进入归档与状态收口；下一张卡 PA-079（workspace 数据模型）可启动实现。
