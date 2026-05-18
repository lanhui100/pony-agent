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

- 前端已从单次阻塞式 `run_turn` 等待，切到事件驱动的 turn 流。
- Rust 已能发出 `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 事件。
- OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架。
- UI 已能实时更新 assistant 文本、`phase`、`traceSteps`、`toolActivities`、`sessionSummary`、`providerName`、`providerProtocol`、`providerModel`、`providerMode`、`fallbackReason`。
- 当前主链路已从“静态占位”推进到“真实 provider + mock fallback + 最小流式回包闭环”。

## 下一步动作

继续补可见性与学习友好度：

- 在主页更直观地区分真实 provider 与 mock fallback
- 展示 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
- 验证两类 provider 的真实 stream 体验，收敛事件字段命名
- 在不破坏当前事件模型的前提下，为后续 tool 调用事件预留更细粒度状态

## 当前卡点

- 主链路已接通，当前卡点已经从“是否能流式工作”转为“运行指标是否足够直观、是否便于产品化展示”

## 断点续跑提示

继续前先看：

- `src/stores/runtime.ts`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/provider.rs`
- `src/types/runtime.ts`
- `docs/architecture/frontend-workbench.md`
