# Agent Note: 队友运行态对模型可见（状态三态可判读 + spawn 回执回读）

Status: implemented

## Problem

模型无法判定「队友到底跑没跑、跑完没跑完、跑挂了还是被打断」，导致 Lead 只能靠人催或自行猜：

1. `spawn_teammate` 返回的是**登记快照**而非运行态：`register_teammate` 构造 `TeammateMember { status: Active }` 后原样返回，触发后台驱动后不刷新，模型永远看到 `active`。
2. 成功 / 失败 / 被中断三种收口全部写 `Inactive`：`drive_teammate_turn_in` 的 Err 补偿、两个中断检查点、`interrupt_agent_as` 写的都是 `Inactive`，`TeammateStatus::Failed` 与 `Provisioning` 全仓零写入（死值）。
3. `is_driving` 与 `last_error` 不进模型面：`is_driving(target)` 只有 TUI 面板在调，`list_agents` 的返回值只有 target/name/role/status/description。
4. `list_agents(wait_ms)` 丢弃等待结果：工具层把 `wait_agent_as` 的 `WaitAgentResult`（含 `timed_out` / `no_progress`）用 `Ok(_) => {}` 吞掉，模型无法得知「等到了边沿」还是「超时了」。
5. `spawn` 丢弃 `delivery_state`：`send_message_in_workspace` 已返回 `queued` / `driving`，spawn 侧用 `let _ =` 丢弃，「初始 prompt 是否真的开始跑」不可见。

证据与逐条判定见 `.dev-team/diagnosis/dev-team-orchestration-status-gap-20261124.md`；账本条目 `backlog:B1795535505312`（转登自 `~/job_copilot` `backlog:B1795488531882`）。

## Decision

把「队友运行态」收敛为**一个可判读的状态面**，全部 additive（不改既有字段语义、不改 input schema）：

- `TeammateStatus` 增 `Interrupted` 变体：`running`（有 turn 在跑）/ `active`（空闲，有待驱动消息或刚派生）/ `inactive`（空闲，上轮成功）/ `failed`（上轮驱动返回 Err）/ `interrupted`（被 `interrupt_agent` 在下一 drain 边界前停下）。`provisioning` 保留但明确「今天不发出」（无异步创建路径）。
- `TeammateMember` 增两个 additive 字段：`is_driving: bool`（读时快照，来自 `DRIVING` 单飞标记）与 `last_error: Option<String>`（上轮失败的错误文本，成功后清空）。
- `drive_teammate_turn_in` 收口写真实状态：成功 → `inactive` 且清 `last_error`；引擎 Err → `failed` + 记录错误文本（消息仍按 C10.2 原序回滚，回执仍投 lead）；两个中断检查点与 `interrupt_agent_as` → `interrupted`。
- `spawn_teammate_in_workspace_with_provider_as` 在触发驱动后**回读**名册返回成员当前状态，并在 `SpawnTeammateResult` 增 `delivery_state`（`driving` / `queued` / `not_started`）。
- `list_agents` 工具层把 `WaitAgentResult` 随名册一起返回（`wait.timed_out` / `wait.no_progress`），不再丢弃。
- 团队策略段同步改写「Member status」条款，冻结快照 `tests/fixtures/team_policy_prompt.txt` 同提交更新；`Interrupted` 加入 TUI 文案映射与前端 `TeammateStatus` 联合类型。
- 冻结用例里把缺陷行为写成契约的断言按新语义更新（`wave61_team_activation` C7c 失败后 `failed`、C7e 空闲中断 `interrupted`、`pa110`/`team_link_fixes` 的中断 `new_status`），并新增 `tests/team_state_observability_test.rs` 覆盖三态可判读、`is_driving` 实时性与 spawn 回执。

不做的部分（本轮显式排除，账本继续挂账）：队友 turn 的 git 作者身份注入（`GIT_AUTHOR_*` / `Co-authored-by`）与「PR 内必须存在非执行者测试证据」的 CI 机械门禁——前者改变用户提交习惯需单独裁决，后者与 dev-team SKILL 强耦合，属流程面而非本仓运行时代码。

## Alternatives considered

- **只加 `last_error` 不加 `Interrupted`**：失败可与成功区分，但「被打断」与「跑完了」仍同态，Lead 会把中断读成完成；代价是 enum 多一个变体（TUI/前端各一处映射），收益是三态真的可判读——选本方案。
- **把中断/失败都压进 `last_error` 文本**：状态枚举不变、改动最小，但文本判定要模型做字符串嗅探，且 `list_agents` 的 `status` 继续撒谎；违背「机器可判读优先于散文」。
- **spawn 同步驱动首个 turn 再返回（把状态做成确定终态）**：模型拿到的必是 `inactive`，但派生被拉长为一个完整 turn 的墙钟（实测数十秒），且与 wave-61 C7 的「派生即返回、驱动异步」契约冲突——拒绝。
- **新增独立 `team_status` 工具**：模型可见工具面是冻结面（46 项 + schema hash golden），新增工具需动 8 处清单与快照，收益只是字段重排——拒绝，改为在既有 `list_agents` 上 additive 扩容。
- **删除 `Provisioning` 死值**：属 breaking（serde 联合类型 + 前端 + 快照），且将来异步创建仍可能需要它；保留并在提示词里明说「今天不发出」，不假装有能力。

## Consequences

- 模型可用 `list_agents` 的 `status` + `is_driving` + `last_error` 判定「在跑 / 跑完 / 跑挂 / 被打断」，`spawn` 回执可用 `delivery_state` 判定「初始 prompt 是否真的开始跑」。
- `wait_agent_as` 的边沿等待语义不变；`active` 仍表示「空闲但有待驱动消息」，与 `send_message` 的入队迁移同源。
- 冻结断言发生三处语义升级（`inactive → failed` / `inactive → interrupted`），均在本提交内同步，`tests/fixtures/team_policy_prompt.txt` 与前端联合类型一并更新。
- 仍未解决：队友提交的作者身份不可审计（证据③）与非执行者测试证据的 CI 门禁（验收①）——留在 `backlog:B1795535505312` 的后续项。
