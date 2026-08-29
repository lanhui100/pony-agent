# 设计与裁决记录

## 关键裁决（产品决策 + 双 reviewer 对抗审核后定案）

### 裁决① 会话创建目标显式化（废除隐式激活副作用）
现状组头「＋」= 先 `activateWorkspace` 再 `createSession`，副作用是悄悄改掉顶层按钮落点。定案：**每个建会话入口在调用时显式携带目标工作区**——顶层「新对话」恒定落入默认工作区（平铺区）；组内「＋」显式传该工作区 id。载体为 `createSession(targetWorkspaceId?)` 参数与瞬态条目自带的 target 字段，不再读激活态。用户可见的激活面（激活徽标、「当前工作区」提示、激活驱动建会话）全部退役；持久化的 `activeWorkspaceId` 降级为内部回退上下文，仅在注册表变更时做归一化清洗（normalizeActiveWorkspace 保留为卫生逻辑）。与需求原文「新增会话在默认工作区」一致；激活栈不需要存在。

### 裁决② 删除工作区的归属语义（P0 修复）：重写为 default
已核实两条运行时路径对失效 `workspace_id` 行为分叉：
- `control_plane/mod.rs` import_attachment：`resolve_workspace_root(...)?` **硬失败**「workspace 不存在」；
- `runtime/mod.rs` 工具链路：解析失败仅 eprintln 并**回退默认 root**（正是 PA-080 consultant 修复要防的跨项目错位）。
若删除工作区仅移除注册，名下会话续聊必然触发其一。定案：`workspace_delete` 在移除注册的同一持久化操作里把名下会话 `workspaceId` 批量重写为 `default`——附件导入恢复可用、工具根显式归位 default，与「会话失去归属→顶部平铺区」的用户心智一致。目录与会话历史数据不动；确认文案披露工具根变化。**边界**：重写只覆盖已持久化会话，故删除时还须把「存活创建目标」（含未保存瞬态条目）归一到 default，否则下一条消息会把已注销 id 盖章成新的永久孤儿（见 spec 场景 Creation target outlives deleted workspace）。改动前遗留库中的孤儿不受本方案治理：平铺区照常渲染，读路径按 path-permission 契约回退默认 root。

### 裁决③ 归档持久化：会话字段而非全局 metadata 集合
备选：store_metadata 全局集合（DSH 先例）。否决理由：SQLite 有两个组装 PersistedStore 的读取点（LegacyBlob 与 normalized 表路径），漏接任一则"操作生效、重启复活"；且集合随删除不回收、无上限增长。定案：`SessionState.archived: bool`（serde default）随会话 blob 走既有序列化——两种持久化模式天然一致、随会话删除自动回收、为后续 unarchive 预留。删除工作区的归属重写对已归档会话同样生效（无害；unarchive 后落在 default 账户位，spec 措辞为 then-current ownership）。

### 裁决④ 归档边界
- 运行中/提交中会话：前端禁用菜单项（原因 tooltip 见文案基线）+ store inflight 守卫；后端不做运行态判断（运行态主要是客户端事实，桌面单窗口场景残余竞态登记 ADR 已知限制；C3 冒烟含「提交中窗口期菜单禁用」人工一例）。
- 当前激活会话归档成功后：复用 deleteSession 的 fallback 选段自动切换到下一可用会话，避免"侧栏消失但主界面停留"。

### 裁决⑤ IA 取舍
仅保留「工作区」一级标题行（不可折叠）；「对话」区块标题移除；组折叠交互与其 localStorage 持久化整体废弃（残留 key 由既有剪枝逻辑消化，停止写入）。

### 裁决⑥ 工作区命名校验 create 与 rename 对称
rename 引入 trim 非空、≤64 字符、对**其他**工作区重名拒绝；`create_workspace_entry` 同步采纳同一规则（消除同一注册表两条写入路径的不变量分叉）。存量同名工作区可共存：改名仅在与"其他"工作区冲突时拒绝，改名回自身当前名视为 no-op 成功。前端创建表单同套校验即时提示。

## 审核驱动的硬约束（实现必须遵守）

1. **titleOverride 的消费点位全图**（架构 reviewer 两轮核实）：override 必须在以下点位统一生效，收敛为单一 `effective_title()`：
   - `refresh_session_metadata`：history 分支（build_title）与 history 空 + trace 存在分支（trace.title）都在 override 存在时跳过 title 赋值；
   - `list_sessions` 投影（侧边栏）与 `snapshot` live 分支投影（主视图）都走 effective_title；
   - **第 6 点（P0 级）**：`snapshot_at` / `snapshot_at_readonly` 的选中节点分支（store.rs L2614 `selected_node.title.clone()`）同样替换为 `effective_title(session)`——历史节点在提交时刻经 `sync_latest_history_node`（L3165/L3178）冻结了当时的派生标题，checkout 改名前旧节点若直取 node.title，主视图必然显示旧标题；
   - 历史节点列表本身（checkpoint 轻量列表等）保留提交时刻标题（那是历史浏览语义），仅顶层标题字段走 override；
   - hydrate 两处（L3235/L3256 的 node.title 回灌 `session.title`）因 override 字段独立天然无害，但以回归测试钉住。
2. **ConfirmPopover 现实现不支持异步确认**：confirm 按钮被无条件包在 PopoverClose 里（点击必关）、无受控 open/loading。必须升级为受控模式（open defineModel + loading + 错误槽），且保持向后兼容（非受控用法行为不变），存量消费方 ProviderConfigPage.vue 及其测试不击穿。
3. **三点菜单锚定行容器**而非 menu item 内部，避免 DropdownMenu item 点击卸载连带确认弹层死亡。
4. **SessionOverview.archived 前端类型必须 optional**：本地构造 overview 的位置（sessions.ts 多处 + 测试 helper）不带该字段，required 化击穿 vue-tsc 与既有测试。
5. **排序禁止发明新规则**：全局 list_sessions 序 = updatedAtMs desc、tie conversationId asc；平铺区/组内均为其过滤投影，纯函数契约写入 doc-comment 与测试；计数徽标排除归档会话。
6. **plugin-dialog 四件套缺一不可**：Cargo 依赖、`.plugin(tauri_plugin_dialog::init())` 注册、capability `dialog:allow-open`（最小化）、npm 包；F1 附 capability JSON 变更冒烟断言。
7. **同步命令主线程 IO 维持现状模式**（与 workspace_create/stamp 同款 rwlock 包装）；≥100 会话时 rename/delete 需手工卡顿测量点，超预期再立异步化任务。落盘失败的"成功回执、重启回退"语义登记 ADR 0015 已知限制。
8. **截断叠加契约**：三级行单行 CSS truncate；hover 出完整存储标题（原生 tooltip，逐字原样，可能自带后端省略号）；与后端 build_title 28 字符截断叠加共存。
9. **raw `.title` 三类消费的语义定案**（Phase 1 审核澄清）：override 存在时 refresh 跳过 title 赋值，字段冻结于改名时点——(a) 历史节点冻结处（sync_latest_history_node）改为局部 `build_title(&session.history)` 重算，保证新节点标题表达提交时刻派生值；(b) 模型上下文注入（SessionContext.title）维持读底层字段并登记 ADR 已知限制（改送 override 属模型可见变更，另行评审）；(c) normalized 表 title 列写入陈旧值在切读回填门槛内一并解决。
10. **stamp 纵深防御**：`stamp_workspace_id` 对非 default 且未注册的 id 归一为 default 并告警——后端不再依赖 Phase 3 F7 的前端时序来阻止孤儿复活。

## 文案基线（集中 sidebar-copy.ts，快照断言依据）

- 删除工作区：「将移除该工作区的注册，不影响磁盘上的目录与文件。名下的 {N} 个对话会保留并移到顶部区域；这些对话后续的文件操作将在默认工作区目录进行。」确认键「删除工作区」。{N}=0 时略去中间句，仅保留首尾句。注：删除时刻已在进行中的任务按其启动时捕获的目录完成，属已知限制（ADR 0015）。
- 归档：「归档后该对话将从侧边栏消失，当前版本无法从界面恢复（磁盘历史仍在）。」确认键「归档对话」。
- 重命名（工作区/会话通用）：确认键禁用 + 提示「名称不能为空」；上限 64 字符（超长提示「名称不能超过 64 个字符」）；重名即时提示「已存在同名工作区」。
- 菜单禁用原因 tooltip：运行中「对话运行中，暂不能执行该操作」；提交中「正在提交，请稍候」。
- 异步确认失败槽：「操作失败：{错误信息}」，弹层保留、行保留、可重试。
- 飞行中取消（Esc/外点）后到达的失败：弹层已关 → 以所属行的行内一次性提示呈现，不重开弹层（Phase 2 审核定案的 spec 契约；三条破坏性操作共用）。

## Phase 3 调用方契约（Phase 2 审核 8 条警示浓缩）

1. 受控确认标准模板：confirm 入口先查 inflight set；`loading=true → await → 成功 setOpen(false)+清 error / 失败 error=文案+停留`。
2. 成功后必须显式 `setOpen(false)`——组件无自动关闭。
3. 飞行中取消后的失败走非弹层面（见文案基线末条）。
4. per-target 弹层状态隔离：`v-if="confirmTarget"` 强制重挂载，或 inflight 期间禁其他行菜单入口。
5. 受控模式下 trigger 点击会发 toggle——程序化置 true 时父组件需接 `update:open(true)`。
6. 三点菜单 select 即关闭：ConfirmPopover 锚定行容器，trigger 不得塞进 menu item 内部。
7. 归档过滤读 `Boolean(overview.archived)`，在 deriveSidebarTree doc-comment 钉"缺省 ⇒ 存活"并配单测（本地构造不带字段的 overview 必须判活）。
8. pickExistingDirectory 仅 Tauri 可用时调用；取消 null ≠ 失败（返回值已收窄为 string|null）。
