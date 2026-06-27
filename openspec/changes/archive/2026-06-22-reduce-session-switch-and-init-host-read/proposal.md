# Proposal: Reduce session switch and initialization host reads

## Why

Pony Agent 在多会话使用场景下出现明显 UI 卡顿。诊断结果表明：

- 会话切换前台状态本身通常在几十毫秒内完成
- 真正的卡顿来自切换后和初始化时自动触发的宿主读取，如 `load_session_runtime_view` 与 `list_sessions`
- 前端还会因为整块 transcript 和 markdown 重新挂载而放大长任务
- 多次新建会话时，session list 合成逻辑会引入重复 `conversationId`，导致 sidebar duplicate key 和多项选中

本 change 目标是把会话切换与新建会话改成缓存优先和本地优先，同时为后续真正的 async runtime 并发改造打基础。

## Scope

- 会话切换不再自动依赖宿主读取完成
- 会话初始化优先使用本地持久化缓存恢复 UI 状态
- 新建会话不再同步刷新 catalog
- session list 在前端侧必须稳定去重，避免 duplicate key
- 工作区在 hydration 期间不得全量挂载 transcript
- 缓存数据必须经过 schema 校验，损坏或不匹配时回退到宿主路径
- 后台运行 turn 状态必须跨重启可恢复
- 宿主读取和 hydration 必须有超时和错误恢复
- 失效会话必须在侧边栏中清晰标识

## Non-goals

- 本 change 不直接把 `AgentRuntime` 改成 fully async actor model
- 本 change 不完成 `reqwest::blocking` -> async `reqwest` 的 provider 全量改造
- 本 change 不重写 backend session storage contract
- 本 change 不引入跨窗口/跨进程缓存一致性协议
- 本 change 不新增后端 Tauri command

## Spec review history

2026-06-25: 经 3 维度并行审核（架构/技术可行性、UX/产品完整性、可测性）后调优：
- 新增缓存校验与损坏回退
- 新增 uncached 会话的自动加载触发
- 新增快速连续切换的取消语义
- 新增后台 turn 跨重启持久化
- 新增宿主读取超时与错误恢复
- 新增失效/已删除会话处理
- 新增 localStorage 不可用降级
- 定义了"轻量占位态"、"轻量 loading 态"、"transient 会话"等关键状态的 observable 属性
- 将剩余工作转化为可测试的 acceptance criteria
