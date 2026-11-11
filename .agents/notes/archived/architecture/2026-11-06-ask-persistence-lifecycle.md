# Agent Note: Ask 控制请求：会话持久化 + 生命周期收口（墙钟 TTL 退出产品语义）

Status: archived (原状态: proposed)

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0074-ask-persistence-lifecycle.md` 之归档快照，仅供溯源。

## 背景

`builtin:ask_user`（模型面 `Ask`）的待决请求是 `PendingControlRequest`，只存在于
`GovernedDispatcher` 的进程内 `Mutex<BTreeMap>`（`crates/pony-agent-core/src/agent/dispatcher.rs:492`），
时效来自单一常量 `DispatchBudgetConfig::control_request_expiry_ms`（默认 60_000，`budget.rs:85`），
由 `can_consume` 以 `now_ms <= expires_at_ms` 判定可消费（`tool_runtime.rs:205-217`）。三条后果已被真实事件验证：

1. **退出即丢失**：进程结束或重开会话后 `list_pending_asks` 为空，面板不会回来；历史里只剩一条
   `control_outcome_pending` 工具结果，模型只能重新提问。
2. **人的思考时间被当成资源约束**：60s 到期后提交必然撞 CAS，用户原样看到
   `control request \`…\` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch`；
   TUI 侧原无 expire 调用点，过期请求仍留在 `Pending`，500ms 轮询把死面板反复拉起、连 Esc 取消也失败。
   （2026-11-06 已用真实 TUI + 隔离 HOME 复现，并在 TUI 侧做了最小兜底：本地守卫 + 对账 + 中文提示。）
3. **两份真值必然漂移**：会话历史里已有该工具调用，"未决"本应是它的状态；另存一份内存态后，
   面板（内存）与历史（落盘）各自演化——这正是 1、2 的根因结构。

用户裁决（2026-11-06）：**未决 Ask 是会话状态，不是计时资源**；只要它是会话里最后一个未完成交互，
重启、重连、换端后都应重建面板继续等待回答。

## 候选方案

| 方案 | 做法 | 优点 | 落选/入选理由 |
| :-- | :-- | :-- | :-- |
| **A** 维持现状（内存 + 60s TTL） | 只做 UI 守卫与中文文案 | 改动最小；已具备 CAS 与一次性消费 | **落选**：退出丢失、思考被计时、两份真值三条缺陷原样保留；用户明确否决 |
| **B** 拉长 TTL（如 10min / 24h） | 调 `control_request_expiry_ms`，按 kind 分时限 | 一行常量见效；不碰存储 | **落选**：任何墙钟值都是拍脑袋（请假、合盖、跨天都会踩）；仍不解决重启丢失；把"确定性的生命周期判据"换成"更大的魔法数" |
| **C 持久化 + 生命周期收口（选中）** | 未决 Ask 落 `sessions.db`；状态机 `Pending→Answered/Cancelled/Superseded/Orphaned`；TTL 退化为 GC；两端同版去倒计时、恢复即重建、显式忽略 | 单一真值；重启/换端可答；判据确定性；消掉过期报错与死面板循环 | **入选**：直接命中三条缺陷；代价是 schema + 控制面 + 两端 UI 的跨模块改动（见影响） |
| **D** 持久化 + 保留长 TTL 兜底 | 同 C，但保留 `expiresAtMs` 作为"保险" | 迁移期风险更低；能自动清理陈旧记录 | **落选**：TTL 一旦存在，UI 就必然要处理"到期"分支，产品语义重新变成计时器；清理需求改由会话归档 + 保留期 GC 满足，不需要把期限暴露给用户 |

## 决策

未决 Ask 作为**会话持久状态**落 `sessions.db`，状态机为
`Pending → Answered | Cancelled | Superseded | Orphaned`；取代判据是"**会话中出现了新的用户消息**"
（旧 Pending 转 `Superseded`），会话归档/删除转 `Orphaned` 并 GC。
`version` 与 `nonce` 随请求落盘，使 CAS 跨进程仍成立；墙钟 TTL 不再参与作答判定，
`ask_expire` 退化为 GC/显式忽略。TUI 与桌面端同版：去倒计时与"已过期"态、恢复会话即重建面板、
提供"忽略此提问"、冲突只上屏中文文案。

## 影响

- **契约级变更**：`crates/pony-agent-core/src/agent/session/`（schema + 迁移）、
  `control_plane/ask_plan_commands.rs`（读写改走存储）、`dispatcher.rs`（CAS 事实来源）、
  `src-tauri/src/lib.rs`（命令语义）、`src/stores/ask.ts` + `AskPanel.vue` + TUI 面板。
- **不可逆面**：`sessions.db` 新增表（向前迁移可开旧库，但新库不被旧版本识别——需在发布说明点名）；
  `tests/ask-store.spec.ts` 中 C2-* 过期语义需重冻。
- **必须同时解决**：回答落点。当前 TUI 常见 `bound_run=None`，回答只被消费不续跑；
  本决策要求"有 graph wait 则 resume，否则以 tool result 注入下一轮"，否则持久化只是把答案存起来没人看。
- 约束后续：任何人不得再以"过期"为由拒绝作答；新增客户端必须实现"恢复即重建 + 显式忽略"两条。

