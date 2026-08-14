# Design

## Decision Summary

1. 新模块 `agent/path_permission.rs` 成为文件工具路径判定的唯一真相源，吸收现有 `resolve_inside_workspace` 实现并升级为"读/写分区 + 授权清单 + Windows 大小写折叠"。
2. 权限分区：`WorkspaceRoot`（写+读）/ `ControlledTmp`（写+读）/ `AuthorizedExternal`（读）/ `Denied`。
3. 判定核心：canonicalize（写新文件复用 `prepare_workspace_file_path` 语义）→ **组件级前缀比较**（Windows 上 ASCII 小写折叠，杜绝字符串前缀混淆）→ 未命中且 Read 时查授权清单（祖先链）。
4. 授权清单持久化：SQLite `store_metadata` key=`path_authorizations.v1`；JSON backend 独立文件；内存 `RwLock<HashMap<PathBuf, AuthorizedPathEntry>>`。
5. 宿主 command：`authorize_path(path, scope)`（本轮仅 `"read"`）/ `revoke_authorization(path)` / `list_authorizations()`，走 control plane 审计面。
6. 受控 tmp：`<workspace_root>/.tmp/`（与 PA-078 导入目录布局逐字一致），fallback 稳定非 PID 的 `temp_dir()/pony-agent/`；写路径允许懒 `create_dir_all`。
7. 判定 root 在调用时按会话 `workspace_id` 经 `WorkspaceRegistry` 解析（不依赖构造时固定 root）。**机制（复检 N-1 钉死）**：工具执行上下文（`ToolCall` 或执行参数）携带 `workspaceId`/resolved root；`ToolRouter` 中所有 `workspace_root` 用途（`display_workspace_relative`、`workspace_run_command` 的 `cwd`、`classify_path` 调用等）统一随会话 root 解析；无上下文（legacy 会话、直接 `execute()` 调用、回归套件）→ 回退构造函数时的默认 root。
8. `classify_path` 接收可注入 canonicalizer（`fn(&Path) -> io::Result<PathBuf>`），测试用 hermetic fake 确定性覆盖符号链接逃逸。

## Chosen Direction

### 1. 判定模型

```rust
pub enum PermissionZone { WorkspaceRoot, ControlledTmp, AuthorizedExternal, Denied }

pub struct PathPermission {
    pub zone: PermissionZone,
    pub canonical: PathBuf,          // 解析后的路径（归一化，Windows 去 \\?\ 前缀）
    pub matched_authorization: Option<AuthorizedPathEntry>,
}

pub fn classify_path(
    raw_path: &str,
    root: &Path,               // 调用时按会话 workspace_id 解析；无上下文 → 构造时默认 root
    tmp_dir: &Path,            // 受控 tmp 布局
    authorizations: &AuthorizeStore,
    purpose: PathPurpose,      // Read | Write
    canonicalize: impl Fn(&Path) -> io::Result<PathBuf>,  // 可注入，默认 fs::canonicalize
) -> Result<PathPermission, PermissionError>;
```

判定顺序：

1. **规范化**：Write 且目标不存在 → 复用 `prepare_workspace_file_path` 语义（向上找最近存在祖先 → canonicalize → 组件级校验剩余后缀无 `.`/`..`/分隔符 → join）；其余路径 canonicalize。canonical 输出归一化：Windows 上去 `\\?\` 前缀。
2. **组件级前缀判定**：`components_equal_within(root_components, canonical_components)` —— 逐组件比较；Windows 上每个组件先 ASCII 小写折叠。命中 root → `WorkspaceRoot`；命中 tmp_dir → `ControlledTmp`。**绝不整体字符串前缀比较**（杜绝 `/ws-1` vs `/ws-10`、`C:\ws` vs `C:\ws-10` 混淆）。
3. 未命中且 `purpose=Read`：查授权清单（目标自身 + 祖先链，止于卷根，不越过）→ 命中 → `AuthorizedExternal`；未命中 → 返回 `PermissionError::RequiresAuthorization`（审批语义入口）。
4. `purpose=Write` 未命中 → `Denied`（`PermissionError::OutsideWorkspaceWriteDenied`；授权清单本轮仅覆盖读，不提供外部写）。

### 2. 受控 tmp 目录

- 主路径：`<workspace_root>/.tmp/`（PA-078 导入目录 `imports/` 为其子目录，同源；两卡布局字符串逐字一致）。
- fallback：`std::env::temp_dir()/pony-agent/`（稳定非 PID，重启不换路径，避免 PA-078 导入目录/授权状态跨重启失联）。
- 判定与 workspace 根互斥：tmp 位于 workspace 内（`.tmp/` ⊂ root）→ 先命中 `WorkspaceRoot`（写允许）→ 同一放行集，无冲突。
- 写路径允许对 tmp 目录懒 `create_dir_all`（避免"目录未创建→判定失败→永远写不进"的死锁）。

### 3. 授权清单

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizedPathEntry {
    pub path: PathBuf,           // canonical 绝对路径（目录或文件），Windows 去 \\?\ 前缀
    pub granted_at_ms: u64,
}
```

- 授权路径自身 canonicalize（存在才可授权；不存在的目标拒绝授权）。
- 判定用祖先链匹配：目标 canonical 从自身逐级向上找父目录（止于卷根，不越过），任一命中授权即放行；文件授权只覆盖该文件，不覆盖其兄弟。
- **scope 本轮仅 `"read"`**：`authorize_path(path, scope)` 传 `"read-write"` 返回显式"暂不支持"错误，不产生"看似授权实际无效"的静默条目。字段不再预留写语义（避免死代码误导）。
- 撤销 = **精确路径**：revoke 只移除该路径条目，子路径授权条目保留（明确语义，后续可加子树级联）。
- 存储：内存 `RwLock<HashMap<PathBuf, AuthorizedPathEntry>>`（祖先查找 O(depth)，授权条目少时足够；`Vec` 线性扫描对祖先链查找更差，故用 HashMap）；持久化走 `SessionBackend::read_metadata/write_metadata`（`store_metadata` key=`path_authorizations.v1`），每次变更即写。
- 授权 `/` 或卷根：允许（用户显式授权），但必须文档化并测试；祖先链止于卷根不越过。

### 4. 工具接入

- `ReadDocumentHandler` / `ViewImageHandler` 现有 `resolve_inside_workspace` 改调 `classify_path(purpose=Read)`。
- `workspace_read_file` / `workspace_read_file_segment` / `workspace_list_files` / `workspace_search_text` / `workspace_glob_files` 的路径解析统一改调。
- `workspace_write_file` / `workspace_edit_file` 改调 `classify_path(purpose=Write)`（写新文件复用 `prepare_workspace_file_path` 语义）。
- `workspace_run_command`：只判定 `cwd`（已在 workspace 根或受控 tmp 内）；进程内部任意写入由 PA-077 沙箱/containment 边界承接，本卡不承诺约束 Run 输出落盘。
- 权限错误统一结构化：`PermissionError { code, message }`，code ∈ `permission_denied` / `outside_workspace_write_denied` / `requires_authorization`；message 中文（供后续前端审批 UI 直接消费）。该 `{ code, message }` 信封为共享合同，PA-078（导入错误）/PA-081（权限提示）引用同一结构。

### 5. 宿主面

- `authorize_path(path, scope)` / `revoke_authorization(path)` / `list_authorizations()` 三个 Tauri command，走 control plane 审计面。
- 审批重发流（不接 Ask）：工具返回 `requires_authorization` 错误 → turn 以 permission event 呈现该错误 → 用户调用 `authorize_path` → 用户重发。`approval-semantics surface` = 结构化错误 + 授权 API，本轮不实现 Ask 集成。

## Edge Cases

- Windows 大小写：组件级比较，每组件 ASCII 小写折叠；测试断言"既放行 `c:\ws\file` ⊂ `C:\WS`，又拒绝 `c:\ws-10` ⊄ `C:\WS`"。**仅折叠 ASCII**：非 ASCII 大小写（如 `Ä`/`ä`、西里尔）在 Windows 上可能误拒——安全方向保守，记录为已接受限制（不引入 OS 大小写 API 依赖）。
- `\\?\` / UNC / 卷边界：canonical 输出归一化去除 `\\?\`（`\\?\UNC\server\share` 保留 UNC 语义但规范化）；`C:` vs `D:` 首组件不同天然不匹配；UNC 共享 CI 无可用则测试标注跳过。
- 授权父目录删除后子路径仍命中：条目保留，判定基于组件前缀；目标不存在直接拒绝 IO（安全方向保守）。
- workspace root 不可达（盘符拔出）：canonicalize 失败 → 所有判定 fail-closed。
- 写全新嵌套目录：`prepare_workspace_file_path` 向上找最近存在祖先并校验后缀，允许 `create_dir_all` 场景。
- 文件名含 `.`/`..`：后缀组件级校验拒绝（`..` 逃逸）；`file_name()` 为 None（结尾分隔符）拒绝。
- 会话级 root：`classify_path` 不持有构造时固定 root；root/tmp 由执行上下文（`ToolCall` 携带的 `workspaceId`）在调用时解析传入（机制见决策摘要 7），避免"多 workspace 下工具对错 root 判定"。
- 孤儿 `workspace_id`（注册表无该记录，root 解析失败）：**读**分类回退默认 workspace root 并打告警（会话保持可用）；**写**保持 fail-closed（拒绝）。该策略在 PA-079/PA-081 的"孤儿未分组"语义上保持一致，避免"会话显示在未分组但工具全部锁定"的隐性失联。
- 受控 tmp 共享取舍：fallback `temp_dir()/pony-agent/` 跨同用户并发实例共享（PID 后缀曾隔离实例）。同用户桌面应用风险低，**接受**并在安全说明记录；对可写 tmp 的读取放行语义不因共享而放大（写仍限 workspace+受控 tmp 两区）。
