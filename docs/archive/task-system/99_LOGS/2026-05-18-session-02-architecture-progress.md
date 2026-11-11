# Session Log 2026-05-18 / 02

## 本次补记了什么

- 将“当前 stream 已可用”这件事同步回任务系统和学习文档
- 明确当前流式主链并不是同步 `run_turn()` 自身完成的，而是已经旁路长出了 `start_turn_stream()` 事件链
- 明确前端与后端当前走的是 Tauri command/event，而不是 HTTP API
- 明确 Pony Agent 未来不是在 Hermes 与 Claude Code 之间二选一，而是按层借鉴

## 当前学习结论

- Pony Agent 当前已经具备最小真实流式 runtime
- `run_turn()` 仍是核心入口，但未来要继续向更完整的 query loop 演进
- 独立 agent core 的下一层重点不再只是 UI，而是 `provider / tool / session / delivery adapter` 的边界

## 对任务系统的影响

- `PA-005` 的关注点已从“前端是否接通 turn”进一步转向“事件契约是否稳定、是否能脱离 Tauri 复用”
- `PA-004` 不应只理解为 provider/tool trait，而应包含未来的核心边界收束意识
- 后续自然延伸的任务方向会包括 `SessionStore`、HTTP/SSE adapter 和更完整的 query loop

## 下一步动作

- 继续收束 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
- 梳理 Rust core 与接入层之间的分层草图
- 在不破坏现有联调链路的前提下，继续为后续持久化和工具执行预留接口
