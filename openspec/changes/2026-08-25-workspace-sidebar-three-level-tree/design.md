# 设计与裁决记录

## 关键裁决（产品决策 + 双 reviewer 对抗审核后定案）

### 裁决① 会话创建目标显式化（废除隐式激活副作用）
现状组头「＋」= 先 `activateWorkspace` 再 `createSession`，副作用是悄悄改掉顶层按钮落点。定案：组内「＋」显式传该工作区 id、**不动激活态**；顶层「新对话」恒定落入默认工作区（平铺区）。与需求原文「新增会话在默认工作区」逐字一致；激活栈不需要存在。
AC：点击组内＋后再点顶层新对话，落点仍是平铺区。

### 裁决② 删除工作区的归属语义（P0 修复）：重写为 default
已核实两条运行时路径对失效 `workspace_id` 行为分叉：
- `control_plane/mod.rs` import_attachment：`resolve_workspace_root(...)?` **硬失败**「workspace 不存在」；
- `runtime/mod.rs` 工具链路：解析失败仅 eprintln 并**回退默认 root**（正是 PA-080 consultant 修复要防的跨项目错位）。
若删除工作区仅移除注册，名下会话续聊必然触发其一。定案：`workspace_delete` 在移除注册的同一持久化事务里把名下会话 `workspaceId` 批量重写为 `default`——附件导入恢复可用、工具根显式归位 default，与「会话失去归属→顶部平铺区」的用户心智一致。目录与会话历史数据不动。确认文案必须披露工具根变化。

### 裁决③ 归档持久化：会话字段而非全局 metadata 集合
备选：store_metadata 全局集合（DSH 先例）。否决理由：SQLite 有两个组装 PersistedStore 的读取点（LegacyBlob 与 normalized 表路径），漏接任一则"操作生效、重启复活"；且集合随删除不回收、无上限增长。定案：`SessionState.archived: bool`（serde default）随会话 blob 走既有序列化——两种持久化模式天然一致、随会话删除自动回收、为后续 unarchive 预留。

### 裁决④ 归档边界
- 运行中/提交中会话：前端禁用菜单项（带原因 tooltip）+ store inflight 守卫；后端不做运行态判断（运行态主要是客户端事实，桌面单用户场景残余竞态登记 ADR 已知限制）。
- 当前激活会话归档成功后：复用 deleteSession 的 fallback 选段自动切换到下一可用会话，避免"侧栏消失但主界面停留"。

### 裁决⑤ IA 取舍
仅保留「工作区」一级标题行（不可折叠）；「对话」区块标题移除；组折叠交互与其 localStorage 持久化整体废弃（残留 key 由既有剪枝逻辑消化，停止写入）。

## 审核驱动的硬约束（实现必须遵守）

1. **titleOverride 的消费点位全图**（架构 reviewer 核实）：override 必须在以下点位统一生效，收敛为单一 `effective_title()`：
   - `refresh_session_metadata`：history 分支（build_title）与 history 空 + trace 存在分支（trace.title）都在 override 存在时跳过 title 赋值；
   - `list_sessions` 投影（侧边栏）与 `snapshot` 投影（主视图）都走 effective_title；
   - hydrate 两处（checkout/切分支用 node.title 回灌 session.title）因字段级独立天然无害，但需回归测试钉住。
2. **ConfirmPopover 现实现不支持异步确认**：confirm 按钮被无条件包在 PopoverClose 里（点击必关）、无受控 open/loading。直接复用会产生重复提交与弹层随菜单卸载两个缺陷。必须先升级组件再接业务。
3. **三点菜单锚定行容器**而非 menu item 内部，避免 DropdownMenu item 点击卸载连带确认弹层死亡。
4. **SessionOverview.archived 前端类型必须 optional**：本地构造 overview 的位置（sessions.ts 多处 + 测试 helper）不带该字段，required 化击穿 vue-tsc 与既有测试。
5. **排序禁止发明新规则**：全局 list_sessions 序 = updatedAtMs desc、tie conversationId asc；平铺区/组内均为其过滤投影，纯函数契约写入 doc-comment 与测试。
6. **plugin-dialog 四件套缺一不可**：Cargo 依赖、`.plugin(tauri_plugin_dialog::init())` 注册、capability `dialog:allow-open`（最小化）、npm 包。
7. **同步命令主线程 IO 维持现状模式**（与 workspace_create/stamp 同款 rwlock 包装）；≥100 会话时 rename/delete 需手工卡顿测量点，超预期再立异步化任务。

## 文案基线（集中 sidebar-copy.ts，快照断言依据）

- 删除工作区：「将移除该工作区的注册，不影响磁盘上的目录与文件。名下的 {N} 个对话会保留并移到顶部区域；这些对话后续的文件操作将在默认工作区目录进行。」确认键「删除工作区」。
- 归档：「归档后该对话将从侧边栏消失，当前版本无法从界面恢复（磁盘历史仍在）。」确认键「归档对话」。
- 重命名空值：确认键禁用 + 提示「名称不能为空」；上限 64 字符；重名即时校验提示。
