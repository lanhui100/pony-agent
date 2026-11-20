# Agent Note: goal 停止原因从「一条字符串」拆成结构化访问器（对齐 deepseek-harness `GoalBlockReason`）

Status: implemented

## Problem

用户报「消息尾部为什么会出现 `goal 续轮未启动（首轮未成功收口，未进入自动续轮）`，这句的设计意图是什么、是否符合面向用户设计」。取证（`crates/pony-agent-core/src/agent/goal.rs:609-643`、`bin/tui/goal_panel.rs:49-51`、`bin/tui/errors.rs:168-171`）确认**设计意图正确但呈现与意图相反**，共三处硬缺陷：

1. **一个字符串服务四种消费者**：`visible_line()` 同时喂 ①底部状态行（窄、会被下一轮进度短语覆盖）②消息区（宽、要能回看）③Ctrl+G 面板（被套前缀）④结构化日志（不需要人话）。为了填满消息区就长出同义反复——`goal 续轮未启动（…未进入自动续轮）` 的括号尾巴只是把主句换个说法，同时外泄 `收口`（`TurnStreamOutcome` 的内部术语，用户无任何键位/命令与之对应）。
2. **严重级靠关键词猜**：`system_message_severity` 用 `error/fail/panic/失败/错误/异常/不可用/未产出终态` 关键词表判级。`FirstRoundNotSucceeded` 文案含「未成功」但不含其中任何一项 ⇒ 判 `Info` ⇒ 渲染成灰字。**为可见性新增的那条消息，成了 10 条里视觉权重最低的一条**，与 B193 的目的（让用户看见「不会自动继续」）直接相反。
3. **面板前缀制造自我否认**：`goal_panel.rs` 硬编码 `format!("已停止：{}", reason.visible_line())` ⇒ Ctrl+G 渲染成 `已停止：goal 续轮未启动（…）`；且 `row_style` 对 `已停止：` 前缀一律涂 `DANGER`，把「未启动」也一并涂成危险色 ——「未启动 vs 已停止」的语义区分在这一层被彻底抹掉，全靠主文案硬撑。

另外两条语用问题：中文「未启动」默认读作「还没启动（稍后会）」，与事实（不会自动继续）相反，等于复制了 B193 要消灭的干等症状；10 条文案里有 3 条在括号尾巴复述主句。

参考实现：deepseek-harness 的 `GoalBlockReason { code, message }`（`packages/goal/goal/src/types.ts:52-57`）把「机器可路由的分类码」与「人读解释」拆成两个字段，`code` 走确定性路由、`message` 给人和模型读，且 `code` 有形态校验（`fold.ts:73-85` 严格重放断言恰好两个字段）。

## Decision

1. **`GoalRoundStopReason` 增加七个结构化访问器**（`crates/pony-agent-core/src/agent/goal.rs`），按「一个事实一个家」划分唯一职责：

   | 访问器 | 唯一职责 | 唯一消费方 |
   |---|---|---|
   | `code() -> &'static str` | 机器可路由码（lower_snake_case） | 结构化日志 `goal_rounds_stopped.reason` |
   | `severity() -> GoalStopSeverity` | 渲染严重级（core 中性枚举 `Info`/`Warning`/`Error`） | 消息区图标配色、面板行样式、状态条 |
   | `headline() -> &'static str` | 人话主句（现在是什么状态） | `visible_line()` 首段 |
   | `detail() -> Option<String>` | 可核对的原因细节（计数/轮次） | `visible_line()` 中段 |
   | `action_hint() -> Option<&'static str>` | 下一步动作 | `visible_line()` 末段 |
   | `enters_history() -> bool` | 是否值得写入会话可见历史 | `GoalRoundsStopped` 事件处理 |
   | `never_started() -> bool` | 续轮是否**从未启动** | 状态条措辞（`·停` / `·未续`） |

   `visible_line()` 改为三段式拼接 `headline｜detail｜action_hint`（`None` 段省略）。

2. **严重级改由枚举给，禁止关键词猜**：`errors.rs` 新增 `severity_for_goal_stop()`（枚举 → `SystemMessageSeverity` 纯翻译）与 `goal_stop_severity_for_line()`（按 `all_variants()` 的逐字文案做身份比对），并在 `system_message_severity()` **最前面**短路。`FirstRoundNotSucceeded` 硬锁 `Error`。

3. **日志 tag 收敛到 `code()`**：删除 `tui.rs` 的 `goal_round_stop_reason_tag`（第二份手写字符串表）。两份表并存正是「日志 tag 与枚举漂移」的根因。

4. **面板去前缀**：`goal_panel.rs` 直接 push `reason.visible_line()`，`row_style` 改按 `state.goal_rounds_stopped` 的枚举严重级着色，不再靠前缀字面量识别。

5. **状态条区分措辞**：`·停` 拆成 `·停` / `·未续`，由 `never_started()` 唯一决定（`FirstRoundNotSucceeded` / `NoActiveGoal` / `LoopAlreadyOwned` 三个原因「从未启动」）。

6. **历史准入**：`LoopAlreadyOwned` 的 `enters_history()` 返回 `false`——事实是「另一个循环在跑，续轮并没有停」，写进消息区等于留一条假结论。

7. **逐字文案表**（十变体，冻结于契约矩阵 `.dev-team/contract-matrix-pa171.md` → `Amendment: b193-2`）：

   | 变体 | `visible_line()` | severity | enters_history |
   |---|---|---|---|
   | `Cancelled` | `目标已暂停：续轮已停止｜用户取消｜发消息可继续` | Info | true |
   | `ConsecutiveFailures` | `目标已暂停：续轮已停止｜连续 {n} 轮失败｜查看上方错误，修复后发「继续」` | Error | true |
   | `BudgetExhausted` | `目标已暂停：续轮已停止｜目标轮次已用尽 {a}/{b}｜/goal 查看或提高轮次上限` | Error | true |
   | `AutoContinueBudget` | `目标已暂停：续轮已停止｜连续自动推进达上限｜发消息可继续` | Warning | true |
   | `TurnSuspended` | `目标已暂停：续轮已停止｜本轮在等你作答/审批｜先回答上面的提问` | Warning | true |
   | `TurnUnclassified` | `目标已暂停：续轮已停止｜本轮未产出可用终态｜查看日志后发「继续」` | Warning | true |
   | `LoopAlreadyOwned` | `本会话已有续轮循环在运行｜本线程让位，未推进轮次` | Info | **false** |
   | `BudgetPersistFailed` | `目标已暂停：续轮已停止｜本轮预算推进写盘失败｜检查磁盘后发「继续」` | Error | true |
   | `NoActiveGoal` | `目标已暂停：续轮已停止｜当前会话没有可续的目标｜/goal 创建目标` | Warning | true |
   | `FirstRoundNotSucceeded` | `目标已暂停：本轮未成功｜首轮失败，自动续轮未启动｜发「继续」重试` | **Error** | true |

   `NoActiveGoal` 的 detail 原拟「没有进行中的目标」，因「进行中」是 `GoalPhase::Active` 的既有译词（`goal_panel::phase_label`），撞词会让横幅「阶段 进行中」与停止原因互相打脸，改为「没有可续的目标」。

## Alternatives considered

- **新增 `MessageRole::Status` 变体承载状态消息**（用户最初追问「能不能新增一种消息/状态类型」）：技术上可行且零污染——TUI `state.messages` 不落盘（会话历史角色白名单只有 `user`/`assistant`/`host`，`agent/session/types.rs:274-276`）、不进模型上下文（`agent/context.rs` 对 `ChatMessage` 零读取）、`rewind`/`selection` 不感知 role。**被否决**，理由三条：①它不治根——严重级仍要靠关键词猜，缺陷 2 原样保留；②会污染「最后一条是谁说的」语义（自动标题、滚动锚点都看末条）；③桌面端 role 只有 `user|assistant|tool`，且 goal 停止在 `src/` 下 `goal_rounds_stopped` 零命中（桌面端完全不可见），单端再细分会加剧双端漂移。改为 `SystemMessageKind` 枚举的提案同样被否决：`ChatMessage` 有 56 处构造点，加字段会连带改动多个冻结测试夹具，成本远大于收益。
- **把 `GoalData.blocked_reason` 从 `Option<String>` 改成结构化 `{ code, message }`**（完全对齐 dsh `GoalBlockReason`）：**显式 deferred**。该字段跨越 serde 持久化格式 / 桌面端 `src/types/` / 工具 JSON schema（`agent/tools/schema.rs:949`）三处契约，改动面远超「停止原因可见性」这一最小必要闭包；且本轮所有缺陷都能在 TUI 投影层闭合。dsh 结构化收益（跨进程路由、模型可读）在本仓尚无消费者，提前引入等于为「以后可能用得上」立法。
- **`visible_line` 拆成「状态行短版 / 消息区长版」两个方法**：会被迫维护两套文案、两处同步漂移风险，且状态行宽度是运行时变量（`status_bar_line(state, width)` 已有让位机制），拆方法解决不了真问题。三段式拼接 + 调用方按需取段更小。
- **保留 `已停止：` 前缀、只在首轮门时特判**：仍是「靠字面量分支」，新增第 11 变体时会再踩一次；且前缀着色与文案严重级两套判据必然漂移。删除前缀让面板与消息区同源。
- **把「未启动」改回「已停止」**（消除语用歧义的最短路径）：与事实相反，且违反 B193 决策记录冻结的语义区分。改为保留「未启动」+ 用 `action_hint` 补「发「继续」重试」消除「稍后会自己继续」的误读。
- **给 10 条文案都补一个动作指引门禁（要求 `action_hint().is_some()`）**：`LoopAlreadyOwned` 不该有动作（用户无需为「另一个循环在跑」做任何事），故门禁改为「**进历史的原因**必须有 action_hint」，在 `tests/b193_2_goal_stop_visibility_contract.rs` 机械可判。

## Consequences

- 用户侧：消息区尾部的结论变成「目标已暂停：本轮未成功｜首轮失败，自动续轮未启动｜发「继续」重试」，且是 Error 红字（此前是灰字）；Ctrl+G 面板不再出现「已停止：…未启动」的自我否认；状态条对「从未启动」显示 `·未续` 而非 `·停`。
- 三处配色（消息区 / 面板 / 状态条）首次同源（`severity_for_goal_stop`），改严重级只改一处枚举。
- 日志 `reason` 取值集合逐字不变（十个 code 与历史一致），审计面无破坏；`tui.rs` 少一张手写表。
- 新增 `tests/b193_2_goal_stop_visibility_contract.rs`（9 条机器判据：code 唯一性与形态、严重级枚举驱动、语义区分不回退、历史准入、禁用词、三段式、never_started 覆盖面）。
- `FirstRoundNotSucceeded` 文案不再含「收口」，`goal_round_stop.rs` 里原来断言 `contains("首轮未成功收口")` 的用例同步改为断言语义（含「未启动」+ 不含「已停止」+ 严重级 Error + 含动作），避免文案改造时用例变成「抄写测试」。
- 桌面端仍无 goal 停止可见化（本轮不动）；已登记为后续独立议题。
