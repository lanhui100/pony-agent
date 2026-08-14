# 2026-08-13 PA-078/079 收口归档 + 前端优化收口

## 会话目标

1. 原子化提交工作区全部未提交改动（PA-078/079 实现 + 前端流式渲染优化 + 任务文档）
2. 收口归档：PA-078/079 的 OpenSpec changes + 3 个已完成 change（optimize-agent-stream-ui / stabilize-markdown-stream-rendering / fix-provider-registry-fallback-test-baseline）
3. 同步任务系统状态，为 PA-080 实现做准备

## 执行过程

### 原子提交（6 个提交）

| 提交 | 内容 |
|---|---|
| `a118786` | feat(core,pa-078,pa-079): workspace 数据模型与附件导入（attachment_import/workspace 模块、workspace_id 持久化与投影、宿主控制面接线） |
| `0f7bf94` | feat(host,pa-078,pa-079): 宿主命令接线（get_workspace_root/import_attachment/workspace_list/workspace_create + probe 参数同步） |
| `57519c4` | feat(frontend,pa-078,pa-079): 附件入口与 workspace 前端接线（pendingAttachments 状态流、文件类型路由、workspaceId 传输与持久化投影） |
| `57519c4` | perf(frontend): 流式渲染与 trace timeline 缓存优化（turn 级 memo、semantic-event timeline 复用、debug log 缓存与新建对话可用性） |
| `docs` | docs(tasks): PA-078~081 任务卡、审核记录与 OpenSpec changes |
| `bfd4f26` | chore: 扩展本地 Claude 权限白名单 |

注：提交过程中 pre-commit 版本治理 hook 自动 bump 版本（.version.json/package.json/Cargo.toml），符合仓库既有机制。

### 收口归档

- PA-078 change `add-chat-file-attachment-entry` → `openspec/changes/archive/2026-08-09-add-chat-file-attachment-entry/`
- PA-079 change `workspace-data-model-and-registry` → `openspec/changes/archive/2026-08-09-workspace-data-model-and-registry/`
- `optimize-agent-stream-ui` → `openspec/changes/archive/2026-08-09-optimize-agent-stream-ui/`
- `stabilize-markdown-stream-rendering` → `openspec/changes/archive/2026-08-09-stabilize-markdown-stream-rendering/`
- `fix-provider-registry-fallback-test-baseline` → `openspec/changes/archive/2026-08-09-fix-provider-registry-fallback-test-baseline/`
- canonical specs 同步（delta → canonical 格式转换，全部通过 openspec validate）：
  - `openspec/specs/chat-file-attachment-entry/spec.md`
  - `openspec/specs/workspace-data-model/spec.md`
  - `openspec/specs/chat-ui/spec.md`
  - `openspec/specs/provider-registry/spec.md`

### 任务系统同步

- 任务卡 PA-078/079：OpenSpec 引用更新为归档路径，Next Action 标记已收口
- 任务板：PA-078/079 归档引用更新；新增 PA-082（前端流式渲染优化收口）、PA-083（provider registry 回归基线对齐）Done 记录
- Dashboard：主线状态更新（2026-08-13 归档完成）

## 验证

- `npm run openspec -- validate --changes`：2 passed（PA-080/081 活跃 change）
- 4 个新 canonical specs 单独 validate 全部通过
- 剩余工作区改动仅为 LF→CRLF 行尾噪音（autocrlf=true 下不产生提交内容）

## 下一步

- 实现 PA-080（Workspace 路径权限边界，P0）：按 `openspec/changes/workspace-path-permission-boundary/tasks.md` 顺序，先读 `crates/pony-agent-core/src/agent/sandbox.rs`、`tools.rs` 的 `ToolPermissionScope`/`ApprovalRequired`、`docs/concurrency/lock-ordering.md`

---

# 追加：PA-080 实现（2026-08-13 续）

## 执行过程

- 委派 @backend-dev 实现（子代理返回空结果，但实际产出完整：path_permission.rs 976 行 + 11 个文件接入）
- 修复 2 处编译错误：`control_plane/mod.rs` 多余 `)`（build_governed_executor 签名变化）、`src-tauri/src/lib.rs` `list_authorizations` 缺 `.collect()`
- 补 `docs/concurrency/lock-ordering.md` 锁序登记（tasks.md 第 9 项，子代理遗漏）：`path_authorizations` RwLock 与 sessions_rwlock 同级、先 registry 后 authorize、判定闭包内不得获取 sessions 写锁
- 自行完成实现后审查（子代理环境异常）：核心判定逻辑（组件级比较/ParentDir 拒绝/祖先链止于卷根/精确 revoke/可注入 canonicalizer）、工具接入完整性、错误码传播（`outside_workspace_write_denied`/`requires_authorization` 进入工具错误消息）、锁序一致性全部核对通过

## 验证结果

- `npm run cargo:check:shared`：通过
- core lib：**772 passed**（含 path_permission 18 项对抗测试：穿越/前缀混淆/Windows 双断言/卷边界/symlink 逃逸+hermetic/全新嵌套目录/`..` 后缀/授权 grant-revoke 循环/目录授权覆盖子孙/卷根授权/持久化回调/真 symlink canary）
- tool_router_regression 13 + session_regression 5 + provider_registry_regression 8 + src-tauri lib 6：全绿
- 前端 vitest：377 passed
- 提交：`2db0b0e` feat(core,pa-080)

## 下一步

- PA-080 实现后 3 路对抗审核（@consultant 权限模型 / @code-reviewer 路径安全 / @tester 对抗矩阵复核）
- 审核通过后收口归档 PA-080，启动 PA-081（侧边栏 Workspace 树导航）

---

# 追加：PA-080 实现后审核 + 修复 + 收口（2026-08-13 续 2）

## 实现后 3 路对抗审核

- **@code-reviewer**（路径安全）：发现 1 P0 + 2 P1 + 若干 P2/P3
  - P0：`workspace_edit_file` 用读判定解析目标后写盘 → 只读授权可升级为外部写
  - P1-1：`workspace_run_command` cwd 可落在授权外部目录
  - P1-2：会话级 root 未接线（判定锚定进程 cwd）
- **@consultant**（权限模型架构）：判定 **FAIL**，核心问题
  - P0：`session_workspace_root: RwLock` ambient state 在并发多会话下串扰 → 采纳**方案 A（显式不可变 `ToolExecutionContext`）**
  - P0：生产调用链未按 workspaceId 解析 root；相对读路径固定默认 root
  - P1：错误码编码进 message 字符串而非顶层错误码；fallback tmp 未接入权限 checker；授权持久化失败不可上报

## 修复内容（commit `a1b75d8`）

1. **P0 edit 写判定**：edit_file 改用 `classify_path(Write)`，外部授权文件编辑返回 `outside_workspace_write_denied`（只读授权不可升级为写）
2. **P1-1 Run cwd**：新增 `resolve_workspace_dir_for_execution`（Write 语义），授权外部目录不可作 cwd；sandbox 基准改用会话 root
3. **P1-2/P0 会话 root（方案 A）**：删除 ambient RwLock，引入 `ToolExecutionContext`（`workspace_root: Option<PathBuf>`）显式贯穿 `execute_internal` → 工具方法 → 路径 helper → classify；`RouterPrimitiveHandler` 经 `PrimitiveToolHandlerRequest.workspace_root` 透传 dispatcher 的 `DispatchContext.workspace_root`；`apply_governed_turn_context` 按 `input.workspace_id` 从 workspace 注册表解析会话 root（生产链路补齐）
4. **错误码结构化信封**：`classify_workspace_path` 返回 `(code, message)`，权限码（`requires_authorization` / `outside_workspace_write_denied` / `permission_denied`）直接作为工具 error.code 顶层字段
5. **fallback tmp 覆盖**：`controlled_tmp_dirs` 纳入 PA-078 的 `<temp_dir>/pony-agent/.tmp/imports/` fallback 布局
6. **相对读路径**：`path_permission::classify` 的 Read 相对路径按 root 基座拼接（防进程 cwd 漂移）

## 回归测试（新增 3 项）

- `edit_file_rejects_authorized_external_path_with_write_denied`（P0 回归：读授权 + edit → 拒绝、文件未篡改）
- `run_command_rejects_authorized_external_cwd`（P1-1 回归）
- `concurrent_calls_keep_session_workspace_roots_isolated`（P0 并发串扰回归：两线程并发写各自会话 root 无串扰）

## 最终验证（2026-08-13）

- core lib **775** 全绿（含 path_permission 18 项 + 新增 3 项回归）
- tool_router_regression 13 + session_regression 5 + provider_registry 8 + src-tauri lib 6 全绿
- 前端 vitest 377 全绿；cargo check 无 warning

## 收口归档（2026-08-13）

- PA-080 change 迁入 `openspec/changes/archive/2026-08-13-workspace-path-permission-boundary/`
- canonical spec 同步：`openspec/specs/workspace-path-permission/spec.md`（delta→canonical，validate 通过）
- 任务卡 PA-080 状态 Done，任务板 In Progress 清空，dashboard 主线更新

## 下一步

- 启动 **PA-081**（侧边栏 Workspace 树导航，P1，spec 已通过）：`HomeSessionSidebar` 平铺列表 → "项目（二级）→ 对话（三级）"树，按 workspaceId 归组、折叠/展开、Workspace 管理入口