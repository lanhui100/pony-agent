# dev-team 编排「未生效」根因诊断（B1795535505312）

> 诊断日期：2026-11-24｜诊断方式：antigravity ACP 子智能体只读诊断 + 人工逐行核验
> 关联账本：pony-agent `B1795535505312`（转登自 `~/job_copilot` `B1795488531882`）
> 结论性质：**契约/可观测性缺陷，不是「团队没跑」**——队友确实跑了，是模型无法判断跑没跑。

## 1. 根因结论

1. **spawn 返回的是登记快照，不是运行态**：`register_teammate` 构造 `TeammateMember { status: Active }` 并作为 `SpawnTeammateResult` 返回；`spawn_teammate_in_workspace_with_provider_as` 在触发后台驱动后仍原样返回该快照，**不刷新 `member.status`** ⇒ 模型永远看到 `active`。
   证据：`crates/pony-agent-core/src/agent/orchestration.rs:893-899`（构造）、`:846-857`（触发驱动后返回旧快照）。
2. **成功、失败、中断三态在工具面全部塌缩成 `inactive`**：`drive_teammate_turn_in` 的收口（`:1776-1781`）、Err 补偿（`:1741`）、`interrupt_agent_as`（`:1188`）、两个中断检查点（`:1674`、`:1690`）写的都是 `Inactive` ⇒ `list_agents` 无法区分「跑完了 / 跑挂了 / 被打断」。
3. **`Failed` 与 `Provisioning` 是死值**：全仓 `TeammateStatus::Failed` / `::Provisioning` 零写入，只在 TUI 文案映射（`src/bin/tui/projection.rs:144,148`）与 `wait_agent_as` 的活跃判定（`:1615`）里被读 ⇒ 「失败」语义在状态机里根本不存在。
4. **`is_driving` 与 `wait_agent` 结果不进模型面**：`list_agents` schema 只有可选 `wait_ms`（`src/agent/tools/schema.rs:680-693`），返回值只有 `target/name/role/status/description`（`:73-79`）；工具层把 `wait_agent_as` 的 `WaitAgentResult`（含 `timed_out`/`no_progress`）用 `Ok(_) => {}` 丢弃（`src/agent/tools/orchestration.rs:371`）⇒ 模型唯一间接手段只剩 `drain_inbox(lead)` 的回执文本。
5. **spawn 丢弃 `delivery_state`**：`send_message_in_workspace` 已返回 `"queued"` vs `"driving"`（`:1022`、`:1130`），但 spawn 侧 `let _ =` 丢弃（`:847`）⇒ 模型无法知道「初始 prompt 是否真的开始跑了」；空 prompt 时更是静默跳过驱动（`:846`）。

## 2. 对原证据的逐条判定

| 原证据 | 判定 | 依据 |
|---|---|---|
| ② spawn 返回 active、随即 list_agents 为 inactive | **成立且可解释**：引擎在位 + prompt 非空时，spawn 返回旧快照 `active`，后台 turn 在宿主下一次 LLM RTT 之前收口为 `inactive` | `:846-857` + `:1776-1781` |
| ① 无 test-agent / 单点自证 | **不属本仓代码缺陷**：dev-team SKILL 的派生纪律是流程约束，仓内无任何机械门禁强制「测试证据须由非执行者产生」 | 见 §3 |
| ③ author 全为 `t <a@b>` | **成立**：仓内零机制给队友 turn 的提交打角色标识（无 `GIT_AUTHOR_*` / `Co-authored-by` 注入） | `rg` 全仓仅测试脚本设置 author |
| ④ 派生无回执 | **部分成立**：失败会投递 lead 收件箱（`:1743-1752`），但「中断」不投递、且 spawn 本身不回执 | `:1741` vs `:1674/1690` |
| ⑤ 共享工作树污染 | 交叉引用既有条目，不重复立项 | — |

## 3. 现有门禁的缺口

- `tests/wave61_team_activation.rs:969` 明确断言 `spawned.member.status == Active`，注释写「空 prompt 派生后成员必须处于 Active（未被自动驱动）」——**把缺陷行为冻结成了契约**，修 spawn 返回值会撞这条断言。
- `tests/teammate_e2e_communication_test.rs:65-66` 同样断言无引擎路径下成员为 `Active`（与 `:1763` 的降级口径一致）。
- 仓内**无任何用例**断言「成功 / 失败 / 中断三态在 `list_agents` 上可区分」，也无用例断言 spawn 回执含驱动状态。

## 4. 修复建议（按最小闭包 + 可机械验收排序）

| 优先级 | 改动点 | 机械验收 | 风险 |
|---|---|---|---|
| P1 | `spawn` 返回值增补运行态：`SpawnTeammateResult` 加 `delivery_state`（复用 `send_message_in_workspace` 的返回值），并把 `member.status` 改为**返回时回读**当前状态 | `cargo test -p pony-agent-core --lib agent::orchestration` + 新增单测断言「非空 prompt ⇒ delivery_state=driving 或 queued」 | 需同步 `wave61_team_activation.rs:969` 的冻结断言 |
| P1 | 失败/中断用真状态：Err 分支写 `Failed`（现在是 `Inactive`），中断写独立变体或 `last_error` 字段；`list_agents` 暴露 `is_driving` + `last_error` | 新增用例：注入必失败 Stub ⇒ 断言 `list_agents` 看到 `Failed` 且带 error 文本 | 改状态机需 ADR；TUI 映射已有 `Failed` 分支可复用 |
| P2 | `list_agents(wait_ms)` 不再丢弃 `WaitAgentResult`，把 `timed_out`/`no_progress` 回给模型 | 新增用例断言 wait 超时后返回值含 `timed_out=true` | 低（additive 字段） |
| P2 | 提交身份：队友 turn 内注入 `GIT_AUTHOR_NAME=<role>` 或统一 `Co-authored-by: <role>` | `git log --format='%an %s'` 可区分 | 需评估是否污染用户提交习惯 |
| P3 | 放行门禁机械化：CI 侧校验「PR 内存在非执行者产生的测试证据」 | 需新增 CI 步骤，非纯仓内改动 | 与 dev-team SKILL 强耦合，建议先改 SKILL 再机械化 |

## 5. 未确认项

- 原观测仓（job_copilot）的 `spawn_teammate` 调用是否真的走引擎在位路径（`subagent_engine()` 是否为 `Some`）——本仓无法复现该仓的运行时注入，故「路径 A vs 路径 B」只能二选一推断，不能定论。
- `interrupted round` 的具体触发点（引擎 120s turn 硬上限 vs 外部 interrupt_agent）在原证据中未给出 trace 定位。
