# Wave-61 交付说明：Agent Team 激活（工具面 × 宿主接线 × 策略提示）

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
| C1 | 10 项团队工具 `ModelVisible`（模型面 29 → 39 项） | `tools.rs:7422-7437` |
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
- 工具面 29 → 39 属 `additive`；**反向不可逆点**：团队工具一旦进入模型面，旧客户端缓存前缀失效一次（可回滚，重新构建即恢复）。
- 三条冻结契约的断言语义被强化（不可见 → 必须可见），回退需连带回退契约（已在契约 C4 留痕）。

## 5. 已知限制与遗留

1. `HostTurnEngine` 的 `workspace_root` 为**进程级首写者胜**：同进程多宿主不同根时沿用首个根（生产单宿主，契约 C2.4 登记）。
2. `SUBAGENT_MAX_CONCURRENT = 2` 未变；团队规模 > 2 并发派生会返回 `subagent_busy`（既有契约，非本波引入）。
3. 策略段只覆盖 pony-agent **真实存在**的工具（`team_task_get` / `wait_agent` 本仓不存在，文本已按真实工具面裁剪）。
4. 既有 schema 漂移（`nfr-baseline.schema.json` 三处既有字段未登记）本波**未修**，登记为 dev-backlog 候选。
5. 全量 `cargo test -p pony-agent-core --tests` 在并发构建压力下曾因 linker `EAGAIN`（环境资源耗尽）失败，非代码缺陷；已降并发复跑（见 §3 附注）。
