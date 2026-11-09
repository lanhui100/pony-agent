# r1-dim6 根因分析：上报链路自检（tracing.self_consistency）

- 轮次：r1-dim6　维度：6（上报链路自检）
- 阶段：audit（collect 回执 001–003）
- 基线提交：`c1f47e9d`　corpus_hash：`879597eb…8300a`
- 证据臂：`replay/base`（9 会话 = 3 任务 × 3 会话），臂 worktree `/tmp/audit-r1-base`

## 结论摘要

| 编号 | 发现 | 级别 | 机器证据 |
| :-- | :-- | :-- | :-- |
| F1 | **重放隔离失效**：`ponycli -w <worktree>` 不决定工具根，工具根取会话注册表 `default` 记录；9/9 会话 env cwd = `/home/dm/pony_agent`，任务在空工作区执行 | blocker | 回执 003：`isolation_mismatch_count=9`，脚本退出码 1 |
| F2 | Langfuse **trace 级 `release=null`**（span 级有 `langfuse.release=0.1.158`），按发布线筛选取不到数 | major | `traces?release=0.1.158` → 0 条；`traces?version=0.1.158` → 5 条 |
| F3 | 同一轮**四信号三判定**：`phase=completed` + `outcome=error` + `tool_failures=3` + `degraded=true`，而 CLI 退出码 0 | major | trace `be963c84…` 根 span 属性 |
| F4 | 成本轨为空：`totalCost=0`、`pony.cost.priced=false`，成本治理维度取不到数 | major | 8/8 近期 trace `totalCost=0` |
| F5 | headless 默认 gateway 为死地址 `http://10.42.0.67:8080`，复现必须显式 `-g` | minor | 默认调用 connect refused；显式 `-g https://tokens.ponyjob.top/v1` exit 0 |

## F1 根因（阻断性）

调用链与实测：

1. `cli.rs:954-959` 把 `-w` 传给 `AgentRuntimeBuilder::workspace_root`，CLI 横幅如实打印 `Workspace: /tmp/audit-r1-base`。
2. `TurnInput.workspace_id` 恒为 `None`（`cli.rs:651`）⇒ `resolve_governed_turn_context`（`runtime/mod.rs:388-410`）不给会话盖章 ⇒ 会话 `workspace_id` 保持 `None`。
3. 工具执行上下文由 `batch_plan.rs:196-197` 构造：`workspace_root: self.resolve_session_workspace_root(session_id)`。
4. `resolve_session_workspace_root`（`runtime/mod.rs:452-470`）：owner 为 `None` → `sessions.resolve_workspace_root(None)` → **`DEFAULT_WORKSPACE_ID` 注册项** → `/home/dm/pony_agent`；只有该解析失败才回落 `self.workspace_root`。**宿主传入的 `-w` 因此被注册表默认值覆盖。**

控制变量实验（三组，均 exit 0）：

| 组 | 注册表 default root | `-w` | 工具实际根 | 结果 |
| :-- | :-- | :-- | :-- | :-- |
| a | `/home/dm/pony_agent` | `/tmp/audit-r1-base` | `/home/dm/pony_agent` | 读 `.audit-marker` → `invalid_path` |
| b | 注册表清空 | `/tmp/audit-r1-base` | 仍 `/home/dm/pony_agent`（回落 `compute_default_workspace_root()`） | 同上 |
| c | `/tmp/audit-r1-base` | `/tmp/audit-r1-base` | `/tmp/audit-r1-base` | 读到 `MARKER-CONTENT-ONLY-IN-SCRATCH` |

⇒ 决定工具根的是**注册表 default 记录**，不是 `-w`。实验后注册表已按 sha256 `c3ee429d…` 还原。

伴生代价（同臂 9 会话）：工具活动 826 次、失败 313 次（**37.9%**）；输入 token 合计 **699,222**；全部消耗在"路径不存在"的无效探索上。这不是重放脚本的写法问题，而是**宿主声明的工作区与工具实际工作区不一致**——同一条缺陷同时污染 `Environment: cwd=` 上报值（`session_workspace_cwd` 同源走注册表），使维度 6 的核心命题（上报值 == 实际值）不成立。

## F2 根因

`otel/mod.rs:98-102` 已记录该缺口（`langfuse.release` 需 span 属性显式给出，`service.version` 资源属性只填 `version`）。span 级已正确写入 `langfuse.release`，但 **trace 级 `release` 由 Langfuse 从根 span 特定字段推导**，当前根 span 未提供该字段，导致 trace 级为空。影响：ADR 0101 的 release 前后对照（`langfuse.release` 作为对照键）实际不可用。

## F3 根因

`otel/mod.rs:2300-2316`（`TurnTraceGuard::drop`）：`phase != "failed"` 但聚合到子 span 错误时，写 `agent.turn.outcome="error"` 并置 `Status::error`；同时 `otel/mod.rs:2416-2417` 在 `phase == "completed"` 且 `tool_failures > 0` 时写 `degraded=true`。两条规则独立成立，于是"完成但有工具失败"这一常见情形同时产出 `phase=completed` 与 `outcome=error`。语义上不是 bug 的实现错误，而是**契约缺一个"降级完成"档**：消费方（Langfuse 看板、verdict 断言）无法用单一键判断轮次结局。

## F4 根因

`deepseek/deepseek-v4.1-flash` 未进价目表 ⇒ `pony.cost.priced=false`、`agent.turn.cost_usd` 不输出（按"无计价不写 0"纪律正确），Langfuse `totalCost=0`。成本维度不可审核，属价目表覆盖缺口，非上报链路缺陷。

## F5 根因

`cli.rs:153-154` 与 `tui.rs:1404` 硬编码兜底 `http://10.42.0.67:8080`。默认值不可达时以 connect refused 失败，缺少"未配置 gateway"的显式配置错误。

## 对维度 6 的判定

维度 6 的命题是「上报值 == 实际值，且四宿主同口径」。F1 证明**声明工作区与实际工作区不一致**，属 blocker；F2/F3/F4 属同一维度的口径缺口。本维度未通过，需进入 propose。
