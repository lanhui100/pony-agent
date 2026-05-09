# Pony Agent Tauri + Rust 重构说明

## 为什么不直接改 Python 主体

当前仓库的核心入口集中在以下位置：

- `run_agent.py`：主智能体循环、模型调用、工具执行、上下文控制
- `cli.py`：终端交互层
- `hermes_cli/main.py`：命令入口、配置和平台编排

这些模块已经承载大量历史能力，直接原地替换为 Rust 会让迁移路径和回归风险都变得很高。

因此这次采用双轨方案：

- 原 Hermes 仓库继续保留，作为功能和行为参考
- 新增 `apps/desktop-tauri` 作为下一代桌面端与 Rust 运行时实验场

## 参考映射

| Hermes 现有能力 | 新 Rust 项目对应方向 |
| --- | --- |
| `run_agent.py` 对话主循环 | `src-tauri/src/agent/runtime.rs` |
| 上下文压缩、提示词拼装、记忆注入 | `agent/session.rs` + 后续 `memory` 模块 |
| 工具注册与调用 | `agent/tools.rs` |
| 子代理与多轮流程控制 | `agent/graph.rs` |
| CLI / TUI | Tauri 前端界面 + 命令桥 |
| MCP / provider 接入 | 后续 `provider`、`bridge` 模块 |

## 建议的 Rust 分层

### 1. App Shell

- Tauri Window
- 前端状态同步
- invoke 命令边界

### 2. Agent Runtime

- 会话状态
- 对话轮次调度
- 中断 / 恢复
- 观察与日志

### 3. Graph Orchestrator

参考 LangGraph 的状态图思路，但不必照搬 Python API：

- `Plan`
- `Reason`
- `CallTool`
- `Observe`
- `Summarize`
- `Done`

每个节点返回显式状态转移，便于：

- 可视化调试
- 可恢复执行
- 子任务拆分
- 后续接入持久化检查点

### 4. Provider Layer

统一封装：

- OpenAI Responses / Chat Completions
- Anthropic Messages
- OpenRouter / 兼容 OpenAI 协议接口

这里建议定义内部抽象：

- `ModelProvider`
- `ToolCall`
- `AssistantTurn`
- `UsageReport`

### 5. Tool Router

统一工具协议，而不是把工具直接散落在业务里：

- terminal
- fs
- search
- http
- mcp
- workflow

### 6. Memory Layer

参考 Claude / LangChain 体系，但更偏工程化：

- 短期上下文窗口
- 会话摘要
- 用户画像
- 任务记忆
- 可选向量索引

## 当前已落地内容

- 新建了 `apps/desktop-tauri` 最小项目骨架
- 搭好了 Vite 前端与 Tauri v2 配置
- 建立了 Rust 侧 `agent/runtime/graph/session/tools` 模块边界
- 前端已通过 `invoke("health_check")` 与 Rust 命令模型对齐

## 下一阶段建议

1. 实现 `AgentRuntime::run_turn()`，支持单轮输入到输出
2. 设计 `Provider` trait，先接一个 OpenAI 兼容模型
3. 设计 `ToolRouter` trait 和统一 `ToolResult`
4. 增加本地会话存储与摘要机制
5. 再迁移多轮图编排、记忆、子代理
