# PA-005 把 Vue 工作台接入真实 turn 执行链路

## 状态

- Status: `In Progress`
- Priority: `P1`
- Owner: `Codex`

## 目标

让当前 Vue 工作台不再只展示静态占位信息，而是能驱动真实的 Rust turn 执行并回显结果。

## 输出

- 前端输入动作
- Tauri command 调用
- 返回结构化 turn 结果
- 面板中展示真实 phase、trace、message

## 验收标准

- 用户可以在工作台中发起一次 turn
- Rust 返回结果后，UI 面板同步更新
- 不需要先引入复杂多轮会话

## 当前进展

- 前端已可通过 Tauri `run_turn` command 发起真实回合。
- Rust 返回后，UI 已能同步更新 `phase`、`traceSteps`、`toolActivities`、`sessionSummary`、`providerName`、`providerProtocol`、`providerModel`、`providerMode`、`fallbackReason`。
- 当前主链路已从“静态占位”推进到“真实 provider + mock fallback 的最小闭环”。
- 剩余问题主要是状态解释层不足，而不是链路未接通。

## 下一步动作

继续补可见性与学习友好度：

- 在主页更直观地区分真实 provider 与 mock fallback
- 展示凭证来源 / fallback 原因 / 当前回合模式
- 评估是否补充更直接的 runtime 状态字段，减少前端推断

## 当前卡点

- 主链路已接通，但“能跑”和“能一眼看懂”之间还有一段体验差距

## 断点续跑提示

继续前先看：

- `src/stores/runtime.ts`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/provider.rs`
- `docs/architecture/frontend-workbench.md`
