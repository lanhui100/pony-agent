# Agent Note: 对齐 DSH 消除提问伪失效：精准生命周期收口与面板静默

Status: implemented

## Problem

用户反馈：在 TUI 交互提问后，界面弹窗及消息历史出现「该提问已失效，已同步最新待处理请求」（会话 `pa_01a1d5c2-1cc6-7952-b1a4-242fd15a8dcd`），用户困惑为何正常提问会有失效时间。

经过对会话底层数据库（`sessions.db`）和运行时日志（`tui-runtime.log`）排查，提问实际已于 28 分钟后由用户成功作答（写入 `state = answered` 且成功触发续轮），但 TUI 界面依然错误上屏「该提问已失效」。根因如下：

1. **粗暴的「非 Pending 即失效」归因短板**：
   `sync_pending_control_request` 每 500ms 轮询 `list_pending_asks`（该列表仅返回 `state == 'pending'` 的请求）。一旦提问在当前端提交作答落地、或在多端/并发进程中转为终态，`pending_panel_should_close` 立即命中。旧实现盲目调用 `close_stale_pending_request`，将「不在 pending 列表」无差别判定为「提问失效」，并向用户聊天流强行注入 System 消息 `ASK_STALE_NOTICE`。
2. **违背仓库已冻结的「提问无墙钟期限」决策**：
   决策真源是 `.agents/notes/archived/architecture/2026-11-06-ask-persistence-lifecycle.md`（ADR 0074：墙钟 TTL 退出产品语义，任何人不得再以「过期」为由拒绝作答），配套闭环见 `.agents/notes/implemented/bug-fix/2026-11-11-approval-listing-and-no-wall-clock-expiry.md`。人类的思考时间绝不是倒计时资源：阻塞式提问无期限，只有在明确被回答（Settled）、主动忽略（Cancelled）、被新消息取代（Superseded）或会话归档（Orphaned）时才收口，绝不存在「超时作废」。
3. **提交回答与轮询收口的并发竞态**：
   在 `submit_ask_answers` 中，存储层持久化成功与界面句柄置空之间存在窗口，极易被 500ms 轮询抢跑触发伪失效收口。

## Decision

1. **控制面暴露真实生命周期查询接口**：
   `HostControlPlane` 新增 `get_ask_lifecycle_state(session_id, request_id) -> Result<Option<AskLifecycleState>, String>`，直接从持久化底层读取真实状态（`Pending` / `Answered` / `Cancelled` / `Superseded` / `Orphaned`），不再仅靠单一 `list_pending_asks` 快照猜状态。`Err` 专表**读失败**（IO/锁/缺作用域），与「没有这条记录」严格区分。
2. **TUI 面板按真实生命周期精准收口**：
   当提问不再出现在 pending 快照中时，先查询底层真实生命周期状态：
   - `Answered`（已作答）：静默收口面板，状态行提示「该提问已完成作答，已同步最新状态」，**绝不上屏「失效」类系统消息**；
   - `Pending`（仍未决）：说明快照缺失仅为瞬时读取延迟或锁竞争，**保持面板打开，绝不误关**；
   - `Cancelled`（显式忽略）：「该提问已忽略，已同步最新状态」并收口；
   - `Superseded`（新对话取代）：「该提问已被新对话取代，已同步最新状态」并收口；
   - `Orphaned`（会话归档）：「该提问所属会话已归档，已同步最新状态」并收口；
   - 读失败（`Err`）：**保持面板** + `ask_lifecycle_read_failed` 结构化日志 + 状态行提示（同一失败只提示一次，不刷屏）；
   - 审批（Approval）保留墙钟 TTL 语义与专属的过期提醒（`APPROVAL_STALE_NOTICE`）。
3. **消除提交与轮询的并发竞态**：
   在 `submit_ask_answers` 中，一旦作答在存储层提交成功，立即原子化清理面板句柄并关闭覆盖层，杜绝轮询抢跑；句柄快照在清理前取出，供失败上报使用。
4. **错误上屏精准化**：
   `report_control_request_error` 不再解析宿主英文散文，改为按「稳定错误码族 + 生命周期真值」选文案：CAS 冲突（`ask_state_conflict` 等五码）先查 `get_ask_lifecycle_state`，按终态给中性结论；句柄未知或读失败时只给中性兜底句（「该提问已收口，已同步最新状态」），不再断言「失效」。

## Alternatives considered

- **A. 直接删除 `ASK_STALE_NOTICE` 全部静默收口**：不可行。审批（Approval）确实存在墙钟 TTL，如果审批过期也静默收口，用户会干等一个永远不会继续的轮次（违背 Wave 65 R3 决策）。
- **B. 仅通过延长 TTL 解决**：不可行。ADR 0074 已经明确废除 Ask 的墙钟 TTL，提问在存储层早已没有失效时间，问题根源出在 UI 侧将「已答完」粗暴判定为「失效」，改时限无法解决任何实质问题。
- **C. 仅在按键提交时设布尔标记**：不可行。对于多终端并发、重连、断线恢复等场景，外来状态迁移依然会被误判为失效。必须以存储层真实生命周期状态为单一真源。
- **D. 用 `error.contains(...)` 猜宿主散文选文案**（首轮实现，复审否决）：宿主串形如 `ask_state_conflict: Ask … is Answered, not Pending`，大小写与措辞随实现漂移，一处 `contains` 对不上就把「已作答」误报成「失效」；且与 `is_stale_control_request_error` 注释「只认码，不认英文散文」自相矛盾。

## Review fixes（复审轮：P1–P3）

复审对本轮交付提出 8 条问题（F1–F9），已逐条收口：

- **F1（P1）错误文案分派不可达**：首轮用 `error.contains("is answered")` / `state=Answered` 匹配，而真实串是 `ask_state_conflict: Ask … is Answered, not Pending`（大写、且全仓无 `state=…` 形态生产者），三分支永不命中——跨端作答后本端再提交依旧上屏「失效」。现改为只认稳定码 + 生命周期真值（`ask_settled_notice` 纯函数），并用**真实错误串**做回归锁。
- **F2（P2）提交路径丢句柄上下文**：`submit_ask_answers` 在 resume 前清 `pending_request`，失败上报只能退回兜底句。现先取 `submitted_handle` 快照再清句柄。
- **F3（P2）生命周期查询吞错误**：接口改为 `Result<Option<_>, _>`；TUI 读失败时保持面板并留结构化日志（见 Decision 2）。
- **F6（P3）`Expired → Superseded` 语义污染**：`Expired` 是审批镜像态，Ask 无墙钟 TTL；现记诊断日志后返回 `None`，不再给出「已被新对话取代」的错误结论。
- **F8（P3）桌面端同源误报**：新增 Tauri 只读命令 `ask_lifecycle_state`，前端 `askSettledNotice()` 按真实终态选文案；读失败/旧宿主退回中性兜底，两处 `ASK_STALE_MESSAGE` 兜底字面量同步改为中性句。
- **F4/F5（P2）文档**：修正上文对 DSH 规范的不实引用（`docs/subsystems/user-questions.zh.md` 全仓不存在），改指仓库内决策真源；`docs/observability.yaml` 补本轮四维归属登记（新增本地取证行，无新增 applicable / enforcement_point）。
- **F7（P2）测试缺口**：新增 `bin/tui/tests/ask_lifecycle_settle_tests.rs`（8 例：纯函数分派、终态收口、`Pending` 保持、读失败保持、错误文案回归、审批文案隔离）；`ask_pending_contract` 补 `Superseded` / `Orphaned` / 读失败三态；前端补 2 例（真值文案 + 读失败兜底）。
- **F9（NIT）**：重复的「清句柄 + 关面板 + 状态行」抽成 `settle_ask_panel_by_lifecycle`；`AskLifecycleState` 改为顶部 `use`。

### 收据

- `cargo test --bin pony`（TUI 内嵌 176 例，含新增 8 例）：通过。
- `cargo test --test ask_pending_contract c1_`（含 c1_7 五态 / c1_8 读失败）：8 通过。
- `vitest run tests/ask-store.spec.ts tests/ask-pending-expiry.spec.ts`：30 通过（含新增 2 例）。
- `npm run adr:check` / `npm run backlog:gate` / `npm run size:check` / `observability audit-diff`：全绿。
