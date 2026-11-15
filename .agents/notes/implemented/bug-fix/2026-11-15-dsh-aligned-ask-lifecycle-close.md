# Agent Note: 对齐 DSH 消除提问伪失效：精准生命周期收口与面板静默

Status: implemented

## Problem

用户反馈：在 TUI 交互提问后，界面弹窗及消息历史出现「该提问已失效，已同步最新待处理请求」（会话 `pa_01a1d5c2-1cc6-7952-b1a4-242fd15a8dcd`），用户困惑为何正常提问会有失效时间。

经过对会话底层数据库（`sessions.db`）和运行时日志（`tui-runtime.log`）排查，提问实际已于 28 分钟后由用户成功作答（写入 `state = answered` 且成功触发续轮），但 TUI 界面依然错误上屏「该提问已失效」。根因如下：

1. **粗暴的「非 Pending 即失效」归因短板**：
   `sync_pending_control_request` 每 500ms 轮询 `list_pending_asks`（该列表仅返回 `state == 'pending'` 的请求）。一旦提问在当前端提交作答落地、或在多端/并发进程中转为终态，`pending_panel_should_close` 立即命中。旧实现盲目调用 `close_stale_pending_request`，将「不在 pending 列表」无差别判定为「提问失效」，并向用户聊天流强行注入 System 消息 `ASK_STALE_NOTICE`。
2. **违背 DSH 人机交互契约中提问无超时产品心智**：
   参考 DeepSeek Harness（DSH）规范（`docs/subsystems/user-questions.zh.md`），人类的思考时间绝不是倒计时资源。阻塞式提问无期限，前台即使超时也不丢弃提问，提问只有在明确被回答（Settled）或主动取消时才会收口，绝不存在「超时作废」。
3. **提交回答与轮询收口的并发竞态**：
   在 `submit_ask_answers` 中，存储层持久化成功与界面句柄置空之间存在窗口，极易被 500ms 轮询抢跑触发伪失效收口。

## Decision

1. **控制面暴露真实生命周期查询接口**：
   `HostControlPlane` 新增 `get_ask_lifecycle_state(session_id, request_id) -> Option<AskLifecycleState>`，直接从持久化底层读取真实状态（`Pending` / `Answered` / `Cancelled` / `Superseded`），不再仅靠单一 `list_pending_asks` 快照猜状态。
2. **TUI 面板按真实生命周期精准收口**：
   当提问不再出现在 pending 快照中时，先查询底层真实生命周期状态：
   - `Answered`（已作答）：静默收口面板，仅在状态栏温和提示，**绝不上屏「该提问已失效」系统消息**；
   - `Pending`（仍未决）：说明快照缺失仅为瞬时读取延迟或锁竞争，**保持面板打开，绝不误关**；
   - `Cancelled`（显式忽略）：温和提示「提问已忽略」并收口；
   - `Superseded`（新对话取代）：温和提示「已同步最新对话，提问已收起」并收口；
   - 审批（Approval）保留墙钟 TTL 语义与专属的过期提醒。
3. **消除提交与轮询的并发竞态**：
   在 `submit_ask_answers` 中，一旦作答在存储层提交成功，立即原子化清理面板句柄并关闭覆盖层，杜绝轮询抢跑。
4. **错误上屏精准化**：
   在 `report_control_request_error` 中，如果错误属于已作答状态冲突，提示「该提问已完成作答，已同步最新状态」，杜绝向用户上屏「失效」误导信息。

## Alternatives considered

- **A. 直接删除 `ASK_STALE_NOTICE` 全部静默收口**：不可行。审批（Approval）确实存在墙钟 TTL，如果审批过期也静默收口，用户会干等一个永远不会继续的轮次（违背 Wave 65 R3 决策）。
- **B. 仅通过延长 TTL 解决**：不可行。ADR 0074 已经明确废除 Ask 的墙钟 TTL，提问在存储层早已没有失效时间，问题根源出在 UI 侧将「已答完」粗暴判定为「失效」，改时限无法解决任何实质问题。
- **C. 仅在按键提交时设布尔标记**：不可行。对于多终端并发、重连、断线恢复等场景，外来状态迁移依然会被误判为失效。必须以存储层真实生命周期状态为单一真源。
