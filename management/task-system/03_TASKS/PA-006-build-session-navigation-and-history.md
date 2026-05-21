# PA-006 实现新对话与历史对话管理

## 状态
- Status: `In Progress`
- Priority: `P1`
- Owner: `Codex`

## 目标
在已有 `SessionStore` 基础上，把当前工作台从“单一固定会话”升级成“可新建、可切换、可恢复”的会话工作流，并让前后端围绕 `sessionId` 保持同一事实来源。

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
- 这一轮已进一步收口：
- 新建但还没发消息的空会话也会立即持久化，避免切换或重启后丢失
- 前端切换会话时会先校验本地缓存与后端 `snapshot.history` 是否一致；只有一致才恢复 trace、provider 统计等增强状态
- 若前端缓存与后端 snapshot 不一致，则以后端历史为真相源，避免串会话或串旧状态
- 切换/新建会话时会清空输入草稿
- 会话列表优先显示 `title`，`summary` 作为辅助信息

## 本轮验证
- `cargo check --target-dir ../target-check` 通过
- `cargo test session --lib --target-dir ../target-check` 通过
- `npm run build` 当前未作为本任务单独通过项，因为被 `PA-009` 引入的 provider 类型扩展影响；在 provider 线合流后需要做一次全量前端构建复验

## 下一步动作
- 继续收敛会话列表元数据和命名策略
- 明确切换会话时，消息区、trace、summary 和 provider 展示哪些需要重置、哪些需要恢复
- 继续优化清除会话后的回退逻辑和视觉反馈
- 为未来的 HTTP/SSE adapter 保持 session 接口稳定

## 当前卡点
- 还缺少真实 UI 手工回归，特别是“新对话 -> 不发消息直接切走 -> 再切回”这条路径
- provider 相关前端类型正在演进，session 线的全量前端构建需要等 provider 线一起复核

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/runtime.rs`
- `src/stores/runtime.ts`
- `src/components/HomeSessionSidebar.vue`
- `docs/architecture/runtime.md`
- `docs/learning/0017-session-store-and-context-boundary.md`
