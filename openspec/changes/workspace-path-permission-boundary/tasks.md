# Tasks

- [ ] 新建 `crates/pony-agent-core/src/agent/path_permission.rs`：`PermissionZone` / `PathPurpose` / `PermissionError { code, message }` / `classify_path`（可注入 canonicalizer）/ `AuthorizedPathEntry` + `AuthorizeStore`（`HashMap` + SQLite `store_metadata` key=`path_authorizations.v1` + JSON fallback，每次变更即写）。
- [ ] 组件级前缀比较 + Windows ASCII 小写折叠（每组件折叠后逐组件比较，绝不用整体字符串前缀）；canonical 输出归一化去除 `\\?\`。
- [ ] 写新文件判定复用 `prepare_workspace_file_path` 语义（最近存在祖先 canonicalize + 后缀组件校验）；受控 tmp 懒 `create_dir_all`。
- [ ] 工具接入：ReadDocumentHandler / ViewImageHandler / workspace_read_file / read_file_segment / list_files / search_text / glob_files 改调 `classify_path(Read)`。
- [ ] 写工具接入：workspace_write_file / workspace_edit_file 改调 `classify_path(Write)`；workspace_run_command 只判 `cwd`（不承诺约束进程内输出落盘）。
- [ ] 会话级 root 接线：工具执行上下文（`ToolCall` 或执行参数）携带 `workspaceId`/resolved root；`ToolRouter` 全部 `workspace_root` 用途（`display_workspace_relative`、Run `cwd` 等）统一随会话 root；无上下文回退构造默认 root；孤儿 `workspace_id` → 读回退默认 root 带告警、写 fail-closed。
- [ ] 新锁登记：`AuthorizeStore` RwLock 加入 `docs/concurrency/lock-ordering.md`（置于 `sessions_rwlock` 之上，与 WorkspaceRegistry 同级，先 registry 后 authorize）；判定闭包内不得再获取 sessions 写锁。
- [ ] 宿主 command：`authorize_path(path, scope)`（仅 `"read"`，`"read-write"` 显式拒绝）/ `revoke_authorization(path)` / `list_authorizations()`（control plane 注册 + 审计面）。
- [ ] 对抗测试矩阵（全部经可注入 canonicalizer 确定性执行）：
  - `..` 穿越；符号链接逃逸（含授权放行、hermetic fake resolver 双路径）；前缀混淆（`/ws-1` vs `/ws-10`）；
  - Windows 双断言：`c:\ws\file` ⊂ `C:\WS`（放行）AND `c:\ws-10` ⊄ `C:\WS`（拒绝）；
  - `\\?\` / UNC（无共享则跳过）/ 卷边界（`C:` vs `D:`）；
  - 写全新嵌套目录；文件名后缀含 `..` 拒绝；
  - 授权 grant→read→revoke→拒绝；文件授权不覆盖兄弟；目录授权覆盖子孙；revoke 精确路径（子授权保留）；授权 `/` 或卷根（文档化语义 + 测试）；
  - `authorize_path("read-write")` 显式拒绝；
  - tmp 写放行；workspace 外写拒绝（断言 `outside_workspace_write_denied`）；workspace 外读无授权（断言 `requires_authorization`）。
- [ ] 真符号链接 canary：`#[cfg(unix)]` 或 Windows junction，无特权则跳过（保留为真实环境验证）。
- [ ] 回归：tool_router_regression 13 项（`cargo test --test tool_router_regression`）+ core lib 全绿 + 前端 vitest + `npm run cargo:check:shared`。

## Validation Notes

- 3 路对抗审核（2026-08-09）已采纳：Windows 组件级大小写折叠 + 双断言（T-1/C-8/R-1）、写新文件复用 `prepare_workspace_file_path` + 全新嵌套目录/`..` 后缀（T-13/R-2）、`\\?\` 归一化（R-3）、scope 仅 read（R-4/C-18）、revoke 精确路径（R-5）、祖先链止于卷根（R-6）、Run 只判 cwd（R-7）、可注入 canonicalizer + hermetic symlink 测试（T-8）、错误码断言 + "before any IO"改写（T-9）、文件/目录授权边界（T-14）、UNC/卷边界声明对齐（T-17）、审批重发流与错误信封（C-9/R-C-3）、每会话 root 解析（C-7）。
