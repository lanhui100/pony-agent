# Agent Note: 删除单轮累计墙钟预算，止损下沉到能力层

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0091-turn-wall-clock-removal.md` 之归档快照，仅供溯源。

## 背景

ADR 0060 决策 5 引入「单轮墙钟预算」（`PONY_AGENT_MAX_TURN_WALL_SECONDS`，默认 900s、上限 7200s）：
累计耗时超预算后，在**下一个工具调用边界**放弃 follow-up，改走恢复式收尾要模型立刻给结论。

上线后该闸从「兜底」变成了**主止损**，并开始误伤正常长任务：多个会话在 900s 处被强制收尾，
其中包含仍在稳定推进（每个 hop 都有真实世界变化）的轮次。根因不是阈值调小了，而是**口径错了**——
它度量的是「整轮累计墙钟」，而工程任务的合法时长与任务规模成正比，任何固定累计阈值都必然
先杀死「最认真干活」的那些轮次。ADR 0060 自己也写明「预算是兜底，不是主止损手段」，事实与此相反。

同期核查能力层已有的止损（对照 `deepseek-harness` 的 `guard/timeout-policy` 与
`util/timeout` 设计：库只做计时与分类，终止权留在各能力；轮次驱动循环
`core/agent-loop/src/agent.ts` 的 `while (await this.turn()) {}` **无总时限**）：

| 层 | pony 现状 | 位置 |
|---|---|---|
| LLM 流 | 不设总 deadline，只有 180s **读空闲**（等价 deepseek 的 `idleWatchdog`） | `provider/mod.rs` |
| 工具 | `Run` 默认 120s / 上限 600s，到期 promote 成后台 job；其它工具默认 deadline 10s；web 15s；MCP connect 20s / request 5s | `agent/tools.rs`、`agent/tool_runtime.rs` |
| 循环 | hops 1024 / followups 500 / 同指纹 3 次 / 同工具连续失败 4 次 / 同签名计数 2 次 | `agent/runtime/limits.rs` |

即：**每一条真正可能「卡死」的通路都已有自己的时限**，累计墙钟不覆盖任何独家风险，
只额外杀死「合法但耗时」的轮次。

## 候选方案 / Alternatives considered

- **A. 保留 900s，只把默认值调大（如 3600s）**：落选。口径错误不因阈值变大而消失，只是把撞墙点后移；
  下一次长任务仍会撞上，且「到底多大才对」没有可论证的答案（任务时长无上界）。
- **B. 改为「无进展超时」（连续 N 秒无任何事件才收尾）**：落选。检查点只有 hop 边界，而 hop 边界
  本身就是「有进展」的证据；把时钟在每次 hop 完成时重置后，唯一能测到的是「单个 hop 内部的
  模型调用 + 工具执行时长」——这两段分别已有 180s 读空闲与工具 deadline 兜底，该计时器要么永不触发
  （死代码），要么与既有能力级时限重复计数（双重止损，语义更难推理）。
  这与 deepseek-harness 明确拒绝「中央 `ctx.timeout` 服务」的结论一致：内核不该知道每种能力怎么被杀。
- **C. 保留累计墙钟但改成「软提醒」**（超预算只注入提示、不阻止 follow-up）：落选。提示会长期驻留
  上下文并被模型当成硬约束，实际效果仍是让模型提前收尾，只是把止损点变得不可预测。
- **D. 删除累计墙钟，止损全部下沉到能力层（采纳）**：LLM 读空闲、工具 deadline、步数闸、三道失败闸
  各自负责自己的风险；轮次本身不设总时限，与 deepseek-harness 的轮次驱动模型对齐。

## 决策

1. **删除** `DEFAULT_MAX_TURN_WALL_SECONDS`、`MAX_ALLOWED_TURN_WALL_SECONDS`、
   `MAX_TURN_WALL_SECONDS_ENV`、`build_turn_wall_budget_error`、`max_turn_wall_seconds()`、
   `parse_max_turn_wall_seconds()` 与测试覆写槽 `TURN_WALL_OVERRIDE` / `set_turn_wall_override`
   （`crates/pony-agent-core/src/agent/runtime/limits.rs`）。
2. **删除两处墙钟分支**：`turn_sync.rs` 与 `turn_stream.rs` 在「还有待调工具」处先判墙钟再判
   follow-up 限额的双分支结构，退化为单分支 `if completed_hops >= max_tool_followups_per_turn()`
   ——恢复式收尾仍在，触发理由只剩「步数用尽」与既有三道失败闸。同步删除 `handle_sync_tool_turn`
   的 `turn_started_at: &Instant` 形参（删闸后该形参在函数体内无消费者；流式侧同名字段仍用于
   首 token 延迟与 span 时长，保留）。
3. **不新增**任何替代计时器、不保留旧环境变量别名、不写「兼容旧名」分支：环境变量
   `PONY_AGENT_MAX_TURN_WALL_SECONDS` 自此失效（未设置即无墙钟），语义已无对应物，留别名等于留谎。
4. **止损责任归属**（删除后的完整清单，任何一条被削弱都必须新开 ADR）：
   - LLM：180s 读空闲（无总 deadline）；
   - 工具：`Run` 120s 默认 / 600s 上限 + 到期 promote 后台 job、通用工具 10s、web 15s、MCP 20s/5s；
   - 循环：hops 1024、followups 500、同指纹 3 次、同工具连续失败 4 次、同签名计数 2 次。
5. **ADR 0060 决策 5 的墙钟部分由本篇接替**（0060 其余四道止损与上下文完整性决策继续现行）。

## 影响

- **用户可见行为**：长任务不再在 900s 处被强制收尾，可以在 hop/followup 预算内一直推进到完成；
  长时间无输出的情形仍由工具 deadline 与读空闲兜底，不会无限静默挂起。
- **遥测**：不新增键、不删除键；恢复式收尾仍通过既有 `llm.request.mode = tool_recovery` 可检索，
  只是 `tool_recovery` 的理由集合少了「墙钟」一项。`docs/observability.yaml` 的非遥测登记条目同步修订。
- **兼容性**：`PONY_AGENT_MAX_TURN_WALL_SECONDS` 失效（不再被读取）。Rust 内部无其它消费者，
  删除后 `cargo check --all-targets` 全绿即为证伪条件。
- **验收**：`cargo check -p pony-agent-core --all-targets` 0 error；`cargo test -p pony-agent-core --lib`
  全绿（删除墙钟用例后计数应减去 2 例：一条解析断言、一条恢复式收尾用例，后者的恢复路径本身
  已由 follow-up 限额用例覆盖）。证伪：`rg -n "turn_wall|TURN_WALL|墙钟预算" crates` 必须为 0 命中。
- **风险与缓解**：删掉唯一的「时间维度」闸后，理论上「大量各不相同且都不失败的工具调用」仍可长跑——
  由 hops 1024 / followups 500 封顶，属可接受的有界成本；若将来出现真实无界案例，应按**能力**
  加时限（工具自带 `timeoutMs` 或读空闲），不得重建中央累计墙钟。

