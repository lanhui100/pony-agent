# Agent Note: 删除单轮累计 900s 墙钟预算，止损下沉到能力层

Status: implemented

## Context

ADR 0060 决策 5 引入的单轮墙钟预算（`PONY_AGENT_MAX_TURN_WALL_SECONDS`，默认 900s、上限 7200s）
在 `turn_sync.rs` / `turn_stream.rs` 的「还有待调工具」边界检查，超预算即转恢复式收尾。
上线后多个真实会话在 900s 处被强制收尾，其中包含仍在稳定推进的轮次——该闸从「兜底」变成了主止损。

口径错误在于：它度量**整轮累计墙钟**，而工程任务的合法时长与任务规模成正比，任何固定累计阈值都会
优先杀死「干活最多」的轮次。ADR 0060 自己写的也是「预算是兜底，不是主止损手段」。

对照 `deepseek-harness`（`packages/guard/timeout-policy`、`packages/util/timeout`、
`core/agent-loop/src/agent.ts` 的 `while (await this.turn()) {}`）：轮次驱动循环**无总时限**，
终止权分散到各能力——工具在 `ToolDefinition` 上声明自己的 `timeoutMs`，无预算的 bash/read/write/edit
永不切断；LLM 侧用 `idleWatchdog`（默认 300s，仅在「等下一个 chunk 未结算」时计时），不做总调用 deadline。
其设计文档明确拒绝中央 `ctx.timeout` 服务：「内核不该知道每种能力怎么被杀」。

pony 已有的能力层止损（逐项核对过，均现行）：

- LLM：无总 deadline，只有 180s 读空闲（`crates/pony-agent-core/src/provider/mod.rs`）；
- 工具：`Run` 默认 120s / 上限 600s，到期 promote 成后台 job；通用工具默认 deadline 10s；
  web 15s（链级总预算 30s）；MCP connect 20s / request 5s（`agent/tools.rs`、`agent/tool_runtime.rs`）；
- 循环：hops 1024 / followups 500 / 同指纹 3 次 / 同工具连续失败 4 次 / 同签名计数 2 次（`agent/runtime/limits.rs`）。

结论：累计墙钟不覆盖任何独家风险，只额外杀死合法长轮。

## Decision

删除整个墙钟闸，不换任何替代计时器：

1. `agent/runtime/limits.rs`：删 `DEFAULT_MAX_TURN_WALL_SECONDS`、`MAX_ALLOWED_TURN_WALL_SECONDS`、
   `MAX_TURN_WALL_SECONDS_ENV`、`build_turn_wall_budget_error`、`max_turn_wall_seconds`、
   `parse_max_turn_wall_seconds`、`TURN_WALL_OVERRIDE`、`set_turn_wall_override`。
2. `turn_sync.rs` / `turn_stream.rs`：双分支（先墙钟、后 follow-up 限额）退化为单分支
   `if completed_hops >= max_tool_followups_per_turn()`；恢复式收尾路径本身保留、逐字不动。
3. `runtime/tests.rs`：删墙钟解析断言、`TurnWallOverrideGuard`、`turn_wall_budget_wraps_up_with_recovery_answer`。
4. 不保留旧 env 别名：语义已无对应物，留别名等于留谎；`PONY_AGENT_MAX_TURN_WALL_SECONDS` 自此失效。
5. ADR：新增 `docs/decisions/0091-turn-wall-clock-removal.md`，并同提交在 ADR 0060 决策 5/6 与影响段
   标注「已被 0091 接替」。

## Alternatives considered

- **保留 900s、只调大默认值**：口径错误不随阈值变大而消失，只是把撞墙点后移；且「多大才对」无
  可论证答案（任务时长无上界）。下一次长任务必然重现同一问题。
- **改为「无进展超时」（连续 N 秒无事件才收尾）**：检查点只有 hop 边界，而 hop 边界本身就是进展证据。
  时钟在每次 hop 完成时重置后，唯一能测到的是「单 hop 内的模型调用 + 工具执行时长」——这两段分别
  已有 180s 读空闲与工具 deadline 兜底，于是该计时器要么永不触发（死代码），要么与既有能力级时限
  重复计数（双止损，语义更难推理）。与 deepseek-harness 拒绝中央 timeout 服务的结论一致。
- **保留墙钟但降级为「软提醒」注入**：提示会长期驻留上下文被模型当硬约束，实际仍让模型提前收尾，
  只是让止损点变得不可预测。
- **删除后按「整轮总 token 或总耗时」重建中央闸**：落选，同属中央计时服务反模式；
  成本侧已有用量治理（`usage_governance`）与预算口径，无需在循环里再造一个。

## 影响 / 验证

- 用户可见：长任务在 hop/followup 预算内可一直推进；静默挂起仍由工具 deadline 与读空闲兜底。
- 遥测：无新增/删除键；`llm.request.mode = tool_recovery` 仍可检索恢复式收尾，理由集合少「墙钟」一项。
- 机械验收：`cargo check -p pony-agent-core --all-targets` 0 error；`cargo test -p pony-agent-core --lib`
  全绿；`rg -n "turn_wall|TURN_WALL|墙钟预算" crates` 为 0 命中（`compaction_ops.rs` 与
  `tests/wave54_metric_truth.rs` 里的「墙钟」是时长口径措辞，不在本闸范围内）。
