# Agent Note: r1-dim6 上报链路自检修复（F1 工作区绑定 / F2 trace 级 release / F3 结局单值）

Status: implemented

## Context

ADR 0101 的 trace 审核 loop 首轮（r1-dim6，维度 6 上报链路自检）在 baseline 臂（`c1f47e9d`）
9 会话重放上实测三条断言全红：

- **A1 重放隔离 9/9 失配**：会话 `Environment: cwd=` 恒为注册表 default（`/home/dm/pony_agent`），
  而非宿主 `-w` 指向的 scratch worktree —— 模型看到的目录与审计声明不一致，corpus 任务
  实际跑在错误目录上。
- **A2 trace 级 release 9/9 为空**：按发布线筛选 trace 取不到数（span 属性里有
  `langfuse.release=0.1.158`，trace 级为 `null`）。
- **A3 结局非单值 8/9 冲突**：同一轮同时 `agent.turn.phase=completed` 与
  `agent.turn.outcome=error`，另带 `tool_failures>0` + `degraded=true` —— 四个信号三种判定，
  消费方无法用单键判结局。

提案 `docs/audit/rounds/r1-dim6/proposals/r1-dim6.pa01a1ba70.md` 经人工批准（frontmatter
`status: approved`），写域限定为四个文件。本 note 记录三处修复的实现决策。

## Decision

1. **F1（blocker）：CLI 复用 `ensure_workspace_for_root` 盖章 `workspace_id`**
   `run_single_turn` 此前恒传 `workspace_id: None` ⇒ `resolve_session_workspace_root` 回落
   注册表 `default`，宿主 `-w` 被静默覆盖。改为在 `main` 里 `bind_workspace_id`（对
   `args.workspace` 调 `control_plane.ensure_workspace_for_root`，取返回 `record.id`）并把该 id
   贯穿单发与 REPL 两条路径 —— 与 TUI 启动路径（`tui.rs:6995-6999`）完全同构，**零新增契约**。
   登记失败 ⇒ 返回 `None` 并打印告警，保持旧行为（不 panic、不静默换根）。
2. **F2（major）：resource 侧补 `langfuse.release`**
   Langfuse 的 `OtelIngestionProcessor.createTraceEvent` 取
   `spanAttributes[RELEASE] ?? resourceAttributes[RELEASE]`；「浅 trace」（子 span 先到触发）不含
   该 span 属性，后到的浅 trace 会把 `trace.release` 覆写为 `null`。`resolve_resource_attributes`
   增推 `(langfuse.release, service_version)` —— 与 `service.version` 同源同值（ADR 0055 已定该语义），
   两条路径都能取到。按 ADR 0068 登记为 **additive**（纯新增 resource 键，旧消费方零影响）。
3. **F3（major）：`agent.turn.outcome` 新增 `degraded` 取值**
   `phase == "completed"` 且工具失败计数 > 0 ⇒ 写 `degraded`（与既有 `agent.turn.degraded=true`
   同源同域）；其余失败路径逐字仍写 `error`；全成功轮次仍完全不输出该键。根 span `Status` 仍置
   error，Tempo 侧按根状态告警能力不变。实现上把失败计数在 `self.phase.take()` **之前**快照，
   与既有 degraded 判定共用同一只读口径（`take_active_turn_tool_failures` 不清零）。
   黑盒契约 `tests/otel_resource_turn_status_test.rs` 的 PA154-T1 期望值随之由 `error` 改为
   `degraded`；该文件声明「实现方不得修改，须 Lead 批准」，本变更随提案批准一并授权
   （见提案 frontmatter `approval_note`）。断言辅助函数改为按用例传期望值，强度不降：
   键仍须恰好一次、值仍须精确匹配、仍禁 `success`。

## Alternatives considered

- **F1 改宿主侧 `cd` 而不盖章**：重放脚本已经 `cd` 进 worktree 再传 `-w`，仍失败 —— 因为
  会话级归属优先于进程 cwd。改 cwd 是绕过症状，且第二个会话会再次漂移。落选。
- **F1 让 `resolve_session_workspace_root` 优先用宿主 root**：会推翻 PA-096 的「会话级归属
  优先 + 死 id fail-closed」边界，属跨模块契约变更，超出本维度写域。落选。
- **F2 在 span 侧补写「浅 trace 也带 release」**：需要在每个子 span 上重复该属性（属性膨胀），
  且无法覆盖「子 span 先于根 span 到达」的时序竞态。resource 侧一次补齐更小更稳。落选。
- **F3 保留 `error` 值、只靠 `degraded=true` 判结局**：消费方仍需读两个键做合取，且
  `outcome=error` 在「轮次正常收尾」时是错误陈述（会污染失败率看板）。落选。
- **F3 新增独立键 `agent.turn.outcome.degraded`**：与 `agent.turn.degraded` 形成第三份真值源，
  违反「每个事实一个家」。落选。
- **本轮顺带修「CLI 退出前 OTel 排空超时导致根 span 丢失」（3/9 会话）**：属新观测缺口，
  阈值已冻结，改断言须新提案；本轮只作为 residual finding 记入回执 023，不越界。

## Consequences

- 维度 6 在两条独立候选臂上判 PASS（`verdict.sh` exit=0，dim6 入棘轮），基线臂同断言集判红 ——
  前后对照成立。
- 顺带收益：CLI 的 `-w` 从此真正生效，重放/无头调用的目录语义与声明一致（此前所有
  headless 审计证据都存在这一污染）。
- 已知残留（记入 `docs/audit/rounds/r1-dim6/receipts/023-verify-clean-arm.pa01a1ba70.json`）：
  clean 臂 3/9 会话缺 `agent.turn` 根 span，伴生 `otel::shutdown ... Timeout(1s)`；
  属维度 6 的观测面缺口，需新断言（新提案）覆盖，不在本轮改阈值。
- 契约变更登记：`docs/observability.yaml` 追加归属说明（additive，四维归属不变）。
