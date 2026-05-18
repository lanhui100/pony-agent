# Pony Agent Dashboard

## 项目状态

- 项目：`Pony Agent`
- 类型：学习模式智能体重构项目
- 当前主线：在已完成的 Vue 调试台上，继续增强已接入真实 provider 的 `run_turn()` 主链路、流式回包体验、运行时可见性和后续 provider/tool 抽象
- 当前阶段：`Phase 3 / Runtime Expansion`
- 总体状态：`In Progress`

## 当前重点

1. 继续增强 `run_turn()` 的流式状态可见性与失败态收束
2. 补齐 `providerMode / fallbackReason / token 统计 / 首 token 延迟` 的运行时观察面
3. 在现有最小主链路上继续抽离 provider 与 tool 抽象

## 当前任务摘要

- `PA-003`：实现 Rust 单轮 runtime 骨架
- `PA-004`：定义 provider 与 tool 抽象
- `PA-005`：把 Vue 工作台接入真实 turn 执行链路

## 当前断点

- 前端已切换到 `Vue + Pinia`
- 工作台已有运行时状态、轨迹和工具预演面板
- Rust runtime 已能在 `run_turn()` 中读取 provider 配置、发起真实请求，并在失败时回退到 mock
- 前端工作台已通过 Tauri command 启动 turn，并通过事件流实时更新 assistant 文本、phase、trace、tool activities、provider 信息
- OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架
- `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 已形成最小事件模型
- 当前仍未接入真实工具执行链
- 当前仍未展示 `token 统计 / 首 token 延迟` 等更细运行指标

## 下一步最小动作

先完成可见性补强，再继续抽象收敛：

- 在主页或状态区补上 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
- 验证两类真实 provider 在本地联调中的 stream 观感是否稳定
- 同步更新 `PA-004` / `PA-005` 任务卡，使正式任务系统与代码现状一致
- 在不破坏现有回包结构的前提下，逐步抽离 provider trait 和 tool router 边界

## 关联入口

- 项目记忆：`AGENT.md`
- 文档索引：`docs/INDEX.md`
- 任务板：`01_TASK_BOARD.md`
