# 任务拆解（依赖序）

## Phase 0——流程门禁 ✅

- [x] P0-1 本变更目录（proposal/design/tasks + 两份 spec delta）落稿
- [x] P0-2 ADR 0015（持久化字段 ×2、dialog 插件、附件 resolve 分叉记录）+ decisions README 索引
- [x] P0-3 Phase 0 对抗审核（双 reviewer）与采纳调优（P0×3：L2614 投影点、激活语义退役、default 行反向表述；P1/P2 全部采纳）

## Phase 1——后端（types → workspace/store → 命令接线 → 测试）✅

- [x] B1 `types.rs`：`SessionState.titleOverride` / `SessionState.archived`（serde default + skip_serializing_if）+ `effective_title()` + `SessionOverview.archived` 投影位
- [x] B2 `store.rs`：`list_sessions`/`snapshot` live 分支改用 effective_title；**`snapshot_at`/`snapshot_at_readonly` 选中节点分支（L2614）替换为 effective_title(session)**；`refresh_session_metadata` 在 override 存在时跳过两分支的 title 赋值（trace 分支 + build_title 分支）；hydrate 两处无害化回归验证
- [x] B3 `workspace.rs`：`rename_workspace_entry` / `delete_workspace_entry`；**create_workspace_entry 同步采纳命名校验（trim 非空 + ≤64 + 对其他工作区重名拒绝）**；单测矩阵（空白/超长/重名/未知id/default 拒删/id-root 不变/create 新规则）
- [x] B4 `store.rs`：`rename_session`（同值 no-op）/ `archive_session`（幂等）/ `rename_workspace` / `delete_workspace`（名下会话归属批量重写为 default 后 save_to_backend）
- [x] B5 control_plane 方法 + lib.rs 注册 `workspace_rename` / `workspace_delete` / `session_rename` / `session_archive`（camelCase 参数、rwlock poison-recovery 包装与 workspace_commands.rs 一致）
- [x] B6 cargo 测试矩阵：override 六类消费点位回归（含 rename→续轮→checkout 改名前节点→snapshot.title == override）；两种持久化模式往返（LegacyBlob/WriteSeparate）；旧 blob 无新字段解析兼容；无 override 派生标题钉住（首条用户消息 28 字符省略号）；archive 幂等与重启保持；delete 后 resolve 成功；遗留孤儿库启动渲染契约

## Phase 2——前端基础件（与 Phase 1 并行）

- [ ] F1 tauri-plugin-dialog 四件套接线（Rust dep + `.plugin()` 注册 + capability `dialog:allow-open` 最小权限 + npm 包）；capability JSON 变更冒烟断言
- [ ] F2 `ui/DropdownMenu.vue`（reka-ui DropdownMenu 系列）
- [ ] F3 `ui/ConfirmPopover.vue` 升级：受控 open（defineModel）+ loading + 错误槽；confirm 不再立即关闭；**保持非受控用法向后兼容**（ProviderConfigPage 存量消费方零破坏）
- [ ] F4 `workspace-api.ts` 四封装 + 目录选择封装；`types/runtime.ts` SessionOverview.`archived?`

## Phase 3——侧边栏重建（依赖 Phase 1+2）

- [ ] F5 `sidebar-groups.ts` → `deriveSidebarTree()`（平铺区三分成员契约 + 组序 + 瞬态钉顶 + 计数排除归档 + 零计数组）；UNGROUPED_GROUP_KEY deprecated
- [ ] F6 HomeSessionSidebar 单一树重构（一级标题行/FolderPlus 按钮/Tooltip「添加工作区」、二级行解剖、三级行截断 + hover 全文、三点菜单、预览上限分区各自 5 条 + 全局显示全部、旧双 section 与激活徽标清除）；截断/hover/图标/tooltip 断言进组件测试
- [ ] F7 store actions：createSession 显式 target 参数（顶层= default，组内= 该组）+ **删除工作区时归一存活创建目标与瞬态条目**；renameWorkspace/deleteWorkspace（成功后激活态归一）/archiveSession（当前激活会话复用 deleteSession fallback 选段）/renameSession + inflight 守卫 set
- [ ] F8 文案集中 `lib/runtime/sidebar-copy.ts`（design 文案基线全量落地，含 {N}=0 规则与禁用原因 tooltip）
- [ ] F9 浏览器降级矩阵落地（管理面隐藏；会话菜单仅保留删除；rail 无需额外降级）

## Phase 4——清理与收口

- [ ] C1 更新 `tests/sidebar-groups.spec.ts`（契约重写）、`tests/HomeSessionSidebar.spec.ts`、`tests/runtime-store.spec.ts`、`tests/ProviderConfigPage.spec.ts`（ConfirmPopover 兼容回归）、ConfirmPopover/DropdownMenu 组件测试
- [ ] C2 废弃折叠 localStorage key 的读写路径清理（停止写入；残留 key 无害说明）
- [ ] C3 门禁：vue-tsc --noEmit / vitest 全绿 / cargo check / cargo:test:lib / 手工冒烟（增删改名归档全链路 + 重启持久化 + ≥100 会话卡顿测量点 + 提交中窗口期菜单禁用一例）
- [ ] C4 双 reviewer 终审 + spec delta 合入 openspec/specs（关键词加粗样式统一已在本 delta 内完成）+ proposal 验收逐条核对 + ADR 事实同步核对
