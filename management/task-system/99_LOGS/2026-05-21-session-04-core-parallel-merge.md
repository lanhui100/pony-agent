# 2026-05-21 Session 04 - core parallel merge

## 背景
本轮把多智能体并行开发正式切换到“先 core、后 adapter”的策略，围绕三条主线并行推进：
- `PA-006` session/history 收口
- `PA-008` 工具层补强
- `PA-009` provider 能力完善

## 本轮合流内容
- 收回并检查了三条并行线的实际代码落地情况
- 确认 `PA-006 / PA-008 / PA-009` 都已经有真实代码修改，不再只是任务规划
- 清理了并行验证产生的临时目录，避免把 `target-check / target-test-pa008` 等中间产物带入提交
- 更新了任务板和关键任务卡，让状态与代码事实一致：
  - `PA-008` 从 Ready 进入 In Progress
  - `PA-009` 从 Ready 进入 In Progress
  - `PA-006` 补上本轮真实进展和验证结果

## 当前代码事实
- `PA-006`
  - 空会话会立即持久化
  - 切换会话时会校验前端缓存与后端 snapshot 是否一致
  - 不一致时以后端历史为真相源
  - 切换/新建会话会清空输入草稿
- `PA-008`
  - 新增 `workspace_batch`
  - 新增 `workspace_gather_context`
  - planner 能更稳地命中目录列举、搜索和上下文聚合
- `PA-009`
  - provider/model 已支持 `capabilities + reasoning`
  - 前端配置页可编辑 reasoning、多模态、上下文相关字段
  - OpenAI / Anthropic 请求已做最小 reasoning 透传

## 验证策略
- Rust：`cargo check`
- session：`cargo test session --lib`
- frontend：`npm run build`

## 后续重点
- 继续围绕这三条 core 主线收敛，而不是现在就抽 adapter
- 等 core 语义更稳定后，再回到 `PA-007`
