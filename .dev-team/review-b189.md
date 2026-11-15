# B189 变更审核报告：对齐 DSH 消除提问伪失效与误报

- 审核对象：commit `691eac5b`（fix(tui) 6 文件 / +321 −43）
- 审核范围：正确性、契约一致性（ADR 0074 / DSH）、错误处理、测试证据、账本与文档门禁
- 结论：**有条件通过（PASS WITH ISSUES）**。核心场景（本端作答后轮询误报）已修好，但「消除误报」只完成一半：跨端/并发冲突路径仍上屏「该提问已失效」，且这段新分支是**不可达代码**。

---

## 1. 复跑证据（均为本轮实跑）

| 校验 | 命令 | 结果 |
| --- | --- | --- |
| 账本门禁 | `npm run backlog:gate` | PASS：lint 活跃 14 / 归档 119；doctor `ok:true`、`leases:[]`、highWater 191 |
| 决策门禁 | `npm run adr:check` | PASS：verify-note 全绿；归档封印 117 件 sha256 一致 |
| 可观测性增量 | `audit-diff.sh --base HEAD~1` | PASS：3 文件 / +86 行，无越界实现 |
| 体积门禁 | `npm run size:check` | PASS：0 new violation（61 已豁免） |
| 编译 | `invoke-rust-target check check -p pony-agent-core` | exit 0 |
| TUI 全套单测 | `... test --bin pony` | **168 passed / 0 failed** |
| 新增用例 | `--bin pony sync_pending_control_request_silently_closes` | 1 passed |
| 契约用例 | `--test ask_pending_contract c1_7_` | 1 passed |
| 工作区 | `git status` | clean（无残留改动） |

账本归档记录（`backlog/archive/tasks-2026.jsonl`）含 `evidence` + `closed_at`，与上述实跑一致，证据成立性可复核。

---

## 2. 阻塞级发现

### F1（P1）「错误上屏精准化」分支不可达，误报依旧存在

- 位置：`crates/pony-agent-core/src/bin/tui.rs:7883-7890`
- 实现用**大小写敏感的英文散文**判定：`error.contains("is answered")` / `state=Answered` / `is cancelled` / `is superseded`。
- 唯一生产者是 `ask_persistence.rs:629-633`：
  `"ask_state_conflict: Ask `{}` is {:?}, not {:?} (terminal states are final)"`
  → 实际串为 `is Answered`（大写 A），且全仓不存在 `state=Answered` 形态。
- 机器验证（六个探针全部 False，stale 家族 True）：
  ```
  is answered   -> False      state=Answered  -> False
  is cancelled  -> False      state=Cancelled -> False
  is superseded -> False      state=Superseded-> False
  ask_state_conflict in msg -> True
  ```
- 可达路径：A 端已作答 → B 端仍挂面板并提交 → `answer_ask_in_session` 返回 `ask_state_conflict` → 进入 `report_control_request_error`，句柄仍为 Some 且 `is_approval=false` → 三个条件全不命中 → 落 `ASK_STALE_NOTICE`，**照样上屏「该提问已失效，已同步最新待处理请求」**。
- 与本仓既有纪律相悖：`is_stale_control_request_error` 注释明写「只认码，不认英文散文」，新代码却反向依赖散文并猜大小写。
- 修复方向：用稳定码 `ASK_ERR_STATE_CONFLICT` 判定，再调 `get_ask_lifecycle_state` 取真值决定文案（Answered / Cancelled / Superseded 三分支），彻底不解析宿主英文。

---

## 3. 应修项

### F2（P2）提交成功后提前清句柄，错误路径丢失上下文

`tui.rs:8075-8078` 在 `answer_ask_in_session` 成功后就 `state.pending_request = None`，随后才 `graph_resume_ask`。这使 `report_control_request_error` 里的 `handle.is_approval` 判定在该路径恒为 `None`（永远走兜底文案），F1 即便修正大小写也无法区分请求种类。建议在清句柄前固化 `was_approval` / 请求种类，或把错误映射前移到 answer、resume 各自分支。

### F3（P2）吞错误导致「面板无声消失」

- `ask_plan_commands.rs:80-84` 用 `if let Ok(Some(record))` 吞掉 store 读失败；trait 默认实现（`ask_persistence.rs:156-163`）对不支持 scoped read 的 store 恒返回 Err，被同等吞掉。
- 后果：读失败 → 返回 `None` → TUI `_ =>` 分支静默收口（`tui.rs:7825-7830`，无 notice、无 eprintln），用户看到面板凭空消失、日志无痕；与 `list_pending_asks` 出错时 eprintln fail-closed 的处理方式不一致，也违背「绝不静默」的排障纪律。
- 建议：接口改 `Result<Option<AskLifecycleState>, String>` 或至少在 Err 分支打日志；UI 把「读失败」与「记录不存在」分开处理。

### F4（P2）决策记录引用不存在的文档

`2026-11-15-dsh-aligned-ask-lifecycle-close.md` 称 DSH 规范见 `docs/subsystems/user-questions.zh.md`；全仓 `find` 无此文件（`docs/` 下亦无 `subsystems/` 目录），证据链不可复核。应改指真实真源（ADR 0074 或 `docs/audit/raw/dsh.md`）。

### F5（P2）未更新 `docs/observability.yaml`

AGENTS.md 常载规则要求：修改 UI 交互前先更新四维归属。本次改变 TUI 收口文案与提示行为，yaml 中无对应登记（`grep "伪失效|提问已作答"` 为空）。`audit-diff` 通过只证明「无越界实现」，不等于登记完成。

### F6（P3）`Expired → Superseded` 映射语义错误

`ask_plan_commands.rs:99-101`：`Expired` 是审批墙钟 TTL 概念，Ask 已按 ADR 0074 废除 TTL；若真出现 `Expired`，UI 会提示「已同步最新对话，提问已收起」，与事实不符。应映射 `Orphaned` 或显式 `None` + 日志。

### F7（P3）测试覆盖缺口

- `report_control_request_error` 的四个新文案分支**零测试**——这正是 F1 死分支未被发现的原因。
- TUI 只覆盖 `Answered` 收口；`Pending`（防误关核心分支）、`Cancelled`/`Superseded`、记录缺失、store 读错均无用例。
- `c1_7` 未覆盖 `Superseded` / `Orphaned`。
- 新用例只删主 db 文件，残留 `-wal` / `-shm`（NIT）。

### F8（P3）跨端一致性残留

桌面端 `src/stores/ask.ts:262` 与 `:305` 仍在 CAS 冲突 / resume 失败时上屏 `ASK_STALE_MESSAGE`（同一句「该提问已失效」），即同一误报在 Tauri UI 未修。B189 标注为 tui 范围可接受，但 DSH 对齐是产品面结论，建议另开后续单。

### F9（NIT）可读性

三处分支重复「清句柄 + 关覆盖层 + set_status_notice」四行；`AskLifecycleState` 三次全路径书写；`tui.rs:7834` 的兜底 `close_stale_pending_request` 在 `pending_panel_should_close` 语义下不可达（`None` 恒 false）。

---

## 4. 已确认正确的部分

- 收口判定从「不在 pending 快照即失效」改为「以持久化生命周期为单一真源」，方向正确，符合 ADR 0074 / DSH「提问无超时」心智。
- `Answered` 静默收口（仅状态栏提示，不上屏 System 消息）已由新用例守住，回退实现会立刻转红。
- 审批（Approval）保留 TTL 与专属 `APPROVAL_STALE_NOTICE`，未被打平，Wave 65 R3 决策未回退。
- `submit_ask_answers` 先清句柄再续轮，消除了「500ms 轮询抢跑」竞态（原始缺陷 P0 场景）。
- 账本闭环（删除活跃行 + 归档 done + evidence）与决策记录（含 `## Alternatives considered`）齐备，相关门禁全绿。

---

## 5. 建议动作

1. **立刻补一条修复单（P1）**：F1 改为「稳定码 + 真值查询」选文案，并补 `report_control_request_error` 四分支单测（否则同类死码会再次逃逸）。
2. 同单内顺带：F4 引用修正、F5 `observability.yaml` 登记、F3 错误不再静默。
3. 记录后续单：F8 桌面端同类误报、F6 `Expired` 映射、F7 补 `Pending` 防误关用例。
