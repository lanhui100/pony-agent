# Agent Team 实现审核报告（对标 deepseek-harness）

- 审核对象：`crates/pony-agent-core/src/agent/orchestration.rs`、`tools/orchestration.rs`、`subagent_runtime.rs`、`agent/team_history.rs`、`src/stores/team.ts`、`src-tauri/src/lib.rs` 团队命令、`system_prompt_template.rs` 团队策略段
- 对标基线：`/home/dm/deepseek-harness` @ `de5d733a18`（`packages/experimental/agent-team`、`tool-agent-team`、`packages/subagent`）
- 方法：静态通读 + 交叉引用（工具层→内核层→引擎层→前端），未改动任何代码
- 结论定级：**P0 5 项、P1 9 项、P2 6 项**；核心问题是「提示词/决策记录承诺的能力」与「内核实现」大面积不一致

## 1. 总体结论

| # | 结论 | 严重度 |
| :-- | :-- | :-- |
| 1 | `wait_agent` 是空实现：不等待、不观察任何边沿，恒返回 `timed_out=false`；模型据此误判"有进展" | P0 |
| 2 | 队友 turn 之间**没有会话延续**：每次驱动新建 `subagent-<uuid>` 会话，队友跨轮失忆 | P0 |
| 3 | 共享任务看板只有 CAS，没有授权、状态机、就绪语义与依赖校验；任何成员可抢/改/删任意任务 | P0 |
| 4 | 队友回执只进 lead 收件箱，且**永不唤醒/消费**；系统提示也未告知 lead 使用 `drain_inbox` | P0 |
| 5 | 团队状态是进程级全局 `static Mutex`：无会话隔离、无持久化、重启即失、测试互相污染 | P1（连锁 P0 影响） |
| 6 | ADR/系统提示与实现漂移：宣称 Tokio 边沿唤醒、前端事件订阅、写域重叠告警、依赖阻塞，实现均不存在 | P1（治理） |

一句话：**当前实现是一个可演示的"消息队列 + 内存任务表"，而不是可托付的 Agent Team 内核**；与 DSH 的差距不在功能数量，而在「真值源、权限、会话延续、可恢复性」四个地基。

## 2. 对标矩阵

| 维度 | deepseek-harness | pony-agent 现状 | 判定 |
| :-- | :-- | :-- | :-- |
| 团队身份 | `TeamId = root SessionId`，隐式一团队一根会话，成员是 root 的直接子会话 | 无团队概念；`TEAMMATES` 进程级单例，`lead` 为硬编码伪成员 | ✗ |
| 状态真值 | Lead Session 事件日志 + `agentTeam` projection（事件溯源、schema 版本化、失败传播） | `static Mutex<Option<HashMap>>` ×4（`orchestration.rs:345/351/353`） | ✗ |
| 成员生命周期 | `provisioning → active \| failed`，含崩溃恢复对账（`roster.ts:reconcileProvisioning`） | 枚举有 5 态，但 `Provisioning`/`Failed` **从未被赋值**（仅 TUI 映射与 wait 判断引用） | ✗ |
| 会话延续 | continuable resident child（`subagent/manager.ts`），跨消息保留上下文 | 每轮 `SubagentRunRequest::new` → `session_id` 为空 → 新 uuid（`subagent_runtime.rs:427/484`） | ✗ |
| 消息投递 | `steer` 注入目标会话；lead 走 `root.steer`；拒绝自发；限制字节；加 `Team message from X:` 前缀 | 入队 `VecDeque`；lead 不驱动；允许自发；无字节上限；无 sender 标注 | ✗ |
| 唤醒机制 | `TeamActivity` waiter + commit 通知，`waitForChange` 有界 10s~1h | `wait_agent` 空实现（`orchestration.rs:1164-1191`）；无 Notify/Condvar/broadcast | ✗ |
| 任务 CAS | `revision` + 每 Lead 串行事务（`journal.transact`） | `revision` 比较（`1117`），持锁串行，CAS 语义本身可用 | △ |
| 任务授权 | owner 或 lead；reassign 仅 lead；interrupt 仅 lead | 无任何调用者校验；claim 直接覆盖 owner；edit 可改 owner；任何成员可 interrupt 任意成员 | ✗ |
| 任务状态机 | `claim` 需 pending+ready+无主；`complete` 需 in_progress；`release/reopen/reassign/delete` 各自校验；删除拒绝仍有依赖者 | 无状态前置校验；deleted 仍可被 claim/complete 复活；删除不检查依赖者 | ✗ |
| 依赖图 | `assertTaskGraphCandidate`：missing / duplicate / cycle 全量校验 | `blocked_by` 原样存储，无校验、无 ready 计算 | ✗ |
| 写域 | 归一化 + 路径安全 + 运行期重叠告警（`task-view.ts`） | 原样存储，无归一化、无重叠告警 | ✗ |
| 容量与预算 | `maxMembers` / `maxTasks` / `maxMessageBytes` / `disposalTimeoutMs` | 无任何上限（成员、任务、消息、subject/description 长度均无） | ✗ |
| 工具面 | 按 Agent scope 安装，仅团队成员可见；`wait_agent`/`team_task_get` 齐备 | 全局工具面；teammate 也拿到 `spawn_teammate`/`interrupt_agent`；缺 `team_task_get` | ✗ |
| 关闭语义 | admission cutoff + settle + 超时 + `AggregateError` + 停止子分支 | 无 dispose；detached 线程无回收 | ✗ |
| 前端可视化 | projection 事件推送，实时 | Tauri 命令拉取，仅 `onMounted` 一次（`TeamRosterPanel.vue:38-42`） | ✗ |
| 错误契约 | `TeamError` + 稳定 code 枚举（如 `TEAM_TASK_STALE_REVISION`） | 自由字符串（`send_failed`/`spawn_failed`…），无契约枚举 | △ |
| 持久化/历史 | Lead Session 日志即真值，重开可恢复 | 内存态；`team_history.rs` 10 个 `todo!()` 空桩且无生产调用点 | ✗ |

## 3. P0 缺陷（阻断，建议优先修复）

### P0-1 `wait_agent` 空实现，向模型输出假信号

- 证据：`orchestration.rs:1164-1191`；`1186: let _timeout = args.timeout_ms.unwrap_or(30_000);`、`1187: // 模拟等待返回`，随后直接 `Ok(WaitAgentResult { timed_out: false, no_progress: None })`。
- 影响：模型被明确要求「wait → re-list 再判断」，却拿到「未超时 + 无进展信息」的空结果；这会诱发两种坏行为：立即宣布完成，或空转重试。比能力缺失更危险的是**假信号**。
- DSH 对照：`tool-agent-team/src/index.ts:250-282`，先用 `listMembers` 做 `no-active-peer` 快速失败（且与 waiter 注册保持同步区间），否则 `waitForChange` 真等待；`TeamActivity` 在 commit 时逐个释放 waiter。
- 修复建议（择一，不可保留现状）：
  1. 真实现：`Condvar`/`tokio::sync::Notify` + 版本号（team 状态 + 成员状态任一变更即 bump），`wait_agent` 阻塞到 bump 或超时（10s~1h），超时返回 `timed_out=true`；`no-active-peer` 分支保留。
  2. 若短期不做：**下线该工具**并从工具面/提示词中移除（fail-closed），绝不允许返回伪造的成功。

### P0-2 队友无会话延续，跨轮失忆

- 证据：`drive_teammate_turn_in`（`orchestration.rs:1207`）调用 `engine.run_turn_with_selection_and_parent(...)`，不传 `session_id`；`subagent_runtime.rs:427` 在 `session_id=None` 时 `format!("subagent-{}", Uuid::new_v4())`；`TEAMMATE_SESSIONS` 仅登记"最近一次成功 turn 的会话 id"供 UI 打开（`orchestration.rs:405-418`），从不回灌给下一轮。
- 影响：`spawn_teammate` 的语义（"a longer independent run"）不成立；第二轮消息只能靠 lead 把上下文重述一遍，token 成本翻倍且质量差。多轮协作（reviewer 复审、executor 返工）会系统性退化。
- DSH 对照：teammate 是 `mode: 'continuable'` 的常驻子代理，消息经 `steerHostSubagentPrompt` 进入同一会话。
- 修复建议：派生时分配并持久化 `teammate_session_id`，驱动时以 `SubagentRunRequest.session_id = Some(id)` 复用；首次 turn 结束后 `set_teammate_session` 生成的 id 必须与请求 id 一致（当前 `outcome.session_id` 与请求 id 无绑定关系）。

### P0-3 任务看板无授权、无状态机、无依赖语义

- 证据：`orchestration.rs:1111-1158`。`Claim` 无条件写 `status=in_progress` 且 `owner=args.owner`；`Complete/Release/Delete` 不校验 owner/状态；`Edit` 可直接改 `owner`；未拒绝 `deleted` 任务被重新 `claim`。工具层 `tools/orchestration.rs:494` 直通内核，无 caller 身份。
- 影响：
  - 两个队友可同时"认领"同一任务且后者静默覆盖 owner，CAS 只防同 revision 的并发，不防语义抢占；
  - 任一队友可完成/删除他人的在途任务，导致"任务完成了但代码没写"；
  - `blocked_by` 只是装饰：被阻塞任务可被认领，`ready` 概念不存在；
  - `delete` 不检查后继任务，留下悬挂依赖。
- DSH 对照：`task-board.ts:104-205` 的 `authorizeOwner`、逐 action 前置状态校验、`reassign` 仅 lead、`delete` 检查 `TEAM_TASK_HAS_DEPENDENTS`、`claim` 需要 `taskReady`；`task-graph.ts` 校验 missing/duplicate/cycle；`task-view.ts` 输出 `ready` 与 `writeScopeWarnings`。
- 修复建议：内核层（不是工具层）新增 `CallerContext { role, member_id }`，把授权/状态机/依赖校验做成**唯一收口**；工具层与 Tauri 命令都只能走这个收口（当前 `src-tauri/src/lib.rs:490` 的 `team_claim_task` 直接传任意 `owner`，属同一缺陷面）。

### P0-4 队友回执成为"死信"

- 证据：`drive_teammate_turn_in` 结束后向 `lead` 入队（`orchestration.rs:1320-1330`）；`send_message_in_workspace` 仅在 `drive_target != LEAD_TARGET` 时驱动（`orchestration.rs:857`），lead 目标永不唤醒；`TEAM_POLICY`（`system_prompt_template.rs:52-62`）未提及 `drain_inbox`，模型没有理由主动取信。
- 影响：队友干完活，lead 看不到结果；同时 lead 的"收件箱"里堆着历史回执，无读即无界增长。团队协作闭环断裂。
- DSH 对照：`sendAdmitted` 对 root 用 `root.steer(input)`，把消息注入 lead 当前/下一 turn；对 teammate 用 `steerHostSubagentPrompt`。
- 修复建议：
  1. 首选：队友回执走 lead 的 steer 通道（进入下一轮 prompt），而不是队列；
  2. 次选：保留队列，但在 `TEAM_POLICY` 中显式声明"每轮结束前必须 `drain_inbox(target="lead")`"，并在 `wait_agent` 超时时返回待读条数；
  3. 无论哪种，都要给 lead 收件箱加上限与丢弃策略（当前无界）。

### P0-5 权限边界缺失：teammate 可取到全套团队工具

- 证据：`tools/orchestration.rs:270` 的 `execute_spawn_teammate` 不校验调用者角色；`382` 的 `execute_interrupt_agent` 同样；teammate 由 `AgentRuntimeBuilder::new()` 默认工具面驱动（`subagent_runtime.rs:462`），无裁剪；`interrupt_agent` 可指向 `lead`。
- 影响：teammate 可递归派生新队友（无深度/数量上限），可中断 lead 或兄弟成员，可任意改任务板。DSH 是**双层防护**：工具按 Agent scope 安装（`tool-agent-team/src/index.ts:170-320` 的 `install`），且服务端 `roster.spawn` 强制 `role === 'lead'`、`interrupt` 强制 lead。
- 修复建议：两层都补——(a) teammate runtime 的工具面剔除 `spawn_teammate`/`interrupt_agent`；(b) 内核 `register_teammate`/`interrupt_agent`/任务流转接收 caller 并做角色校验。

## 4. P1 缺陷

| ID | 缺陷 | 证据 | 影响 |
| :-- | :-- | :-- | :-- |
| P1-1 | spawn 失败静默：成员已登记、初始 prompt 投递失败被丢弃 | `orchestration.rs:746` `let _ = send_message_in_workspace(...)`；无 provisioning→failed 回滚 | 僵尸成员，lead 以为队友在跑 |
| P1-2 | 无成员/任务/消息容量上限与文本长度校验 | 全文件无 `MAX_`/`max_` 常量；`team_task_create`（`1080`）直接入库 | 内存与 prompt 膨胀；恶意/失控成员可放大 |
| P1-3 | 无持久化与多会话隔离 | 全局 static（`345/351/353`），进程退出即失 | 桌面端重启后看板清空；多窗口/多会话共享一份脏状态 |
| P1-4 | 前端只在挂载时拉一次，无事件订阅/轮询 | `TeamRosterPanel.vue:38-42`；`src-tauri/src/lib.rs` 无 team emit；注释残留「B098/B105 UI 状态恢复尚未实现」 | 面板状态与真实团队脱节，用户误判 |
| P1-5 | `team_history.rs` 为 10 处 `todo!()` 空桩且无生产调用点 | `team_history.rs:76-175`；全仓 grep 无生产引用 | 死模块占位；注释显示曾为过 `-D warnings` 改形参，属治理债 |
| P1-6 | `drain_inbox` 的 caller 反查在队友场景失效 | `tools/orchestration.rs:428-432` 用 `teammate_target_for_session` 反查，而该映射只在**成功 turn 之后**写入（`orchestration.rs:1272`） | 队友首轮/失败轮调用 `drain_inbox` 被误判为 lead 而拒绝 |
| P1-7 | `context: fresh\|fork` 为死参数 | `SpawnTeammateArgs.context` 在 `register_teammate`（`769`）中未被使用；`SubagentRunRequest.fork_context` 在队友驱动路径恒为 `false` | 模型按提示传 `fork` 得到 `fresh` 行为，提示词欺骗 |
| P1-8 | 无关闭/收口：detached 驱动线程无回收 | `send_message_in_workspace` 内 `std::thread::spawn`（`868`）；无 dispose 入口 | 退出/重载时线程与许可悬挂；DSH 有 `disposeRuntime` + `settle` + `AggregateError` |
| P1-9 | Mutex 滥用 `.unwrap()`，中毒即 panic | `orchestration.rs` 多处 `lock().unwrap()`（`1030/1069/1080/1111/1164/1331`） | 任一处 panic 后所有团队工具永久 panic，无降级 |

## 5. P2 缺陷

1. 缺 `team_task_get` 工具（`tool-agent-team` 有 `team_task_get`；pony 仅 create/update/list）。
2. 消息无 sender 标注：`combined_prompt` 只拼 `- {content}`（`orchestration.rs:1243-1246`），模型无法区分来源；DSH 加 `Team message from {name}:`。
3. `send_message` 工具描述「紧急插队 (steer)」与实现不符（`schema.rs:691`）：`Steer` 仅 `push_front`，不打断在途 turn。
4. 错误返回为自由字符串，无稳定 code 枚举，测试无法断言契约（DSH `TEAM_*` 常量）。
5. 团队相关测试无 `wait_agent`、越权、阻塞、依赖环、容量上限覆盖（`tests/` 仅断言 stale revision）。
6. `docs/observability.yaml` 未见 agent team 域归属条目（全仓 grep `team` 仅命中 `/teams` 别名与历史记录），驱动线程/LLM 调用/后台任务的可观测归属需按门禁复核。

## 6. 决策记录与提示词的一致性核查（治理重点）

| 承诺位置 | 承诺内容 | 实现事实 |
| :-- | :-- | :-- |
| ADR `2026-11-11-agent-team-collaboration-and-task-board.md:16` | 「基于 Tokio Notify / 广播通道，消息入队与任务更新亚毫秒级边沿唤醒」 | 无任何 `Notify`/`broadcast`/`tokio::sync` 引用；`wait_agent` 空实现 |
| ADR `:24` | 「前端通过 Pinia `useTeamStore` 订阅团队事件并同步数据」 | `onMounted` 一次性 `invoke` 拉取，无事件订阅、无轮询 |
| ADR `:20-22` | 「CAS 乐观锁 + 写域声明 + 依赖阻塞控制」 | CAS 存在；写域仅存储、无重叠告警；依赖无就绪/环校验 |
| `TEAM_POLICY_TEMPLATE`（`system_prompt_template.rs:52-62`） | 「send_message：running 目标在最近 step 边界收到；inactive 目标会被启动/恢复」「inactive/provisioning/failed 三态语义」 | lead 目标永不启动；`Provisioning/Failed` 从不赋值；steer 不打断在途 turn |
| 同上 | 「write-scope overlap is advisory」「use task dependencies when work must be ordered」 | 两者均无实现 |
| 同上 | 「Re-list with list_agents and team_task_list after each action」 | `wait_agent` 空实现使该工作流失去事件来源（只剩轮询） |

结论：这不是"实现未完成"，而是**文档先行、实现未跟上却已标记 implemented**。按仓库宪法第 1、2 条，应把该 ADR 状态与承诺一并修正，或补齐实现后再声明。

## 7. 优化建议（按里程碑）

### M1「停止说谎」（1-2 天，不改架构）

1. `wait_agent` 二选一：真实现或下线（P0-1）。
2. 修正 `TEAM_POLICY_TEMPLATE`：删除未实现的承诺（写域重叠、依赖阻塞、provisioning/failed、steer 边界语义），补齐 `drain_inbox` 工作流说明（P0-4/治理）。
3. 修正 `send_message` 工具描述，去掉"紧急插队"。
4. `team_history.rs` 空桩：删除模块或标注 `#[allow(dead_code)]` + 明确 owner，不留在生产 crate 里。
5. ADR 加「实现现状/差距」章节并降级为 `proposed`，或新建一条"团队内核 v2"决策记录。

### M2「补齐地基」（3-7 天）

1. **状态真值迁移**：`TeamStore` 从全局 static 改为按根会话注入的实例（`AgentRuntime` 持有），提供 `team.jsonl` 或 SQLite 事件表持久化；启动时重放。
2. **真边沿唤醒**：`TeamActivity`（`Condvar`/`Notify` + 版本号），成员状态、任务流转、消息入队三处 bump；`wait_agent` 接上。
3. **权限收口**：`CallerContext { role, member_id }` 贯穿内核；spawn/interrupt/reassign 仅 lead；teammate 工具面裁剪。
4. **任务状态机与依赖图**：补齐 claim/complete/release/reopen/reassign/delete 前置校验；`ready` 计算；missing/duplicate/cycle 校验；`writeScope` 归一化（拒绝绝对路径与 `..`）+ 重叠告警。
5. **会话延续**：teammate 固定 `session_id` 并复用。
6. **容量上限**：`max_members=16`、`max_tasks=256`、`max_message_bytes=64KiB`、`subject<=200`、`description<=16KiB`。
7. **回执直达**：队友结果 steer 进 lead 下一轮，或由 lead turn 收口强制 drain。
8. **关闭语义**：提供 `dispose`，等待在途驱动、超时上报、聚合错误。

### M3「工程化」（1-2 周）

1. 事件/投影：团队事件进会话日志，前端订阅（Tauri event 或 SSE），替代一次性拉取。
2. 错误契约：`TeamError` + 稳定 code 枚举，工具返回 `{ code, message, hint }`。
3. 测试矩阵：越权、阻塞、依赖环、容量、并发认领、崩溃恢复（provisioning 对账）、`wait_agent` 边沿（含"无活跃同伴快速返回"）。
4. 可观测性：按 `docs/observability.yaml` 补齐团队域归属（驱动线程、LLM 调用、后台任务、UI 交互收口点）。
5. 前端：面板状态订阅 + `dispose` 清理；`TeamRosterPanel.vue` 残留 TODO 注释清理。

## 8. 建议验收命令（M1 即可落地）

```bash
# 1) 假实现必须消失：wait_agent 内不得出现 "模拟等待" 且必须使用 timeout
grep -rn "模拟等待返回" crates/ || echo "OK: no fake wait"
# 2) 边沿唤醒必须存在（M2）
grep -rn "Notify\|Condvar\|broadcast" crates/pony-agent-core/src/agent/orchestration.rs
# 3) 提示词不得承诺未实现能力
grep -rn "overlap is advisory\|provisioning means\|failed means" crates/pony-agent-core/src/agent/system_prompt_template.rs
# 4) 空桩不得留在生产 crate
grep -rn "todo!" crates/pony-agent-core/src/agent/team_history.rs
# 5) 团队测试（当前无 wait/越权覆盖）
cargo test -p pony-agent-core --test pa110_teams_acceptance --test team_drain_scope_test --test wave61_team_activation
```

## 9. DSH 可直接借鉴的机制清单

1. `TeamJournal.transact`：按 Lead 串行化"读-校验-追加"，把 CAS 检查放进事务而不是散落在 handler。
2. `TeamRuntimeLifecycle`：admission cutoff + `settle` + `disposalTimeoutMs` + `AggregateError`。
3. `reconcileProvisioning`：用子会话自身日志判定 `provisioning` 的真实终态，重启后自愈。
4. `TeamActivity.wait` + `no-active-peer` 快返：等待语义与"不可能有进展"分离。
5. `task-graph.assertTaskGraphCandidate` + `taskReady` + `writeScopeWarnings`：把"能不能做"与"谁在做"分离。
6. 双层权限（scoped tool install + 服务端 role 校验）。
7. `TeamError` 稳定 code + schema 版本化投影（`stateVersion` / `version: 2`）。
