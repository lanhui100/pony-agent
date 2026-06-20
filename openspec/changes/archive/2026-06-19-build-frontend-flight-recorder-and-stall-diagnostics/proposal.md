# Proposal: Build Frontend Flight Recorder And Stall Diagnostics

## Why

Pony Agent 当前已经能在前端记录零散 runtime 日志、stream debug latest bucket 和少量阶段耗时，但当页面发生“turn 完成后卡住数秒”的问题时，仍然缺少一套系统化证据链来回答：

- 卡顿是否发生在业务逻辑、响应式传播、DOM 提交、Markdown 渲染、滚动/动画还是 GC
- 卡顿发生前后，当前 session / turn / phase / message 规模 / trace 规模 / DOM 规模是多少
- 同类卡顿是否具有稳定触发模式，例如集中发生在 `turn:completed`、`syncStreamingPresentationState`、`turns` 重算或 `MarkdownRenderer`
- 事后是否可以按会话回放，而不是依赖浏览器 console 或临时补丁式日志

当前已有观测能力不足的根因在于：

- `debugLog` 是 console-oriented，而不是会话级可查询 trace
- `__ponyStreamMetrics` 与 `record_stream_debug_metrics` 主要保留 latest state，而不是时间序列
- 现有日志文件与真实运行环境并不总是一一对应，导致事后排查容易落到旧日志或无关环境

这使得“修一个怀疑点再试一次”的补丁式诊断成本很高，也容易误把现象归因到错误层面。

## What Changes

- 建立一套前端 flight recorder 正式方案，统一前端耗时、状态、卡顿与现场快照记录
- 定义 recorder 生命周期、`sessionId / turnId` 绑定规则、`seq / tsPerfMs` 语义与可配置阈值
- 定义主线程 stall 检测机制，不依赖浏览器开发者工具或人工观测
- 定义 stall 触发后的冻结现场快照内容、体积约束与触发策略
- 将诊断数据正式持久化到 Tauri/SQLite，而不是依赖 console 或 localStorage
- 定义最小查询/导出能力，使单次卡顿可以按 `sessionId / turnId / 时间窗口` 回放
- 定义索引、retention、分页、失败降级与非阻塞 flush 边界，确保诊断链路不会反向放大卡顿
- 定义首批必须埋点的关键链路与开销控制规则
- 将落地拆为“基础诊断管线”与“关键链路埋点/导出体验”两阶段，避免首期范围失控

## Impact

- 前端卡顿排查将从“猜测 + 临时补丁”升级为“可回放的结构化诊断”
- Tauri 宿主将成为前端诊断事件的权威持久化载体
- 后续前端性能优化可以基于同一套事件模型持续演进，而不是每轮重新设计日志口径
- 当前 UI 偏好和轻量缓存仍可保留 localStorage，但诊断体系不再依赖它作为主数据源

## Tracking

- Task card: `PA-057`
- OpenSpec Change: `build-frontend-flight-recorder-and-stall-diagnostics`
