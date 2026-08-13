# Design

## Decision Summary

1. 新增 `agent/workspace.rs`：`WorkspaceRecord { id, name, root_path }` + `WorkspaceRegistry`（内存 `RwLock<Vec<WorkspaceRecord>>` + SQLite `store_metadata` 持久化，key=`workspaces.v1`；JSON backend 存独立 `workspaces.json`）。
2. **默认 workspace id = 保留字 `"default"`**（豁免 `ws-<slug>` 规则），首次初始化创建（root = 当前 `workspace_root`），始终存在。
3. `SessionState` 增加 `#[serde(default)] pub workspace_id: Option<String>`（兼容旧数据）；`SessionOverview` 投影透传；**线上传输 = `TurnInput.workspace_id`**（前后端同步增加），首次持久化时盖章（None → 默认投影）。
4. **移除 `workspace_activate`**（激活为前端 localStorage 单一真相源，PA-081 拥有；本卡不提供后端激活持久化）。
5. 持久化走 `SessionBackend::read_metadata/write_metadata` 扩展（复用 `store_metadata` 的 `INSERT OR REPLACE`），每次变更即写，与 full-store save 不互相覆盖；新锁登记进 `docs/concurrency/lock-ordering.md`。
6. 宿主 Tauri command 最小面：`workspace_list` / `workspace_create`。

## Chosen Direction

### 1. 数据模型

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,            // 默认保留字 "default"，其余 ws-<slug>
    pub name: String,
    pub root_path: PathBuf,    // canonical 绝对路径（Windows 去 \\?\ 前缀）
}
```

`WorkspaceRegistry` 提供 `list() / create(name, root) / get(id) / default()`，全部返回 `Result`（结构化错误）。

- 默认 id 常量：`pub const DEFAULT_WORKSPACE_ID: &str = "default";`，前端导出同名常量（PA-081 消费），None→default 投影与此一致。
- 命名去歧义：本卡的 `workspace_id` 是"项目目录"概念，与既有 `WorkspaceRef`/`WorkspaceRefKind`（`session.rs:88-111`，git/host-snapshot 回滚引用）含义不同，文档中显式区分。

### 2. 持久化

- **实现简化（实现后审核 P2-3 记录）**：注册表随 `PersistedStore.workspaces` 全量保存（SQLite `store_metadata` key=`workspaces`；File/Memory backend 随 full-store 序列化自动覆盖），`create_workspace`/`stamp_workspace_id` 走既有 `save_to_backend()`。不再实现独立的 `read_metadata`/`write_metadata` 通用扩展——注册表是 store 的一部分故无双写/互覆盖风险，且 create/stamp 低频，全量保存可接受。JSON backend 不存独立 `workspaces.json`（与 spec 文字存在偏差，行为等价）。
- SQLite：`store_metadata` 表 key=`workspaces.v1`（实现用 key=`workspaces`），value=JSON 序列化的 `Vec<WorkspaceRecord>`。
- 读取失败时 fail-safe：损坏 JSON 经 `read_metadata` 的 `unwrap_or_default()` 回退空注册表，`SessionStore` 再重建默认 workspace，不阻塞启动。
- 每次变更（create）即持久化；损坏/读取失败的回退可覆盖到下一次成功写入。

### 3. 会话归属

- `SessionState.workspace_id: Option<String>`（`#[serde(default)]`）。
- `SessionSnapshot` / `SessionOverview` 投影：`workspace_id` 为 None → `"default"`。
- **传输通道**：前端 `TurnInput.workspaceId?: string | null`（`src/types/runtime.ts:775-786` 扩展）与后端 `TurnInput.workspace_id`（`runtime/mod.rs:150-163` 扩展）同步增加；`submitTurn` 传 `activeWorkspaceId ?? "default"`；`run_turn`/`start_graph_run_stream`/`start_turn_stream` 首次持久化 `SessionState` 时盖章。
- 后端 `workspace_root` 解析钩子：session 携带 `workspace_id` 时，工具沙箱用对应 workspace root；本轮只做解析钩子（PA-080 深化：`classify_path` 在调用时按会话 `workspace_id` 经注册表解析 root，per-turn context 线程传递）。

### 4. 前端

- `SessionOverview` / `SessionSnapshot` 增加 `workspaceId?: string | null`。
- `TurnInput` 增加 `workspaceId?: string | null`；`runtime.ts` `sessionList` 透传；`createSession()` 传默认/激活 workspace（`activeWorkspaceId ?? "default"`）。
- **3 个本地构造器同步补字段**（默认 `"default"`）：`createTransientSessionOverview` / `buildSessionOverviewFromPersistedState` / `buildSessionOverviewFromRuntimeState`（`src/lib/runtime/sessions.ts`）。
- 不做 UI 树（PA-081）。

## Edge Cases

- 旧会话加载：workspace_id None → 默认 workspace，不重写存储（只投影）；serde roundtrip 断言所有既有字段原样保留。
- 注册表损坏：读取失败回退默认，启动不阻塞。
- workspace root 删除/不可达：注册表保留记录，路径解析失败时返回明确错误（不 panic）。
- 并发创建同名 workspace：id 唯一（slug + 时间戳后缀），name 允许重复但提示；重复 root 以 **canonical 比较**（Windows 大小写归一）判定拒绝。
- root 不是目录（是普通文件）：canonicalize 成功但 `is_dir()` 为假 → 拒绝创建。
- 孤儿 workspace_id（注册表丢失该记录）：投影保留 `workspace_id`，前端侧边栏归入"未分组"（PA-081 定义），本卡不删数据。
- 新锁登记：`WorkspaceRegistry` 的 `RwLock` 加入 `docs/concurrency/lock-ordering.md` 规范顺序（置于 `sessions_rwlock` 之上），poison 策略与 PA-069-C 一致。
