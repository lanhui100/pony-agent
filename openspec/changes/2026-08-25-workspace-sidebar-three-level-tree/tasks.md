# 任务拆解（依赖序）

## Phase 0——流程门禁 ✅

- [x] P0-1 本变更目录（proposal/design/tasks + 两份 spec delta）落稿
- [x] P0-2 ADR 0015（持久化字段 ×2、dialog 插件、附件 resolve 分叉记录）+ decisions README 索引
- [x] P0-3 Phase 0 对抗审核（双 reviewer）与采纳调优

## Phase 1——后端（types → workspace/store → 命令接线 → 测试）

- [ ] B1 `types.rs`：`SessionState.titleOverride` / `SessionState.archived`（serde default + skip_serializing_if）+ `effective_title()` + `SessionOverview.archived` 投影位
- [ ] B2 `store.rs`：`list_sessions`/`snapshot` 改用 effective_title；`refresh_session_metadata` 在 override 存在时跳过两分支的 title 赋值（L3755 trace 分支 + L3764 build_title 分支）；hydrate 两处（node.title 回灌）无害化验证
- [ ] B3 `workspace.rs`：`rename_workspace_entry` / `delete_workspace_entry` + 校验单测（空白/超长/重名/未知id/default 拒删/id-root 不变）
- [ ] B4 `store.rs`：`rename_session` / `archive_session`（幂等）/ `rename_workspace` / `delete_workspace`（名下会话归属批量重写为 default 后 save_to_backend）
- [ ] B5 control_plane 方法 + lib.rs 注册 `workspace_rename` / `workspace_delete` / `session_rename` / `session_archive`（camelCase 参数、rwlock poison-recovery 包装与 workspace_commands.rs 一致）
- [ ] B6 cargo 测试矩阵：override 五类消费点位回归、两种持久化模式往返（LegacyBlob/WriteSeparate）、旧 blob 无新字段解析兼容、archive 幂等与重启保持、delete 后 resolve 成功

## Phase 2——前端基础件（与 Phase 1 并行）

- [ ] F1 tauri-plugin-dialog 四件套接线（Rust dep + `.plugin()` 注册 + capability `dialog:allow-open` + npm 包）
- [ ] F2 `ui/DropdownMenu.vue`（reka-ui DropdownMenu 系列）
- [ ] F3 `ui/ConfirmPopover.vue` 升级：受控 open（defineModel）+ loading + 错误槽；confirm 不再立即关闭
- [ ] F4 `workspace-api.ts` 四封装 + 目录选择封装；`types/runtime.ts` SessionOverview.`archived?`

## Phase 3——侧边栏重建（依赖 Phase 1+2）

- [ ] F5 `sidebar-groups.ts` → `deriveSidebarTree()`（平铺区契约 + 组序 + 孤儿并入 + 瞬态钉顶 + 零计数组）；UNGROUPED_GROUP_KEY deprecated
- [ ] F6 HomeSessionSidebar 单一树重构（一级标题行/添加按钮/Tooltip、二级行解剖、三级菜单、预览上限分区各自 5 条 + 全局显示全部、旧双 section 状态清除）
- [ ] F7 store actions：renameWorkspace/deleteWorkspace（成功后 normalizeActiveWorkspace）/archiveSession（当前激活会话复用 deleteSession fallback 选段）/renameSession + inflight 守卫 set；顶层新对话显式传 default
- [ ] F8 文案集中 `lib/runtime/sidebar-copy.ts`（删除三要素句、归档不可恢复句、工具根披露句）
- [ ] F9 浏览器降级矩阵落地（含 rail 态）

## Phase 4——清理与收口

- [ ] C1 更新 `tests/sidebar-groups.spec.ts`（契约重写）、`tests/HomeSessionSidebar.spec.ts`、`tests/runtime-store.spec.ts`
- [ ] C2 废弃折叠 localStorage key 的读写路径清理（停止写入；残留 key 无害说明）
- [ ] C3 门禁：vue-tsc --noEmit / vitest 全绿 / cargo check / cargo:test:lib / 手工冒烟（增删改名归档全链路 + 重启持久化 + ≥100 会话卡顿测量点）
- [ ] C4 双 reviewer 终审 + spec delta 合入 openspec/specs + proposal 验收逐条核对
