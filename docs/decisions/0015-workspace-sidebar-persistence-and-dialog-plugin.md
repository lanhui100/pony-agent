# ADR 0015：侧边栏三级树的持久化字段、删除归属重写与 dialog 插件依赖

Status: implemented

- 关联变更：openspec/changes/2026-08-25-workspace-sidebar-three-level-tree
- 接替关系：不接替既有 ADR；补充 0013 未涉及的工作区侧边栏信息架构与持久化面

## 背景

侧边栏工作区三级树改造需要四项后端新能力（工作区重命名/删除、会话重命名/归档）。核查发现三个必须先定案的契约问题：

1. 会话标题由首条用户消息派生（`build_title`），且 `refresh_session_metadata` 每轮持久化都会覆盖 `session.title`（另有 history 空 + trace 存在时的 `trace.title` 分支、checkout/切分支的 node.title 回灌）——直接改 title 字段会被静默冲掉。
2. 归档若采用全局 metadata 集合，SQLite 有两个组装 PersistedStore 的读取点（LegacyBlob 路径与 normalized 表路径），漏接任一则"操作生效、重启复活"，且集合随会话删除不回收。
3. 删除工作区若只移除注册表记录，名下会话已盖章的 `workspace_id` 将在两条运行时路径上行为分叉：附件导入对解析失败硬报错（`import_attachment` 的 `?` 传播），工具执行却静默回退默认 root——后者正是 PA-080 consultant 修复要防的跨项目错位。

## 决策

1. **持久化字段随会话 blob**：`SessionState` 新增 `titleOverride: Option<String>` 与 `archived: bool`，均 serde default + skip 序列化缺省，双向兼容旧数据（沿 `workspace_id` 先例）。标题投影收敛为单一 `effective_title()`，在 `list_sessions`、`snapshot`、`refresh_session_metadata` 全部分支统一生效；hydrate 回灌因字段级独立天然无害。
2. **归档不做全局集合**：`archived` 随会话 blob 走既有序列化，两种持久化模式天然一致、随会话删除自动回收、为后续 unarchive 预留（清标志即可恢复原工作区账户位）。
3. **删除工作区 = 移除注册 + 名下会话归属批量重写为 default**：同一持久化操作内完成。附件导入与工具根两条路径随之收敛到 default root；确认文案显式披露工具根变化。磁盘目录与会话历史不动；`default` 拒删。
4. **引入 tauri-plugin-dialog**（Rust 依赖 + `.plugin(tauri_plugin_dialog::init())` 注册 + capability 仅 `dialog:allow-open` + npm 包），用于「添加工作区」选择已存在目录；root 校验沿用 canonicalize + is_dir，不支持经此流程新建目录。
5. **同步命令维持现状模式**：四条新命令与 `workspace_create` 同款 rwlock 同步包装、全量 store 序列化落盘。

## 候选方案

- **归档持久化**：全局 store_metadata 集合（DSH 式 registry-global set）——落选：SQLite 存在 LegacyBlob 与 normalized 表两个组装读取点，漏接任一则"操作生效、重启复活"；集合随会话删除不回收且无上限增长。会话字段方案随 blob 双模式天然一致。
- **标题覆盖机制**：直接改写 `session.title` 并在投影点跳过重算——落选：title 有五类消费/覆盖点位（refresh 两分支、hydrate 两处、双投影），字段内联方案的回归面大且 checkout 回灌仍会冲掉；独立 override 字段使 hydrate 无害化成为结构性质。
- **删除归属语义**：仅移除注册、保留脏 id——落选：附件导入硬失败与工具根静默漂移两条路径行为分叉，续聊必踩其一；禁删非空工作区——落选：把注册表管理耦合到会话生命周期，用户心智里"移出列表"不该被数据绑票。重写 default 与平铺区展示语义一致。
- **目录选择**：沿用手填 root 路径表单——落选：需求要求"默认以文件夹命名"，目录选择器 + basename 预填直接满足；手填易错且无存在性预检体验。

## 已知限制

- 归档运行中会话的竞态由前端禁用 + store inflight 守卫兜底，后端不做运行态判断（运行态主要是客户端事实）。
- 本期归档无恢复入口；字段设计使恢复仅需新增 unarchive 命令 + 清标志。
- `import_attachment` 对失效 `workspace_id` 硬失败 vs 工具链路软回退的根因分叉仍在；本功能路径通过归属重写消除了触发面，但语义分叉留待后续统一。
- 多窗口场景未处理（现仅 main 窗口）。

## 影响

- 持久化格式：两处 additive serde default 字段，旧二进制读新 blob 忽略未知字段，可直接回滚，无迁移脚本。
- 依赖：+tauri-plugin-dialog（Rust/npm/capability 三处登记）。
- 前端：废弃组折叠 localStorage key 的写入路径。
