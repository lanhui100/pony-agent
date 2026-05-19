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
- 输入区已经支持 `Enter` 发送、`Shift+Enter` 换行，assistant 消息支持 Markdown 渲染并在消息尾部显示 `provider/model`。
- 浏览器预览模式已补上 Tauri 环境检测与兜底，不再因为 `npm run dev` 直接白屏。
- 当前主链路已从“静态占位”推进到“真实 provider + mock fallback + 原生 tools + 最小流式回包闭环”。
- 工具侧栏已能区分 `planned / running / done / error`，`trace` 里的调用工具步骤也能显式标出失败态。
- 前端当前会把最近几轮 `history` 一起发送给 Rust runtime，后端 planning 与本地工具推断已开始消费这段最小多轮语境。
- “文件解释 -> 继续问该文件第 N 行”这条多轮工作流已联调通过：可从最近用户消息中回溯 `tauri.conf.json`，并命中 `workspace.read_file_segment`。

## 下一步动作

继续补可见性，并开始为“独立 agent core”收边界：

- 在主页更直观地区分真实 provider 与 mock fallback
- 展示 `providerMode / fallbackReason / token 统计 / 首 token 延迟`
- 验证两类 provider 的真实 stream 体验，收敛事件字段命名
- 在不破坏当前事件模型的前提下，继续补 tool 调用的多工具边界
- 继续收束 history 策略，把“最近几轮消息”升级成更明确的 session/runtime 状态，而不长期停留在前端临时拼接
- 明确当前 Tauri event 流与未来 HTTP/SSE event 流的共用事件契约

## 当前卡点

- 主链路已接通，当前卡点已经从“是否能流式工作”转为“运行指标是否足够直观、tool event 是否足够稳定、history/session 是否足够明确、是否便于未来脱离 Tauri 复用”

## 断点续跑提示

继续前先看：

- `src/stores/runtime.ts`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/provider.rs`
- `src/types/runtime.ts`
- `docs/architecture/frontend-workbench.md`
