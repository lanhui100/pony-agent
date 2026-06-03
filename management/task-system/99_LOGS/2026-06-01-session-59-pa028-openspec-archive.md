# 2026-06-01 Session 59 - PA-028 OpenSpec 归档收口

## 本次目标
- 完成 `add-history-node-management` 的 OpenSpec 归档收口，避免已完成任务继续滞留在 `openspec/changes/`。
- 同步任务系统中的 spec 引用，防止任务卡链接失效或状态漂移。

## 归档前核查
- `openspec/changes/archive/2026-06-01-add-history-node-management/specs/history-node-management/spec.md` 与 [openspec/specs/history-node-management/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/history-node-management/spec.md) 哈希一致，说明 canonical spec 已同步完成。
- [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-01-add-history-node-management/tasks.md) 中所有任务均已勾选完成。
- 归档前，仓库内直接引用 `openspec/changes/add-history-node-management` 的任务系统文件仅有 [PA-028-build-history-node-management-and-branching.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-028-build-history-node-management-and-branching.md)。
- 本地环境未安装 `openspec` CLI，因此 artifact 状态核查改为基于 proposal/design/tasks/spec 文件和已同步 canonical spec 的人工校验。

## 执行动作
- 将 `openspec/changes/add-history-node-management` 移动到 `openspec/changes/archive/2026-06-01-add-history-node-management`。
- 更新 `PA-028` 任务卡中的 spec change 链接到归档路径。

## 结果
- `PA-028` 的任务系统、canonical spec、OpenSpec change 目录状态已对齐。
- 历史节点管理不再停留在活动 change 列表中，后续查阅应以 canonical spec 和 archive change 为准。
