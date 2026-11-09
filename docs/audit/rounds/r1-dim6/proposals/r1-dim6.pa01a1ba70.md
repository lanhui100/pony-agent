---
status: approved
approved_by: user
approved_at_unix: 1794140100
approved_scope: F1 / F2 / F3（写域按提案声明；F3 附带黑盒测试契约演进，见 approval_note）
approval_note: F3 使 `tests/otel_resource_turn_status_test.rs` PA154-T1 的 outcome 值断言（error → degraded）失效；该文件声明「实现方不得修改，须 Lead 批准」，本批准视为 Lead 对同一提交内契约演进的授权。
dimension: 6
dimension_name: tracing.self_consistency（上报链路自检）
round: r1-dim6
author_session: pa_01a1ba70-a314-7712-adf2-4bf675a96f99
baseline_commit: c1f47e9d6f8d1509f57d1cff71e38c9b2c4a2916
corpus_hash: 879597eb3594eb365bdbe033f7fbcd8353380e359170267f8d3e621255b8300a
evidence: 9 sessions / 3 tasks × 3（replay/base），receipts 001–005
---

# r1-dim6 提案：上报链路自检

## 阈值与断言（批准即冻结）

| 断言 | 口径 | 基线实测 | 通过条件 |
| :-- | :-- | :-- | :-- |
| **A1** 重放隔离 | 会话 `Environment: cwd=` 必须等于臂 worktree | 9/9 失配（`/home/dm/pony_agent`） | 失配数 = 0 |
| **A2** trace 级 release | Langfuse trace `release` 非空 | 9/9 为 `null`（`?release=0.1.158` 命中 0） | 空值数 = 0 |
| **A3** 结局单值 | 禁止 `phase=completed` 与 `outcome=error` 并存 | 9/9 冲突 | 冲突数 = 0 |

断言命令（唯一裁决者，退出码语义 0=pass / 1=fail / 2=证据缺失）：

```bash
bash docs/audit/rounds/r1-dim6/tools/assert-dim6.sh \
  docs/audit/rounds/r1-dim6/replay/<arm> <arm-worktree> \
  docs/audit/rounds/r1-dim6/metrics.json
```

已实测红相：缺 `metrics.json` → exit 2（UNKNOWN ≠ PASS）；基线数据 → exit 1。

## 修复方案（三处最小闭包）

### F1（blocker）CLI 工作区绑定缺失 → 与 TUI 同构

根因：`cli.rs:651` 恒传 `workspace_id: None`，会话不盖章，`resolve_session_workspace_root` 回落注册表 `default`，宿主 `-w` 被覆盖。

方案（复用既有 API，不新增契约）：CLI 启动时 `control_plane.ensure_workspace_for_root(&args.workspace)`，把返回 `record.id` 作为 `TurnInput.workspace_id` 的唯一来源 —— 与 `tui.rs:6995-6999` + `tui.rs:3309` 完全同构。

- 写域：`crates/pony-agent-core/src/bin/cli.rs`（含 CLI 侧单测）
- 回滚：还原该文件；会话 `workspace_id=None` 即恢复旧行为
- 反向证据（已实测）：注册表 default 指向 worktree 时，同一 `-w` 即可读到 `.audit-marker` ⇒ 缺陷定位在绑定链，不在工具层

### F2（major）trace 级 release 为空 → resource 侧补齐

根因：`langfuse.release` 只写在 span 属性。Langfuse `OtelIngestionProcessor.createTraceEvent` 对 trace 的 `release` 取 `spanAttributes[RELEASE] ?? resourceAttributes[RELEASE] ?? null`，而「浅 trace」（由子 span 先到触发）不含该 span 属性；worker 并发消费时后到的浅 trace 会把 `release` 覆写为 `null`。实测并存证据：trace `release=null` 但 `metadata.attributes["langfuse.release"]="0.1.158"`。

方案：`otel/mod.rs::resolve_resource_attributes` 增加 `("langfuse.release", service_version)` —— 与 `service.version` 同源同值（ADR 0055 已定该语义），使根 span 与浅 trace 两条路径都能取到。加单测断言资源属性含该键。

- 写域：`crates/pony-agent-core/src/agent/otel/mod.rs`
- 回滚：删除该键；span 级 `langfuse.release` 不受影响

### F3（major）结局语义缺「降级完成」档

根因：`otel/mod.rs:2300-2316` 在 `phase != failed` 但聚合到子 span 错误时写 `outcome=error`；`otel/mod.rs:2416-2417` 在 `phase=completed` 且 `tool_failures>0` 时写 `degraded=true`。两条规则独立成立，同一轮产出三种判定。

方案：`phase == "completed"` 且有工具失败时，`agent.turn.outcome` 写 `degraded` 而非 `error`（`Status` 仍置 error 以便 Tempo 侧可见）；`phase != completed` 的失败路径逐字不变。同步 `docs/observability.yaml` 该键值域，并按 ADR 0068 走契约版本化说明。

- 写域：`crates/pony-agent-core/src/agent/otel/mod.rs`、`docs/observability.yaml`
- 回滚：还原值域为恒 `error`
- 风险：值域变更影响 Langfuse 看板既有筛选；提案选择「新增值」而非「改既有值语义」，旧 `error` 语义（轮次失败）不变

## 不属本维度修复（记录，转后续维度）

- **F4 成本轨为空**：`deepseek/deepseek-v4.1-flash` 未进价目表，`pony.cost.priced=false` 属既定「宁缺勿错」策略，非上报缺陷 → 维度 5。
- **F5 headless 默认 gateway 为死地址**：`cli.rs:153-154`、`tui.rs:1404` 硬编码 `http://10.42.0.67:8080`，应改为空值 + 显式配置错误 → 单独 follow-up 卡。

## 写域冻结与防作弊

- 本提案批准时冻结 `corpus_hash` 与上述三处断言口径；事后放宽须新提案重审。
- 实施只允许触碰「修复方案」列出的写域；harness 其他改动一律判红。
- 验证：scratch worktree 双臂（baseline = `c1f47e9d`，candidate = 实施后提交）各 ≥3 会话重放 + `assert-dim6.sh` 退出码。
