# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式智能体重构项目
- 当前主线：在已完成的 Vue 工作台、真实 stream 主链路和原生 tools 闭环上，继续收束运行时可见性，并把 agent core 逐步推向可独立部署的 Rust 引擎
- 当前阶段：`Phase 3 / Runtime Expansion`
- 总体状态：`In Progress`

## 当前重点

1. 继续把 `run_turn()` 从“单轮最小流式闭环”推进到“更完整的 query loop 骨架”
2. 收束 `provider / tool / stream / trace / fallback` 的运行时可见性，减少学习成本
3. 为未来的 session 持久化、HTTP/SSE 接入层和多工具执行链预留稳定边界

## 当前任务摘要

- `PA-003`：实现 Rust 单轮 runtime 骨架
- `PA-004`：定义 provider 与 tool 抽象
- `PA-005`：把 Vue 工作台接入真实 turn 执行链路

## 当前断点

- 前端已切换到 `Vue + Pinia`
- 工作台已有运行时状态、轨迹和工具预演面板
- Rust runtime 已能在 `run_turn()` 中读取 provider 配置、发起真实请求，并在失败时回退到 mock
- 实际流式入口已经从同步整包 `run_turn()` 旁边长出 `start_turn_stream()` 这条事件驱动链路
- 前端工作台已通过 Tauri command 启动 turn，并通过事件流实时更新 assistant 文本、phase、trace、tool activities、provider 信息
- OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架
- `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 已形成最小事件模型
- 当前 `ToolRouter / ToolCall / ToolResult` 最小闭环已经成立，并已切到 OpenAI `tool_calls` 与 Anthropic `tool_use/tool_result` 原生协议主路径
- runtime 已补上本地 planner，对明显的工作区请求可直接前置命中 `workspace.*` 工具，不再先白等远端 decision
- `npm run dev` 浏览器预览已具备非 Tauri 环境兜底，不会再因直接调用 Tauri API 白屏
- 对话主区、Markdown 渲染、模型脚注、输入交互和 provider/model 选择器已进入可用状态
- 前端已开始把最近几轮 history 发送给后端，最小多轮语境已经能支持“文件说明 -> 继续问该文件第 N 行”这类真实工作流
- 当前会话历史仍主要停留在前端内存态，尚未落到持久化存储
- 当前 agent core 仍主要通过 Tauri command/event 暴露，尚未抽成独立 HTTP/SSE 服务层
- 当前真实工具执行链已具备单工具 roundtrip，并已有 `workspace.list_files / workspace.read_file / workspace.read_file_segment` 这组最小工作区工具；但仍未进入多工具、并发工具和更严格错误恢复阶段
- 当前仍未展示 `token 统计 / 首 token 延迟` 等更细运行指标

## 下一步最小动作

先完成运行时可见性与架构边界的收束，再继续往独立 core 演进：

- 在主页或状态区补上 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
- 验证两类真实 provider 在本地联调中的 stream 观感是否稳定
- 在不破坏现有事件模型的前提下，逐步抽离 provider trait、tool router 和 session store 边界
- 继续补工具侧边界：更完整错误态、更多工作区工具、以及多工具策略
- 把当前“history 推断文件”逐步升级为更明确的 session/runtime 状态，减少临时规则继续膨胀
- 规划独立接入层：至少明确未来的 Tauri adapter 与 HTTP/SSE adapter 如何共用同一个 Rust core

## 关联入口

- 项目记忆：`AGENT.md`
- 文档索引：`docs/INDEX.md`
- 任务板：`01_TASK_BOARD.md`
