# Pony Agent Dashboard

## 项目状态
- 项目：`Pony Agent`
- 类型：学习模式重构项目
- 当前主线：在已完成的 Vue 工作台、真实 stream 主链路和原生 tools 闭环上，完成一轮 `provider / session / runtime / tool` 收口与验证闭环，接下来转向更明确的 adapter/service 边界整理
- 当前阶段：`Phase 3 / Runtime Expansion`
- 总体状态：`In Progress`

## 当前重点
1. 基于已经稳定下来的 core 边界，开始评估 `PA-007` 的 adapter 抽离前置条件
2. 继续把 provider 能力声明真正接入 runtime 输入编排与限制逻辑
3. 继续把工具层从“组合工具可用”推进到“多工具语义和更细粒度 telemetry 可解释”

## 当前任务摘要
- `PA-003`：实现 Rust 单轮 runtime 骨架
- `PA-004`：定义 provider 与 tool 抽象
- `PA-005`：把 Vue 工作台接入真实 turn 执行链路
- `PA-006`：实现新对话与历史对话管理
- `PA-009`：完善 provider 能力配置（思考、多模态、上下文与模型能力）
- `PA-007`：拆分独立接入层（Tauri / HTTP-SSE adapter）
- `PA-008`：补强工具层（多工具、并发、权限、错误恢复）

## 当前断点
- 前端已切换到 `Vue + Pinia`
- 工作台已有运行时状态、轨迹和工具预演面板
- Rust runtime 已能在 `run_turn()` 中读取 provider 配置、发起真实请求，并在失败时回退到 mock
- 实际流式入口已经从同步整包 `run_turn()` 旁边长出 `start_turn_stream()` 这条事件驱动链路
- OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架
- `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 已形成最小事件模型
- 当前 `ToolRouter / ToolCall / ToolResult` 最小闭环已经成立，并已切到 OpenAI `tool_calls` 与 Anthropic `tool_use/tool_result` 原生协议主路径
- runtime 已补上本地 planner，能对明显的工作区请求前置命中 `workspace.*` 工具，减少无意义的远端等待
- 这轮已修复 OpenAI 兼容 provider 的流式 SSE 解析问题，`data: ...` chunk 不再被误当成普通 JSON
- 对话主区、Markdown 渲染、模型脚注、输入交互和 provider/model 选择器已进入可用状态
- 前端已开始把最近几轮 history 发送给后端，最小多轮语境已能支持“文件说明 -> 继续问该文件第 N 行”这类真实工作流
- 已明确把 prompt caching 纳入重构考量：后续 history、session summary、工具清单注入方式都要兼顾 provider 侧缓存命中
- 当前真实会话状态已开始由 Rust `SessionStore` 持有，并落到 `.pony-agent/sessions.json` 做最小持久化
- 当前 agent core 仍主要通过 Tauri command/event 暴露，尚未抽成独立 HTTP/SSE 服务层
- 当前真实工具执行链已经具备单工具 roundtrip，并已有 `workspace.list_files / workspace.read_file / workspace.read_file_segment`
- 当前已经补出最小会话 UI：左侧“对话历史”可折叠，可新建、可切换、可清除
- 当前 provider 仍处在“最小可用”阶段，尚未系统纳入模型思考、推理强度、多模态输入、上下文窗口与模型能力矩阵
- 2026-05-23 这一轮已完成重构收口：
- `runtime.ts` 已统一 trace、browser-preview、失败态文案与状态流，并抽出重复 helper
- 前端已建立固定验证闭环：`npm run verify = test:unit + build + cargo check`
- 已补齐组件层回归测试：`HomeWorkspace` / `HomeSessionSidebar`
- Rust `cargo check` warning 已清干净，probe-only dead_code 噪音已隔离
- 已完成真实 `tauri dev --no-watch` 冒烟，确认桌面壳、Vite dev server 和 Rust 进程可正常拉起

## 下一步最小动作
1. 为 `PA-007` 做一次“现在能否从 Tauri command/event 抽出独立 adapter”的边界审计
2. 把 `supportsImageInput / contextWindowTokens / reasoning` 继续接入 runtime 的真实输入限制与提示逻辑
3. 继续完善工具 telemetry，让 batch/gather 这类组合工具在 trace 中有更细粒度展开
4. 在真实 provider 联调下补一轮非 mock 的长链路人工回归

## 关联入口
- 项目记忆：`AGENT.md`
- 文档索引：`docs/INDEX.md`
- 任务板：`01_TASK_BOARD.md`
