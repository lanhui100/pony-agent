# Agent Note: 编排层团队/子代理进程内状态收敛为单一持有者 `TeamState`

Status: implemented

## Problem

`crates/pony-agent-core/src/agent/orchestration.rs`（2,497 行）此前把团队/子代理的进程内状态
分散在 **11 个独立 `static`**：`DRIVING`、`INTERRUPTED`、`TEAM_TASKS`、`TEAMMATES`、
`TEAMMATE_INBOXES`、`TEAMMATE_PROVIDERS`、`TEAMMATE_SESSIONS`、`TEAMMATE_HOST_SESSIONS`、
`TEAM_TASK_HOST_SESSIONS`、`TEAMMATE_TRACEPARENTS`、`SUBAGENT_RUNS`
（审计源 `backlog:B1795831358484`，`ENTROPY_REPORT.md#hotspots`）。

三个具体病灶：

1. **归属不可见**：读代码无法回答「这个进程里到底有哪些团队状态、谁在改它们」。11 份声明各自
   散落在不同小节，新增一张表只需随手加一行 `static`，没有地方能强制它被登记。
2. **锁中毒策略三种语义并存**：同一类表上同时存在 `.lock().unwrap()`（panic）、`.lock().ok()`
   （静默把「中毒」当成「表不存在」，读出错误的默认值）、
   `.unwrap_or_else(PoisonError::into_inner)`（恢复）。这一次 panic 在三条路径上给出三种结果，
   其中 `.ok()` 那条会把「进程内还有队友」误读成「没有任何队友」。
3. **无法整体复位**：11 张表没有统一清零点，测试隔离只能靠外部进程隔离（nextest
   `--no-fail-fast` 逐进程）绕开互染。

## Decision

**1) 新建 `TeamState` 作为唯一持有者**

11 个 `static` 收敛为 `pub(crate) struct TeamState` 的 11 个私有字段，进程内只有
`static TEAM_STATE: TeamState = TeamState::new()` 一个实例（`const fn new()`，无运行期初始化）。
字段一律 `Mutex<Option<...>>`（保留「尚未建表」与「空表」的既有区分，不改语义）。

**2) 锁中毒策略收敛到唯一实现点**

`fn recover<T>(slot: &Mutex<T>) -> MutexGuard<'_, T>` 是全模块唯一的取锁入口，中毒一律
`PoisonError::into_inner` 恢复。理由：这些表是**进程内缓存/可见性状态**，不是持久化真值；
恢复后继续比让宿主里所有后续团队调用 panic 更安全。取锁侧的宽松解析一并清零，实测口径
（`git show e8907724 -- .../orchestration.rs | grep '^-'` 逐行统计）：
`.unwrap()` **29 处**（20 处单行 `.lock().unwrap()` + 3 处跨行 `.lock()` 换行 `.unwrap()` +
6 处链式 `.as_mut()/.as_ref().unwrap()`）、`.ok()?` **3 处**、`.ok()`（不带 `?`）**5 处**。
对应 `scripts/clippy-baseline.json` 中 `clippy::unwrap_used @ orchestration.rs` 由 **28 → 0**
（clippy 按 lint 触发点计数，与上列源码调用点数因跨行/链式写法略有差异；三平台分册同步删除该条，
该文件无 `#[cfg(windows/unix)]`，三册条目集合逐字相同）。

**3) 锁粒度刻意保持字段级，不合并成大锁**

`TeamState` 只收敛**归属与策略**，不合并锁。原因：现有调用路径普遍「取 A 锁、释放、再取 B 锁」，
且 `is_driving()` 会在取 `teammates` 锁之前被调用（`interrupt_agent_as` 与 `list_agents` 的
既有顺序依赖）。合并成一把锁会把「先 is_driving 再 teammates」变成自锁死
（`std::sync::Mutex` 不可重入）。

**4) 新增 `#[cfg(test)] TeamState::reset_fields(&mut self)`（实例方法，非全局函数）**

复位语义是整值替换 `*self = TeamState::new()`，而不是逐字段置 `None`：逐字段写法要求未来
每加一张表都记得补一行，漏一行的后果是同进程测试互染。挂在**实例**上而非全局：全局清零函数
一旦被某条用例调用，就会与 `cargo test --lib`（同进程多线程）里并行运行的其他用例互染；
生产路径的 `static TEAM_STATE` 是不可变借用，天然不暴露该能力。
配一条自证用例（`team_state_holder_tests`）：用局部实例写满全部表再复位，断言全部回到 `None`。

**5) 孤儿文档并入字段注释**

原 11 个 `static` 上的语义说明（单飞语义、C10-A2 中断标记、信箱双端语义、provider 继承、
子会话可见性、跨会话归属过滤、traceparent 快照）逐条迁到 `TeamState` 对应字段的 rustdoc，
`clippy::empty_line_after_doc_comments` 新增 6 → 0（不留悬空 doc）。

## Alternatives considered

- **方案 A：11 个 `static` 收敛为单一 `TeamState` + 字段级锁 + 统一 recover（采纳）**。
  Deletion Test：删掉 `TeamState` 这层，则「进程内有几张表」「中毒怎么处理」「测试怎么复位」
  三个判断重新扩散到 30+ 处调用点；留着它复杂度收在 40 行结构体里，故保留。
- **方案 B（任务描述的推荐表述）：收敛为 `TeamOrchestrator` **实例**并沿调用链注入**。落选（本轮）**：
  免费函数 `orchestration::send_message` 等的调用方横跨 25 个文件（`pony-tui`、`pony-tools`、
  `pony-agent-core` 测试），实例化必须改动 public API 与 `OrchestrationToolBridge` 桥接；
  而桥接反转正是待拍板 ADR `proposed/architecture/2026-11-28-protocol-decouple-and-bridge-inversion.md`
  的范围（`backlog:B1795831358054`，未获批）。在未批复的架构前提下先改 public API 会制造
  「新旧并存」的过渡态，违背 Deletion over Accumulation。本轮只做**可逆、纯内部**的状态归属收敛，
  实例化待桥接反转批准后作为其自然结果推进（见 Consequences）。
- **方案 C：合并成一把大锁（`Mutex<TeamStateData>`）**。落选：如 Decision §3 所述，
  「先 is_driving 再 teammates」的既有顺序会让 `std::sync::Mutex` 自锁死；改动锁序等于改并发语义，
  超出本任务「清 unwrap 热点、清全局 Mutex」的范围，且无测试能证明等价。
- **方案 D：仅加 `#[allow(clippy::unwrap_used)]` 进基线**。落选：任务红线明写
  「clippy unwrap_used 计数只降不升」；加 allow 是把债记进基线而不处理中毒语义分裂。

## Consequences

- 新增一张团队状态表：改 `TeamState` 一个结构体（字段 + 访问器 + `reset_for_tests` 一行），
  不再有「随手加 static」的路径。这是有意的摩擦，代价换来状态清单唯一可见。
- `.lock().ok()?` 的 3 处（及 `.lock().ok().and_then(...)` 的 5 处）改为 `recover` 后，**中毒时会继续使用可能半更新的表**而非返回 `None`/`false`/`0`。
  这是语义变化：此前「中毒 ⇒ 视为无表」，现为「中毒 ⇒ 恢复并继续」。判断依据是这些表都是
  内存缓存（队友名册、信箱、traceparent 快照），半更新最多导致一次可见性信息缺失，
  而「视为无表」会把团队健在误读成无队友。**靠 review**：中毒路径无自动化测试
  （构造中毒需要持锁 panic，代价高于收益）。
- 计数口径：任务描述写「12 处 global static Mutex」，实测本文件内为 **11 个**；第 12 处是
  `crates/pony-tools/src/tools/mod.rs` 的 `ORCHESTRATION_BRIDGE`（`OnceLock<Arc<dyn ...>>`），
  属桥接反转范围（待拍板 ADR `protocol-decouple-and-bridge-inversion`），本轮未动。
- 剩余热点按 crate 分波次（Task 的第二半）：`dispatcher.rs` 32、`jobs.rs` 25、`terminal.rs` 21、
  `ponysentry/sanitizer.rs` 12、`process.rs` 11 处 `unwrap/expect`。本轮只清零了
  `orchestration.rs` 自身（28 → 0）；其余波及 `pony-tools` 与 `pony-observability`，
  每处都要判断「失败是应该传播成错误码还是应当 panic」，需独立提交与独立 review，
  已登记为后继任务承接。
