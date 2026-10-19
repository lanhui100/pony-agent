# Plan / Goal 链路冻结契约（wave-1，C1–C6）

状态：**契约冻结**（protocol-agent 签署，wave-1）；实现态：**空桩**（`unimplemented!("wave-1")` / `throw new Error("NotImplemented: wave-1")`），由 wave-1 executor 在 green 阶段替换为业务实现。
唯一真源：`.dev-team/contract-wave-1.md`（波内不得就地修改）；本文档是该契约的可执行落点，用于把 C1–C6 转成签名、事件名、文案、env 与语义；**本文档不新增、不削弱任何条款**。
配套 NFR：`.dev-team/nfr-baseline.json`（`auto_continue_max=8`、`goal_store_atomic_write=true`、`external_call_timeout_ms=3000`、`logging{format:"json",trace_id_required:true,trace_id_field:"trace_id"}`）。
基线：commit `4e4fedc`（main）。

---

## 0. 空桩清单（唯一写方：protocol-agent）

| 文件 | 追加内容 | 纯度 |
|---|---|---|
| `crates/pony-agent-core/src/agent/goal.rs` | 5 个 `pub fn`（见 §1/§2/§3） | 仅签名 + `unimplemented!("wave-1")`，既有函数体零改动 |
| `src/stores/plan.ts` | `export async function refreshForTurn(sessionId: string)` | 仅签名 + `throw` |
| `src/stores/goal.ts` | `export async function refreshForTurn(sessionId: string)` | 仅签名 + `throw` |

签名区外的两处**非业务**标注（不改变语义、green 阶段随实现删除）：

1. Rust 空桩函数上 `#[allow(dead_code, unused_variables)] // wave-1 空桩期（未接线）`——沿用 wave-38/39/40 协议空桩惯例，避免空桩期 `cargo check` 产生 unused-variable 噪音。
2. TS 空桩体内 `void sessionId;`——`tsconfig.json` 开启 `noUnusedParameters`，签名参数在空桩期未读取会触发 `TS6133`；不得改用 `_sessionId`（会破坏冻结签名字面）。

**重名检查**：两处 `refreshForTurn` 追加前，`src/stores/plan.ts` / `src/stores/goal.ts` 均无同名导出（plan.ts 既有导出仅 `usePlanStore`；goal.ts 仅 `useGoalStore`），故**不启用 `Wave1` 后缀**，既有导出零删除。

---

## 1. C1 目标真值源（goal-core）

```rust
// crates/pony-agent-core/src/agent/goal.rs
pub fn active_goal_for_session(session_id: &str) -> Option<GoalData>;
```

判定规则（全部为硬条件，缺一不命中）：

1. `GoalData.session_id == session_id`（精确匹配，禁止前缀/模糊匹配）；
2. `GoalData.phase == "active"`；
3. `GoalData.activation == GoalActivation::Armed`（`Disarmed` 不得驱动继续）。

`DefaultGraphPlanner::decide_after_turn` 的目标解析顺序（`crates/pony-agent-core/src/agent/planner.rs`）：

1. `context.session_id` 命中 `active_goal_for_session` → 用其 `objective` 作为 run 目标；**仅 armed 才允许 `Continue`**；
2. 未命中 → 回退 `run.goal` + 既有关键词启发（保留 run 级续行行为，不得删除）。

上下文与预算：

```rust
pub struct GraphPlanningContext<'a> {
    pub run: GraphPlanningRunView<'a>,
    pub handoff: &'a GraphTurnHandoff,
    pub session_id: Option<&str>, // wave-1 新增；from_run 用 run.session_id（GraphRun.session_id: Option<String>）
}
```

- 既有 `MAX_GRAPH_AUTO_CONTINUE_STEPS: usize = 8`（`planner.rs:12`）语义改为「**同一用户消息系列内的连续自动续行次数**」；
- 新增 `MAX_CONSECUTIVE_AUTO_CONTINUES: usize = 8`（与 NFR `auto_continue_max` 同值）；
- 两个常量描述**同一预算的读取面**，实现中不得相乘（禁止出现 8×8 复合预算）；用户手动轮次重置该预算且不消耗额度。

问号启发式收紧（回归用例必须覆盖「代码示例含 `?` 但未提问」）：

- `WaitUser` 仅当助手消息 `trim()` 后**最后一个字符**为 `?`/`？`，或命中既有显式短语表；
- 正文中间出现问号**不再**拦截。

---

## 2. C2 持久化（goal-core）

```rust
// crates/pony-agent-core/src/agent/goal.rs
pub fn goal_store_path() -> std::path::PathBuf;      // sidecar 路径真值
pub fn reset_goal_store_for_test();                  // 测试接缝
```

| 事实 | 冻结值 |
|---|---|
| 存储介质 | JSON sidecar（**不新增 sqlite 表**，避免迁移风险） |
| 默认路径 | `default_sqlite_path().parent()/goal_store.json` |
| env 覆盖 | `PONY_AGENT_GOAL_STORE_PATH`（非空即整体替代默认路径；测试与宿主共用同一入口） |
| 写入时机 | `create_goal` / `update_goal` 成功后；`advance_goal_round` 成功后（§3） |
| 原子写 | `*.tmp` + `rename`（`goal_store_atomic_write=true`；禁止原地截断覆写） |
| 读取 | 首次访问懒加载，进程内仍缓存；`reset_goal_store_for_test()` 清空缓存并重新加载 |
| 损坏容错 | 文件解析失败**降级为空**，不得 panic、不得拒绝启动 |
| 重启语义 | 同一路径下重启后 `get_goal` 必须返回完整目标（phase / rounds / revision 不变） |

`GoalData` wire 形状沿用既有冻结（`roundsStarted` / `maxGoalRounds` camelCase；`session_id` / `blocked_reason` snake_case），本波不改字段名。

---

## 3. C3 turn 结束续行驱动（autocontinue）

```rust
// crates/pony-agent-core/src/agent/goal.rs
pub fn build_goal_round_message(objective: &str, round: u32, max_rounds: u32) -> String;
pub fn advance_goal_round(goal_id: &str) -> Result<GoalData, String>; // rounds_started += 1 并持久化
```

**goal-round 文案模板（唯一实现，宿主与 TUI 必须共用此函数，禁止各自拼串）**：

```text
<goal_round round="N" max="M">继续推进目标：{objective}。以工作区与工具结果为准；仅当目标真正达成才 update_goal complete。若本轮仍有未完成步骤，直接继续执行，不要询问用户。</goal_round>
```

- `N = round`（下一轮的序号，从 1 起）、`M = max_rounds`（`GoalData.max_goal_rounds`）；
- 属性顺序固定为 `round` 再 `max`，属性值不加空格。

后端循环（桌面 graph 路径，`HostControlPlane::advance_graph_run`，`crates/pony-agent-core/src/agent/control_plane/mod.rs`）：

1. 每轮结束后按 `GraphDecision` 判定；仅当 `decision.kind == Continue` **且**存在 armed 目标 **且** 系列内连续续行 `< MAX_CONSECUTIVE_AUTO_CONTINUES` 时继续；
2. 继续时构造 goal-round `TurnInput`：`user_message = goal::build_goal_round_message(objective, round, max_rounds)`；
   `display_message` 必须以 `[goal round N]` 起始（N 为同一轮序号）；
3. 达到上限或 `decision != Continue` 立即停止，返回最后一次响应（不得再发一轮）；
4. 每成功发起一轮：`advance_goal_round(goal_id)` → `rounds_started += 1` 并持久化，同时发 `goal://round` 事件（§7）；
5. TUI 路径：同一文案 + 同一预算，在 turn 流结束后由 TUI 循环发起下一条 goal-round 消息；**不得改变用户手动输入路径**。

`rounds_started` 语义（冻结）：

- = **已成功发起的 goal-round 轮次数**（跨重启保留，随目标持久化）；**不是** turn 数、不是 graph step 数、不是用户手动轮次数；
- 递增时机唯一：成功发起一轮 goal-round 时（先持久化再发出下一轮请求）；发起失败不计；
- 用户手动轮次：不递增，且重置系列内连续续行预算（`MAX_CONSECUTIVE_AUTO_CONTINUES` 计数归零，但 `rounds_started` 不回退）；
- `rounds_started >= max_goal_rounds` 时不得再发起 goal-round（由 planner 与循环双重判定，两者口径一致）。

---

## 4. C4 每轮回灌（prompt / context）

`crates/pony-agent-core/src/agent/context.rs::volatile_context_note` 追加两行（位于既有 `Run goal` 行**之前**）：

- `active_goal_for_session(conversation_id)` 命中时：
  `Active goal: {objective} (phase={phase}, round={rounds_started}/{max_goal_rounds}).`
- `todo::todo_list(session)` 非空时：
  `Plan: {completed}/{total} completed; in_progress: {content, …}.`（`in_progress` 列表以 `, ` 连接，空则省略该子句）

注入位置不变：developer 动态层，位于稳定层之后；必须保持 `prompt_layers_are_ordered` 通过（`context.rs` / `crates/pony-agent-core/tests/system_prompt_contract.rs`）。

session 键统一为 `session_context.conversation_id`（= session id）；工具参数缺省 session 时由 runtime 注入当前 session id。

---

## 5. C5 工具与提示词（prompt / tools）

`BASE_SYSTEM_PROMPT` 新增 `Goal & planning policy` 段，位置：工程契约段之后、输出规范段之前。必须包含：

- 何时**主动** `create_goal`（长时 / 多步 / 跨轮目标，即使用户未说“目标”）；
- `update_goal` 仅在真正达成时 `complete`；`pause` / `resume` 需人类请求；`blocked` 需同一阻断**连续 3 轮**；
- 多步任务开始前用 `TodoWrite` 建计划并保持状态；
- 允许**一行**计划状态更新（把 “No restating: never restate the plan” 改为“不复述已展示的工具输出；计划状态变更可一行播报”）。

全文工具名统一为模型可见名 **`TodoWrite`**，禁止残留 `todo_write` 文案；`system_prompt_contract` 测试同步断言 `TodoWrite`。

```rust
// crates/pony-agent-core/src/agent/tools.rs:1034
#[derive(Clone, Debug, Default)]
pub struct ToolExecutionContext {
    pub workspace_root: Option<PathBuf>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>, // wave-1 新增；Default => None
}
```

- `session_id` 为**进程内**字段（该结构未实现 serde，无 wire 形状变更）；由 runtime 在构造上下文时注入当前 session id；
- `TodoWrite` / `TodoList` / `create_goal` / `get_goal` / `update_goal` 在参数未显式给 session 时回退到该字段；
- 参数显式给 session 时以参数为准（不得静默覆盖模型显式值）。

---

## 6. C6 前端可见性（frontend）

```ts
// src/stores/plan.ts 与 src/stores/goal.ts 各一份
export async function refreshForTurn(sessionId: string): Promise<void>;
```

| 契约 | 冻结内容 |
|---|---|
| 键 | `planStore` 与 `goalStore` 以**真实 sessionId** 为键（禁止 `"default"` 兜底冒充） |
| 触发 | `turn:completed` / graph 流结束 → `refreshForTurn(sessionId)`；`GoalBar` 在 `goal://changed` / `goal://round` 后亦刷新 |
| 语义 | `plan.ts` 版 = `list(sessionId)` + `listTodos(sessionId)`；`goal.ts` 版 = `fetchGoal(sessionId)`（空 sessionId 直接返回，不抛错） |
| 空桩期 | 抛 `NotImplemented: wave-1`，不得被生产路径调用（挂载前由 executor 接线） |
| PlanPanel | `HomeSidebar` 的 PlanPanel **常驻**渲染，空态显示“暂无计划”，挂载即拉取（不得以“已有数据”为渲染前提） |
| PlanStore 单例 | `HostControlPlane` 与 runtime 的 `governed_executor` 共享**同一** `PlanStore` 实例（生产构造必须注入；测试注入路径保持） |

前端事件订阅统一走 `src/lib/tauri.ts::safeListen`（非 Tauri 环境返回空 `unlisten`，不得直接 `listen`）。

---

## 7. 宿主事件契约（backend → frontend）

| 事件名 | 载荷（camelCase） | 发射时机 |
|---|---|---|
| `goal://changed` | `{goalId, sessionId, phase, activation, revision, roundsStarted, maxGoalRounds}` | `create_goal` / `update_goal` 成功后；`advance_goal_round` 成功后 |
| `goal://round` | `{goalId, round, maxRounds, status}` | 每成功发起一轮 goal-round（与 `rounds_started += 1` 同一次成功） |

- `status`：wave-1 唯一取值 `"started"`；其他取值（`"completed"`/`"stopped"`）属后续波次，**需要重锚契约**后才能发射与断言。
- `round` 与 `rounds_started` 同步：`round == GoalData.rounds_started`（递增后）。
- 发射失败只记诊断，不得中断 turn（与既有 `tauri_adapter.rs::emit` 的容错一致）。
- 命名空间前缀 `goal://` 为冻结字面量，禁止改写为 `goal:` / `goal.changed` 等变体。

---

## 8. 环境变量与路径真值

| 名称 | 语义 | 缺省 |
|---|---|---|
| `PONY_AGENT_GOAL_STORE_PATH` | goal JSON sidecar 绝对/相对路径，非空即覆盖 | `default_sqlite_path().parent()/goal_store.json` |

---

## 9. 与 NFR 基线的对应

| NFR 字段 | 值 | 契约锚点 | 校验 |
|---|---|---|---|
| `auto_continue_max` | `8` | §3 `MAX_CONSECUTIVE_AUTO_CONTINUES` | `.dev-team/nfr-baseline.schema.json`（const 8） |
| `goal_store_atomic_write` | `true` | §2 `*.tmp` + rename | schema（const true） |
| `external_call_timeout_ms` | `3000`（上限） | 外部调用超时 | schema `maximum: 3000` |
| `retry.backoff_ms` | `500`（指数退避起点） | wave-1 新增别名，与 `retry.backoff_cap_ms=1000`（上限）配对 | schema `minimum: 0`，additive-only |
| `logging.format` / `logging.trace_id_required` / `logging.trace_id_field` | `"json"` / `true` / `"trace_id"` | 结构化日志面（PA-119 债务） | schema；`logging_compliance_claimed=false` 恒真防护不变 |
| `concurrency_lock` | `"mutex"` | 既有锁纪律 | `nfr-lint.py` 全域校验 |
| `resource_release` | 既有 7 条（未改） | 本波不新增释放点（sidecar 为短生命周期句柄） | `nfr-lint.py` |

机械校验：`python3 .dev-team/nfr-lint.py --instance .dev-team/nfr-baseline.json`（schema + 未知字段拒绝 + 锁/日志硬约束），`--self-test` 21 例反例。

---

## 10. 契约冲突与勘误登记（protocol-agent，wave-1）

1. **NFR 字段名映射**：任务文本写作 `retry{max_attempts,backoff_ms}` 与 `logging.trace_id_field`，而冻结 schema 既有必填为 `retry{max_attempts,backoff_cap_ms}` 与 `logging.trace_id_required`。处置：保留既有必填字段（不动既有契约），以 **additive-only** 方式把 `backoff_ms`、`trace_id_field` 加入 schema 并写入基线（`backoff_ms` 与 `backoff_cap_ms` 同时声明，前者为退避起点、后者为上限）。
2. **`ADR 0007` 引用勘误**：契约 C4 括号内标注的 `ADR 0007` 实为「缓存命中提升为一等产品指标」，与提示词分层无关；`prompt_layers_are_ordered` 的真实载体是 `agent/context.rs` 与 `crates/pony-agent-core/tests/system_prompt_contract.rs`。本条仅登记引用勘误，**不改变 C4 语义**。
3. **`.dev-team/state.json` 覆写**：wave-1 交付要求该文件为 `{baseline_commit, waves[], counters}` 结构，而基线内该文件为 wave 1..41 的历史账本（1722 行）。处置：原账本原样归档至 `.dev-team/state-archive-through-wave41.json`（未删改任何历史事实），新 `state.json` 仅承载本次波次编排；归档文件由 Lead 决定是否纳入提交。
4. **TS `noUnusedParameters`**：冻结签名与 `tsconfig.json` 冲突，见 §0 第 2 条（`void sessionId;`），不改变签名与抛错文案。
5. **版本四处同步既有偏差**（非本波引入）：`crates/pony-agent-core/Cargo.toml` 为 `0.1.127`，`package.json` 为 `0.1.153`，基线 commit 处即如此；本波不触碰，仅登记以免 green 阶段被误判为本波回归。
