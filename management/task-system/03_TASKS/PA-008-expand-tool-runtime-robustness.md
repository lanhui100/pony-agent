# PA-008 补强工具层（多工具、并发、权限、错误恢复）

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
在已经成立的原生 tools 主路径之上，继续把工具层从“最小单工具闭环”推进到“更接近真实 agent 工作流”的状态。

## 输出
- 更完整的工具层路线图
- 多工具调用边界设计
- 工具错误恢复与权限约束方案
- 新一轮工作区工具扩展清单

## 验收标准
- 不再只覆盖单工具 roundtrip
- 工具失败时能更清楚地进入 trace、tool activity 和 follow-up 语义
- 对未来多工具、并发工具和权限审批有明确的内部抽象方向

## 当前进展
- `ToolRouter / ToolCall / ToolResult` 最小闭环已经成立
- OpenAI 已走 `tool_calls -> tool`，Anthropic 已走 `tool_use -> tool_result`
- 已有 `workspace_list_files / workspace_read_file / workspace_read_file_segment / workspace_path_info / workspace_search_text`
- runtime 已有本地 planner，可前置命中高确定性工作区工具
- 这一轮新增：
- `workspace_batch`：受限批量工具执行，支持 `parallel` 和 `continueOnError`
- `workspace_gather_context`：围绕文件、目录、搜索场景自动聚合多个上下文子调用
- `workspace_read_file` 大文件保护：超限时引导改用 `segment / gather`
- `workspace_search_text` 目录预算保护：默认跳过 `.git / node_modules / target / dist / build`
- `LocalTurnPlanner` 已升级为区分目录列举、显式路径概览、基于历史路径的继续追问和带引号的本地搜索语句
- 2026-05-23 这一轮继续完成工具/runtime 收口：
- 前端 runtime 已统一失败态与 browser-preview fallback 的 trace 构造，减少工具相关状态在 UI 侧的重复拼装
- store 层已验证 `turn:started -> delta -> completed`、`turn:started -> failed`、即时 `start_turn_stream` 失败和 browser-preview 回退链路
- Rust probe 二进制引入的 warning 噪音已隔离，便于后续继续观察真正的工具层 warning 与 regression
- 本轮继续把 telemetry 推进到“组合工具可解释”：
- `workspace_batch / workspace_gather_context` 在运行态会展开 planned 子调用，在完成态会展开 nested tool activity，便于区分父调用与子调用结果
- 已补 `telemetry.rs` 单测，覆盖 batch planned 子调用暴露与 nested 结果展开
- 本轮继续把“多工具计划”从隐式结果推进到显式结构：
- 新增 `ToolPlan / ToolPlanStep`
- `workspace_batch / workspace_gather_context` 已在结果中显式输出 `plan`
- `LocalTurnPlanner` 在多路径批量场景下已显式写入 `toolPlan`
- telemetry 已优先消费显式 `toolPlan`，减少仅靠 arguments/nested result 反推计划的歧义

## 本轮验证
- `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` 通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` 通过（41/41）
- 已补充 `tools.rs / planner.rs` 单元测试，覆盖：
- batch 的部分成功语义
- gather context 的文件聚合路径
- planner 对显式路径、历史路径、搜索语句的本地命中
- `npm run test:unit` 通过（含 tool/runtime 事件链前端回归）
- `npm run verify` 通过

## 下一步动作
- 继续评估是否把当前组合工具级 `ToolPlan` 再上提到 runtime 级通用计划对象
- 继续补 workspace 侧高价值工具，例如更稳的路径匹配、局部目录树和文件 diff / summary
- 评估工具权限层是否需要独立 trait 化，而不是继续放在 `ToolRouter` 内部

## 当前卡点
- 当前 runtime 仍以“单次工具 follow-up”作为主语义，因此多工具目前主要通过组合工具实现
- 若要继续推进到 provider 原生多工具，需要同时协调 runtime、telemetry 和 provider follow-up 语义

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/tools.rs`
- `src-tauri/src/agent/planner.rs`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/provider.rs`
- `docs/learning/0013-native-tools-protocols.md`
- `docs/learning/0016-tool-call-as-data-not-direct-function-call.md`
## 2026-05-23 补充进展
- `ToolPlan` 不再继续塞进 `ToolCall.arguments.toolPlan`，而是提升为 `ToolCall.plan`，组合工具的计划信息终于进入 runtime 级数据结构。
- `workspace_batch / workspace_gather_context` 的 direct/SSE probe 已覆盖“多路径”和“大结果”两类真实场景。
- OpenAI follow-up 现在对超大工具结果先做压缩摘要；若 sync/stream follow-up 仍失败，则退回本地整合响应，避免工具已经执行成功却整轮报错。
- 验证通过：
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- multipath-context`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- large-result`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-multipath`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-large-result --raw`
- 2026-05-24：本轮验收确认当前目标已完成：多 hop follow-up、组合工具 `ToolPlan`、更细粒度 telemetry、前后端回归链路都已闭环；未来若继续推进 provider-native 并发多工具，将新开后续任务。
