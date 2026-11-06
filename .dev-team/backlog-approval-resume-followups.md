# wave-65 审批可恢复：R2 审查遗留与后续工单（Lead 登记）

> 来源：`.dev-team/review-plan-approval-resume.json`、`review-approval-resume-a/b.json`、`review-l3p-approval-resume.json` 的 ADVISORY 与 R2 裁决。
> 原则：R2 已修 BLOCKING 项；下列条目**不**在本波扩大闭包，逐条登记责任与重开判据。

| id | 事项 | 触发证据 | 重开判据 |
|---|---|---|---|
| F-1 | 非路径审批（`tool_rule_ask` / `external_requires_approval` / 写执行保守抬升）目前保持 fail-fast，未接入可恢复挂起 | `dispatcher.rs` 的 `approvalKind:"approval"` 分支；ADR 0077 R2 裁决 | 需要 dispatcher 级**一次性豁免（waiver）**：批准后在同一 turn 重试该工具时策略评估器能命中豁免，否则续轮必再审批。新增 waiver store（act）/ CAS 语义 + e2e |
| F-2 | TUI 接线「授权→批准→续轮」正向路径仍无自动化 e2e（**可测未测**：本仓 32 个集成测试已用 `#[path="../src/bin/tui.rs"]` bin-as-module 形态，正向用例可用 `approval_resume_e2e.rs` 的 `run_turn_until_approval_suspension` + `pending_approvals` 造真实 Pending Approval 后直接调 `apply_approval_decision`，断言 `list_authorizations`/`list_pending_approvals`/续轮文案） | `review-plan` BLOCKING#4；`review-a2`/`l3p-r2` 复核；契约 C5 R2 增补 | 按上述配方补 `tests/approval_seam_e2e.rs`（Test Agent 唯一写），覆盖 authorize+approve 两副作用；删接缝实现必须变红 |
| F-3 | 批准顺序「先授权后 CAS」：CAS 失败会残留一条 read 授权 | `review-b` ADVISORY SECURITY（tui.rs:8791） | 改为「先 CAS 预占 + 授权 + 失败回滚授权」，或授权记录带 request_id 关联以支持精确撤销 |
| F-4 | 审批请求仅进程内存储；进程重启后悬挂审批静默消失（无提示） | `review-b` LEAK、`review-plan` ADVISORY | 复用 `ask_persistence` 的 durable store 承载 Approval（含 kind 过滤），启动时 `list_pending_approvals` 重建面板 |
| F-5 | `dispatcher.pending_requests()` 用 `expect()`；`list_pending_approvals` 每 500ms 从 UI 线程调用，锁毒化会 panic UI | `review-b` ADVISORY CONCURRENCY（dispatcher.rs:940） | 统一 poison 恢复（同文件其他点已用 `unwrap_or_else(into_inner)`），或列表接口改为只读快照 + 缓存 |
| F-6 | 续轮无独立重入闸门（仅 `turn_loop_in_flight` 信号，guard Drop 会清） | `review-b` ADVISORY CONCURRENCY（tui.rs:11205） | 与排队派发共用同一把「在飞轮次」互斥（或续轮前置检查 `is_streaming == false` 的强断言） |
| F-7 | `AllowOnce` 与 `AllowSession` 同路：落盘 read 授权跨重启持久，无 revoke UI | `review-plan` ADVISORY（tui.rs:8779） | 产品层决定 `AllowOnce` 语义（一次性授权）或补授权清单管理面板（`authorize_path` 已有 revoke 能力） |
| F-8 | 审批面板展示的 `options.path` 原始串与落盘授权的 canonical 路径可能不同（符号链接/`..`） | `review-a` ADVISORY SECURITY（tui.rs:7388） | 面板回显 canonical 路径（与 `authorize_path` 同源归一化），或展示双路径 |
| F-10 | ADR 编号冲突：本波与并行会话同时占用 0076（已把本波重编号为 0077）；`docs/decisions/README.md` 索引行随并行会话提交后再补 | `review-l3p-r2` ADVISORY | README 索引补齐 0077 行 |
| F-9 | 新增 3 处 `eprintln!` 非 JSON/无 trace_id（PA-119 日志债务增量） | `review-l3p` ADVISORY（control_plane/mod.rs:1397） | 随 PA-119 统一日志面迁移，或本模块单独接 `tracing` |
| F-11 | 契约/ADR 登记滞后（C3 过滤条件、C5 R3 用例、§4 流水、ADR 0077 options 表述、NFR 锚点） | `review-a3`/`review-l3p-r3` ADVISORY | 本提交已原地补齐（契约 C3/C5/§4、ADR 0077、NFR inert_this_wave/approval_branch）——关闭 |
| F-12 | 挂起轮的会话上下文未落盘：批准后的续轮里模型看不到上一轮用户原始请求（tmux 实测出现「no prior context visible」），只能靠翻工作区文件重建 | tmux 复验 `docs/release/evidence/wave65-approval-resume/02-approved-continuation.txt` | 挂起时把本轮 user 消息与已产出的 assistant/工具摘要落会话历史（或在 `turn:suspended` 载荷里携带 resume_context，续轮作为 host 上下文注入），并补 e2e |
