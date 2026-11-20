# Agent Note: ACP 整轮预算补登 NFR 对照键 + B164 诊断清单补登 + timeoutMs 校验前移到 spawn 之前

Status: implemented

影响文件：`.dev-team/nfr-baseline.json`、`.dev-team/nfr-baseline.schema.json`、
`.dev-team/contract-matrix-acp-p0.md`、`crates/pony-agent-core/src/agent/tools/orchestration.rs`、
`crates/pony-agent-core/src/agent/tools/tests/orchestration.rs`。

## Problem

`B170`（Gate-2 r4 三条 ADVISORY 合并）暴露的是**声明层与实现层脱钩**，不是新缺陷：

1. **整轮预算没有 NFR 对照键**：`ACP_DEFAULT_TURN_BUDGET`（`acp/mod.rs:36`，120s）是 `acp_delegate`
   与 CLI `--timeout-secs` 共同的单轮预算真源，但 `.dev-team/nfr-baseline.json` 的 `acp` 段只有
   `request_timeout_ms=5000`（单请求往返）。声明面找不到整轮预算的值，等于「120s 只在代码里，
   审计时无法机械对照」。
2. **B164 诊断清单漏登一处**：B164 把 ACP 诊断从裸 `eprintln` 收口到结构化日志时，清单覆盖了
   `acp/{transport,client,session,handlers}.rs` 与 CLI，却漏了
   `agent/tools/orchestration.rs` 的 `acp_delegate 起轮（budget/能力/父 turn 归属）`
   （`log_json_with_fields`）。结果是「ACP 诊断已全量结构化」这句 A 面声称，在工具层那一处没有清单依据。
3. **纯输入校验排在副作用之后**：`execute_acp_delegate` 里 `timeoutMs == Some(0)` 的
   `invalid_arguments` 校验原本在 `spawn_with_full_options(...)` 成功之后，靠 `session.close()` 兜底。
   后果是两重倒置——①非法入参先付一次子进程 spawn 开销；②当 agent 不可启动时，用户拿到的是
   `acp_spawn_failed`/`acp_config_agent_not_found`，而真正的原因是参数非法。

前序决策见 `implemented/architecture/2026-11-14-acp-client-external-agent-integration.md`
（Fix-round ①「单轮预算单一真源 = 120s」）。本条**不改变** 120s 这个数值决定，只补它的声明缺口
与校验顺序。

## Decision

1. **`acp.turn_budget_ms = 120000` 作为 additive NFR 键登记**：`.dev-team/nfr-baseline.json` 的
   `acp` 段追加 `turn_budget_ms`；`.dev-team/nfr-baseline.schema.json` 同批追加 `properties` 与
   `required`（该对象 `additionalProperties: false`，只改基线文件会被 schema 拒）。
   `.dev-team/contract-matrix-acp-p0.md` §3 增对照行，真值源锚到 `acp/mod.rs:36`。
2. **B164 诊断出口清单补登**：契约矩阵新增 §3.1，逐行列出五处结构化诊断通道，把
   `agent/tools/orchestration.rs:231` 标为 B170 补登项，并保留 `bin/cli/acp.rs` 的 B237 C3
   「用户面 stderr，不属日志通道」豁免口径。
3. **`timeoutMs == Some(0)` 校验前移**：校验块移到 `prompt` 非空校验之后、`resolve_agent` 与
   任何 spawn 之前；错误码 `invalid_arguments` 与文案不变；删除 spawn 后的 `Some(0)` 分支与
   `session.close()`，该处只留 `Some(ms)` / `None => ACP_DEFAULT_TURN_BUDGET` 的纯映射。
4. **行为由顺序性代码保证，并由测试双向绑定**：新增
   `acp_delegate_zero_timeout_checked_before_agent_resolution`——同一不存在的 agent 下，
   `timeoutMs: 0` 必须得 `invalid_arguments`（文案含「timeoutMs 必须大于 0」），
   `timeoutMs: 200` 必须得 `acp_config_agent_not_found`。后者证明前移只拦 0、没有顺手改掉 agent 解析语义。

## Alternatives considered

- **把 120s 塞进 `acp.request_timeout_ms`**：语义不同。5000ms 是 stdio 子进程的**单请求往返**，
  120000ms 是**整轮 prompt** 的墙钟预算；合并会让「单请求 5s 硬顶」这条既有承诺失去载体，
  也让审计无法区分「一次往返超时」与「整轮超时」。故按 additive 增键。
- **只改 `nfr-baseline.json`，不动 schema**：schema 里 `properties.acp.additionalProperties: false`
  且 `required` 显式列举，只加实例会直接校验失败；且「required 全覆盖」是这道门的存在意义，
  放宽成 `additionalProperties: true` 等于用降低约束换绿灯。
- **保留 spawn 后的校验，把 `session.close()` 换成更早的 `try_spawn` 探测**：仍需先决定 `spec`
  与装配 handler，改动面更大；而「纯输入校验就该在任何副作用之前」本就不需要额外机制。
- **把校验再前移到 `serde_json::from_value` 之前**：不可行——`timeout_ms` 就在 args 里，
  不解析拿不到值；解析失败自身已有独立的 `invalid_arguments` 分支。
- **把 B164 清单补登写进已归档的 B164 账本记录**：账本终态是冻结事实（归档不改一字），
  补登属于新事实，应落在契约矩阵并挂 B170 号，而不是回写历史条目。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core` → exit 0。
- `bash scripts/cargo-lane.sh test -p pony-agent-core --lib agent::tools::tests::orchestration`
  → `14 passed; 0 failed`，含新增用例。
- `python3 .dev-team/nfr-lint.py` → 仍为 `RESULT: FAIL (23 error(s))`（主干历史存量，与 acp 无关），
  且 `grep -i 'acp\.'` 零命中——证明本次新增键既未引入新错误，也不靠删字段降数。
- `python3 -c "import json;print(json.load(open('.dev-team/nfr-baseline.json'))['acp']['turn_budget_ms'])"`
  → `120000`；schema 侧三集合（baseline 键 / properties / required）相等且 `additionalProperties: false`。
