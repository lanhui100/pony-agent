# Proposal: Add Checkpoint Message Controls And Bottom Menu

## Why

当前 Pony Agent 的 checkpoint / history-control 能力已经存在，但主交互面仍然偏“调试视角”：

- 主入口集中在 `HomeSessionSidebar.vue` 的 history graph / session control 卡片，信息密、操作重、离消息上下文远
- `HomeWorkspace.vue` 的消息流里看不到“这条 agent 回复对应哪个 checkpoint、能不能回退、有没有 fork”
- 用户想回到某条旧回复对应的状态时，需要先理解侧边栏中的 branch / node 结构，再决定 checkout / restore / fork
- composer 底部区域缺少一个低摩擦的 checkpoint picker，导致“我要从哪个 checkpoint 继续”这个高频动作没有就近入口

这会让当前 session control surface 显得笨重，也让 checkpoint 能力没有真正融入主对话区。

## What Changes

- 为主对话区里的每条非最新 agent 消息增加消息级 checkpoint 控制
- 为每个可回退 checkpoint 提供两个纯图标动作：
  - 仅回退对话历史
  - 回退对话历史并尝试恢复文件改动
- 当某个 checkpoint 已经产生 fork 对话轨迹时，增加 fork 摘要图标，点击后展示各 fork 的摘要与跳转入口
- 清理当前笨重的 session control 主设计，把 checkpoint 选择主入口收口到 composer 底部 bar 的一个功能键
- 为 checkpoint picker 增加键盘快捷键
- 为消息级 affordance、底部 picker、fork 摘要弹层建立稳定的前端 read model 与验收测试矩阵

## Out Of Scope

- 新增新的 history-control backend command
- 改写既有 `checkout / restore / fork / switch` 的 truth-source 语义
- 实现新的 workspace rollback 能力
- 重做 provider/model 菜单、消息 markdown 样式或整个 sidebar 信息架构
- 引入跨 session 的可视化分支图或 diff viewer
