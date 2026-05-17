# TASK-002 增强 run_turn() 的运行状态可视化

- 状态：`In Progress`
- 优先级：`P1`
- 负责人：`Codex / 用户协作`

## 目标

让学习阶段的 `run_turn()` 不只是“能跑”，而是“能看懂”。

## 预期输出

- 对话区或右侧状态面板展示更清晰的执行状态
- 能看出本轮：
  - 是否走了工具判断
  - 是否执行了工具
  - 使用了哪个 provider / protocol / model
  - 会话处于什么阶段

## 验收标准

- 用户能基于 UI 理解一轮最小 agent 回合发生了什么
- 不破坏“对话区为主体”的布局原则

## 当前进展

- `run_turn()` 已可返回 phase、traceSteps、toolActivities、toolExecution、sessionSummary
- 前端已有右侧折叠面板
- 但状态文案和信息分层还不够强

## 下一步动作

继续优化主页状态区文案、标签和展开细节，并评估是否需要在 Rust 响应中增加更直接的状态字段。

## 断点续作提示

优先检查：

- [runtime.rs](C:/Users/HUAWEI/Documents/New%20project/src-tauri/src/agent/runtime.rs)
- [main.ts](C:/Users/HUAWEI/Documents/New%20project/src/main.ts)
- [styles.css](C:/Users/HUAWEI/Documents/New%20project/src/styles.css)
