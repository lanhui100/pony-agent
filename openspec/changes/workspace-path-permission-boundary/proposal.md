# workspace-path-permission-boundary

## Background

现有权限模型只有 scope 级（`ToolPermissionScope`：WorkspaceRead/Write/Execute，`agent/tools.rs:199`）+ `requires_approval` 审批语义，没有路径级判定。文件工具各自实现 `resolve_inside_workspace`（canonicalize + `Path::starts_with` 前缀校验，`document_conversion.rs:172`、`image_artifact.rs:130`），但：

- 各工具自行复制该逻辑，无统一模块；
- "workspace 之外能否读/写"没有显式策略：读会被 canonicalize 拒绝，但没有"授权后放行"的机制；
- 写权限没有边界定义（workspace 外写文件是否允许无明确判定）。

Workspace 限定文件夹后需要正式回答：写权限默认仅 workspace 根 + 受控 tmp；workspace 之外读取需显式授权（授权清单持久化）。这是安全敏感改动。

3 路对抗审核（2026-08-09）确认的关键约束：

- **`Path::starts_with` 在 Windows 上大小写敏感**（已在本机实测）；原设计"忽略大小写"论断错误。lowercase 字符串前缀比较会重新引入前缀混淆。正确做法：Windows 上对两侧路径做**组件级 ASCII 小写折叠后逐组件比较**（绝不整体字符串前缀）。
- **写新文件判定须复用既有 `prepare_workspace_file_path` 语义**（`tools.rs:3106-3140`：向上找最近存在祖先 → canonicalize → 组件级校验后缀无 `.`/`..` → join）；原"父目录 canonicalize + 文件名 join"在父目录不存在时拒绝，会破坏已回归的"写入全新嵌套目录"行为（回归 #8）。
- **Run 无输出路径参数**（`tools.rs:1375-1419` 只有 command/cwd/timeoutMs）；进程内部写入无法用路径判定约束，归 PA-077 沙箱边界；本卡只判定 `cwd`。
- **授权清单 scope `"read-write"` 是死代码**（写路径不查授权清单）；本轮 `authorize_path` 仅接受 `"read"`，`"read-write"` 返回显式"暂不支持"。

## Goals

- 建立统一路径权限模块 `agent/path_permission.rs`：路径规范化 + 前缀校验 + 授权清单判定，全部文件工具共用。
- 写权限默认仅授予 workspace 根（递归）与受控 tmp 目录；其余位置一律拒绝（结构化错误）。
- 读权限：workspace 内自由；workspace 之外需授权清单命中（用户显式批准，持久化）或返回结构化错误（审批语义入口）。
- 防 `..` 穿越、符号链接逃逸、前缀混淆；Windows 上组件级大小写折叠比较。
- 既有工具回归集不回归（tool_router_regression 13 项）。

## Non-goals

- 不做完整 SandboxBackend / Windows Job Object containment（PA-077 承接）。
- 不做前端授权审批 UI（本卡提供结构化错误语义 + API 面；UI 记录为后续候选）。
- 不改变既有工具名、结果合同与 dispatch 流程。
- 不做多 workspace 权限矩阵（PA-079 提供注册表后按需扩展；本卡 root 在调用时按会话 `workspace_id` 经注册表解析）。
- 不接 Ask/`PendingControlRequest` 审批流（本卡错误语义先返回，重审-重发流见 spec；Ask 集成后续承接）。

## Scope

- 新模块 `agent/path_permission.rs`：`PathPermission`（canonicalize + 组件级前缀校验 + 授权判定）、`PermissionZone`（WorkspaceRoot / ControlledTmp / AuthorizedExternal / Denied）、授权清单（`AuthorizedPathEntry { path, granted_at_ms }`，SQLite `store_metadata` key=`path_authorizations.v1` 或独立表持久化）。
- `tools.rs` 文件类工具（Read/List/Glob/Grep/Write/Edit/ReadDocument/ViewImage）接入统一判定；写新文件复用 `prepare_workspace_file_path` 语义。
- 写操作（Write/Edit/Run 的 `cwd`）路径边界：workspace 根（递归）+ 受控 tmp（`<workspace_root>/.tmp/` 或稳定 fallback `temp_dir()/pony-agent/`）。
- 授权 API：`authorize_path(path, scope)`（本轮仅 `"read"`）/ `revoke_authorization(path)` / `list_authorizations()`（宿主 command）。
- 对抗测试：穿越/软链/大小写/前缀混淆/全新嵌套目录写入/文件名 `..`/授权 grant→read→revoke→拒绝/tmp 写放行/workspace 外写拒绝/卷边界。

## Risks

- canonicalize 依赖文件存在：对"即将创建"的写路径（Write 到新文件/新目录）canonicalize 会失败。缓解：复用 `prepare_workspace_file_path`（向上找最近存在祖先 → canonicalize → 组件级校验后缀无 `.`/`..` → join）；受控 tmp 目录允许懒 `create_dir_all`。
- 符号链接在 workspace 内指向外部：canonicalize 会解析到外部 → 拒绝（正确行为）；但允许用户显式授权目标路径（授权清单命中放行）。测试用**可注入 canonicalizer**（hermetic fake resolver）确定性验证，另留 `#[cfg(unix)]`/Windows junction 真链 canary（无权限则跳过）。
- TOCTOU：校验与使用之间路径变化。缓解：与 PA-069-B 一致，判定结果与执行在同一闭包内，接受窗口内竞态（与既有 `resolve_inside_workspace` 一致）；并发授权变更同样记为已接受的 TOCTOU，不承诺强一致（文档化，不作确定性并发测试）。
- Windows 大小写不敏感/UNC/卷差异：组件级比较，Windows 上两侧先 ASCII 小写折叠；卷边界由组件语义天然处理（`C:` vs `D:` 首组件不同）；`\\?\` 前缀在 canonical 输出归一化去除；UNC 共享 CI 无可用则测试标注跳过。
- 授权路径不存在：拒绝授权（存在才可授权）；授权父目录删除后子路径仍命中（条目保留，判定基于组件前缀；目标不存在直接拒绝 IO）。
- `authorize_path` scope：本轮仅 `"read"`；传 `"read-write"` 返回显式"暂不支持"，不产生"看似授权实际无效"的静默条目。

## Validation

- 对抗测试矩阵全部通过；tool_router_regression 13 项不回归；core lib 全绿；`npm run cargo:check:shared` 通过。
- 授权清单持久化 roundtrip 测试；revoke 后立即拒绝测试；Windows 双断言（`c:\ws\file` ⊂ `C:\WS` 放行 AND `c:\ws-10` ⊄ `C:\WS` 拒绝）。
