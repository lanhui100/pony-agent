# PA-005 把 Vue 工作台接入真实 turn 执行链路

## 状态
- Status: `Done`
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
- 前端已从单次阻塞式 `run_turn` 等待，切到事件驱动的 turn 流
- Rust 已能发出 `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed` 事件
- OpenAI 兼容协议与 Anthropic 协议都已接入真实 stream 骨架
- UI 已能实时更新 assistant 文本、`phase`、`traceSteps`、`toolActivities`、`sessionSummary`、`providerName`、`providerProtocol`、`providerModel`、`providerMode`、`fallbackReason`
- 输入区已经支持 `Enter` 发送、`Shift+Enter` 换行，assistant 消息支持 Markdown 渲染并在消息尾部显示 `provider/model`
- 浏览器预览模式已补上 Tauri 环境检测与兜底，不再因为 `npm run dev` 直接白屏
- 当前主链路已从“静态占位”推进到“真实 provider + mock fallback + 原生 tools + 最小流式回包闭环”
- 工具侧栏已能区分 `planned / running / done / error`，`trace` 里的调用工具步骤也能显式标出失败态
- 前端当前会把最近几轮 `history` 一起发送给 Rust runtime，后端 planning 与本地工具推断已开始消费这段最小多轮语境
- “文件解释 -> 继续问该文件第 N 行”这条多轮工作流已联调通过：可以从最近用户消息中回溯 `tauri.conf.json`，并命中 `workspace.read_file_segment`
- 当前真实会话状态已开始由 Rust `SessionStore` 持有，`sessionId`、session snapshot 和回写逻辑已经从前端临时 history 中收回到 core
- 已把 prompt caching 纳入这一层的后续设计约束：history 不应无节制重写，工具清单和稳定指令区应尽量保持前缀稳定
- 2026-05-23 这一轮继续完成工作台收口：
- `runtime` store 已统一 browser-preview、失败态和 trace step 的状态流，减少前端临时拼装逻辑
- 已新增 `HomeWorkspace` 组件测试，覆盖“会话切换禁用输入”和“失败横幅展示”
- 已通过 `tauri dev --no-watch` 冒烟确认真实桌面壳可以拉起，不再只停留在 `cargo check`

## 下一步动作
继续补可见性，并开始为“独立 agent core”收边界：
- 在主页更直观地区分真实 provider 与 mock fallback
- 验证两类 provider 的真实 stream 体验，收敛事件字段命名
- 在不破坏当前事件模型的前提下，继续补 tool 调用的多工具边界
- 继续收束 history 策略，把“最近几轮消息”进一步升级成更明确的 session/runtime 状态，并减少前端对隐式固定会话的依赖
- 在 history/session 重构时，明确区分“稳定前缀上下文”和“本轮增量上下文”，兼顾 provider cache 命中
- 明确当前 Tauri event 流与未来 HTTP/SSE event 流的共用事件契约

## 当前卡点
- 主链路已接通，当前卡点已经从“是否能流式工作”转为“tool event 是否足够稳定、session 是否足够可管理、是否便于未来脱离 Tauri 复用”
- 当前没有阻塞本任务推进的编译或测试问题；后续主要是边界整理和真实 provider 长链路回归

## 断点续跑提示
继续前先看：
- `src/stores/runtime.ts`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/provider.rs`
- `src/types/runtime.ts`
- `docs/architecture/frontend-workbench.md`

## 备注
- 2026-05-20：前端侧边栏语法问题已修复，`npm run build` 通过。
- 2026-05-20：当前主线已从“静态占位”转为“真 turn + 真 stream + 真 session”，后续继续围绕 agent core 收口。
- 2026-05-24：真实 stream、trace、tool activity、provider/source/mode/fallback 展示与测试闭环已具备，任务完成。
