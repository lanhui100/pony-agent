# PA-006 实现新对话与历史对话管理

## 状态

- Status: `In Progress`
- Priority: `P1`
- Owner: `Codex`

## 目标

在已有 `SessionStore` 基础上，把当前工作台从“单个固定会话”升级成“可新建、可切换、可恢复”的会话工作流。

## 输出

- 新对话入口
- 历史对话列表
- 会话切换与恢复
- 前后端统一的 `sessionId` 交互约定

## 验收标准

- 用户可以在 UI 中显式发起一个新对话
- 用户可以看到已有会话列表，并切换到历史会话
- 切换会话后，消息区、session summary 和运行状态能同步恢复
- 前端不再把当前会话写死成单一固定值，而是基于可管理的 `sessionId`

## 当前进展

- Rust core 已有 `SessionStore`，支持 `sessionId -> session state`
- session 已开始持久化到 `.pony-agent/sessions.json`
- runtime 已在 `run_turn()` / `start_turn_stream()` 中通过 session snapshot 读取上下文，并在 turn 结束后统一回写
- 前端 `submitTurn()` 已开始携带 `sessionId`
- 左侧“对话历史”已可折叠，支持新建、切换、清除会话
- 右侧状态栏已收回为纯状态 + trace
- 当前还可以继续收束会话标题、清除后的回退体验，以及更细的历史元数据展示

## 下一步动作

- 继续收束会话列表元数据和命名策略
- 明确切换会话时，消息区、trace、summary 和 provider 展示哪些需要重置、哪些需要恢复
- 继续优化清除会话后的回退逻辑和视觉反馈
- 为未来的 HTTP/SSE adapter 保持 session 接口稳定

## 当前卡点

- 会话元数据还比较轻，后续可能需要补标题、更新时间、摘要等列表展示字段
- 要避免把 Tauri 当前实现写死成最终会话接口，需兼顾未来 HTTP/SSE adapter 复用

## 断点续跑提示

继续前先看：

- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/runtime.rs`
- `src/stores/runtime.ts`
- `docs/architecture/runtime.md`
- `docs/learning/0017-session-store-and-context-boundary.md`
