# Wave-61 交付说明：Agent Team 激活（工具面 × 宿主接线 × 策略提示）

> 最终状态：契约 FROZEN **v1.5**；模型可见工具面 **38 项**；红相 `52a7a21` → 实施 `60e7cb8`/`922e960` → 收口 `ed6d35e`(+ v1.5 驱动循环修正)。

- 骨架分级：B（跨模块：tools / runtime / context / control_plane / provider 测试面）
- 红相锚定：`52a7a21` `test(wave-61-team): freeze red-phase acceptance tests (C1-C5) + migrate 3 frozen contracts`
- 绿相实施：`60e7cb8` `feat(wave-61-team): expose team tools, wire host subagent engine, inject team policy prompt`
- 契约：`.dev-team/contracts/wave-61-team-activation.md`（FROZEN v1.1）

## 1. 缺陷因果（诊断）

trace 证据（`~/.local/share/PonyAgent/sessions.db` → `build_context_observations`）：
`session-4bdcc39a / turn-0c9aebe0` 的 `toolDefinitionsText` 只有 28 项，**不含** subagent / workflow /
spawn_teammate / list_agents / send_message / interrupt_agent / drain_inbox / team_task_*；更早的
`turn-62edc32c` 里这些工具都在。→ 不是模型"忘了用"，而是工具面被收窄且运行时未接线。

三层根因（全部有代码真值源）：

| # | 根因 | 真值源 |
|---|---|---|
| R1 | 10 项团队工具 `exposure` 全落 `_ => Internal`，模型面只保留 `ModelVisible` | `tools.rs:7341`（旧）、`tool_runtime.rs:161-174`、`tools.rs:625` |
| R2 | `subagent_runtime_wired()` = `subagent_engine().is_some()`，而 `set_subagent_engine` **生产代码零调用**（仅测试）→ 即使可见也 fail-closed | `tools.rs:152`、`subagent_runtime.rs:37-45`、10 处 `capability_unavailable_result` |
| R3 | 系统提示无任何「Agent Team 可用」策略段，模型不知有团队能力 | `context.rs:15` BASE_SYSTEM_PROMPT |

参考实现对齐（deepseek-harness）：团队工具常驻模型面（`packages/experimental/tool-agent-team/src/index.ts:192-400`）、
策略段与工具注册**同源门控**（同文件 `:173-183`）、策略段落在稳定前缀层（`packages/core/system-prompt/src/index.ts:135`
`SECTION_ORDERS.TEAM_POLICY=600`）、`subagent`/`workflow` 门控在执行期而非隐藏 schema。

## 2. 实施内容

| 契约 | 内容 | 落点 |
|---|---|---|
| C1 | 9 项团队工具 `ModelVisible`（模型面 29 → **38** 项；`workflow` 因桩退回 Internal，见 §6） | `tools.rs` `tool_exposure_for_name` |
| C1.6 | `model_visible_tool_name_opt` 小写原名透传（否则 telemetry 把工具记成 `Run`，污染 trace） | `tools.rs:7282-7295` |
| C2 | `AgentRuntimeBuilder::build()` 内 `OnceLock` 幂等注册 `HostTurnEngine`（tauri / harness / tui / cli 四条宿主零改动获益） | `runtime/builder.rs:79-104` + `:135-152` |
| C3 | 静态团队策略段作为 `BASE_SYSTEM_PROMPT` 之后的第二条 `System` 消息，门控 `subagent_runtime_wired()` | `context.rs:50-88`、`:449-455`、`:548-566`、`:613` |
| C3.7 | 检索代理阈值补偿，避免策略段让 `context_uses_retrieval` 恒真 | `context.rs:84-88`、`runtime/trace_timeline.rs:194-201`、`control_plane/mod.rs:3227-3231` |
| C4 | 三处冻结契约迁移（只收紧不放宽）+ src 内 6 处派生断言同步 | `tests/tool_surface_wave20_contract.rs`、`tests/wave37_tool_surface.rs`、`tests/wave25_capability_honesty_test.rs`、`tools.rs:6623/6645/6524/6572/6821`、`provider/mod.rs` 4 hunk |

## 3. 机器门禁收据（物理凭证）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 红相 | `cargo test -p pony-agent-core --test wave61_team_activation`（红相期） | exit 101；5 红 2 绿（`.dev-team/red-phase-wave61-team.json`） |
| 绿相验收 | 同命令 | exit 0；7/7 通过 |
| 关联套件 | 8 个测试二进制（wave61/wave20/wave37/wave25/orchestration/pa110/team_task_order/teammate_e2e） | exit 0；33/33（`.dev-team/green-phase-wave61-team.json`） |
| lib 单测 | `cargo test -p pony-agent-core --lib -q` | exit 0；1210 passed / 0 failed |
| 类型检查 | `cargo check -p pony-agent-core --all-targets` | exit 0（仅既有 `stop_contract_wave60` dead_code warning） |
| 回归基线 | 红相前基线 | exit 0；22/22（`.dev-team/regression-baseline-wave61-team.json`） |
| 可观测性 | `audit-diff.sh --base 52a7a21` / `check-declaration.sh .` | exit 0 / exit 0 |
| 契约 lint | `dev-nfr-lint.py .dev-team/nfr-baseline.json` | exit 0（严格门 `nfr-lint.py` 为 HEAD 既有 6 错，见契约 §6.1） |

## 4. 不可逆面登记

- 无 DB schema / 序列化编号 / Loki label 变更。
- 工具面 29 → 38 属 `additive`；**反向不可逆点**：团队工具一旦进入模型面，旧客户端缓存前缀失效一次（可回滚，重新构建即恢复）。
- 三条冻结契约的断言语义被强化（不可见 → 必须可见），回退需连带回退契约（已在契约 C4 留痕）。

## 5. 已知限制与遗留

1. `HostTurnEngine` 的 `workspace_root` 为**进程级首写者胜**：同进程多宿主不同根时沿用首个根（生产单宿主，契约 C2.4 登记）。
2. `SUBAGENT_MAX_CONCURRENT = 2` 未变；团队规模 > 2 并发派生会返回 `subagent_busy`（既有契约，非本波引入）。
3. 策略段只覆盖 pony-agent **真实存在**的工具（`team_task_get` / `wait_agent` 本仓不存在，文本已按真实工具面裁剪）。
4. 既有 schema 漂移（`nfr-baseline.schema.json` 三处既有字段未登记）本波**未修**，登记为 dev-backlog 候选。
5. 全量 `cargo test -p pony-agent-core --tests` 在并发构建压力下曾因 linker `EAGAIN`（环境资源耗尽）失败，非代码缺陷；已降并发复跑（见 §3 附注）。

## 6. v1.2 / v1.3 复审后追加（L3-A / L3-P 两条 FAIL 的处置）

| 审查发现 | 严重度 | 处置（v1.3） | 收据 |
|---|---|---|---|
| `workflow` 是桩却进模型面（伪造 `status:"success"`） | BLOCKING | 退回 `Internal`；模型面 39 → **38** 项；新增守卫用例 `c1_workflow_stub_is_not_model_visible` | `review-wave61-l3p.json` / `review-wave61-a.json` |
| 9 个团队工具是桩（spawn 丢弃 prompt / send 无消费方 / interrupt 只翻位） | BLOCKING | **接通真实现**：`send_message` 入队后后台 `drive_teammate_turn_in` 驱动真实 turn；spawn 投递初始 prompt 即驱动；回执投递 Lead 收件箱；`Running→Inactive` 收口 | `c7_send_message_drives_teammate_turn`（Stub 计数 + 状态机 + 回执三断言） |
| 桌面宿主不传 workspace_root ⇒ 子代理跑错目录 | BLOCKING | additive `SubagentTurnEngine::run_turn_in` + `HostTurnEngine` 覆写 + `ToolRouter.workspace_root` 透传 | `c8_subagent_turn_uses_parent_workspace_root` |
| 双重扣并发许可 ⇒ 有效并发 2→1 | BLOCKING | `gates_concurrency_internally()` 能力标志 + 入口门按需扣减（Stub 仍走入口门，t3_J/t3_K 零迁移） | 契约 C6.1 |
| 超时分支提前归还许可 ⇒ 实际并发可超 2 | BLOCKING | guard move 进工作线程，线程结束才归还 | 契约 C6.2 |
| 策略段引用不存在工具/错误码（`Bash` / `FS_STALE_VERSION`） | BLOCKING | 改为本仓真实 `Run` / `cas_conflict`，并加机器断言（必含 / 必不含） | `c3_*` 四条文本判定 |
| 策略段门控二次求值（splice 越界窗口） | ADVISORY | 单次求值 + `len-1` 推算下标（C9） | `c9_policy_gate_evaluated_once` |
| `pa110` 队列顺序用例与「入队即驱动」互斥 | ADVISORY→迁移 | 暴露 additive seam `enqueue_message`（纯入队），队列断言逐字未改 | `tests/pa110_teams_acceptance.rs:173-186` |

**最终门禁（v1.3 收据）**：8 个测试二进制 / 38 用例 exit 0（`.dev-team/green-phase-wave61-team.json`）；`wave61_team_activation` 12/12；`pa110` 5/5；连跑 3 次无竞态。

## 7. 遗留（本波不做，登记本波后续）

1. `interrupt_agent` 只能保证「下一次 drain 前停止」（引擎无 stop API）——契约 C7-A4 显式登记。
2. `--lib` 全量单测在并发构建压力下多次因 `failed to spawn thread: EAGAIN` 失败（环境线程耗尽，非断言失败）；低载时曾 1210/0。
3. 既有 schema 漂移（`nfr-baseline.schema.json` 3 处既有字段）未修，登记 backlog。
4. `HostTurnEngine.workspace_root` 首写者胜（同进程多根）——契约 C2.4 登记。

## 8. 终审 recheck 链（三轮复核）

| 轮次 | 审查者 | 对象 commit | verdict | 处置 |
|---|---|---|---|---|
| recheck-1 | L3-A 并发/生命周期 | `ed6d35e` | FAIL（1 BLOCKING） | 驱动循环以 `status==Inactive` 退出 ⇒ 在途消息滞留 → v1.5 改为「收件箱为空」唯一退出（`6b9e23a`） |
| recheck-1 | L3-P 生产六问 | `ed6d35e` | **PASS**（6 问全 PASS，7 ADVISORY） | 无阻断；ADVISORY 已登记/修正 |
| recheck-2 | 聚焦复核 v1.5 | `6b9e23a` | FAIL（1 BLOCKING，新回归） | 空闲态 interrupt 写 stale 标记吞掉下一次驱动 → v1.6 修复（`e403302`） |
| recheck-3 | 最终确认 | `e403302` | **PASS**（stale-marker CLOSED，4 ADVISORY） | 收口 |

收据：`review-wave61-fa.json`、`review-wave61-fp.json`、`review-wave61-fr.json`、`review-wave61-fv.json`（四份均通过 `dev-review-lint.py`）。
v1.6 后门禁：8 套件 **42 用例全绿**（wave61 16）、`cargo check --all-targets` exit 0、`wave61` 连跑 3 次一致。

**已知环境噪声（与本波无因果，已在基线 commit `52a7a21` 上复现）**：`otel_resource_env_test::pa153_t2_env_overrides_service_version_and_environment` 在 base 与本分支同样失败，属既存失败。
