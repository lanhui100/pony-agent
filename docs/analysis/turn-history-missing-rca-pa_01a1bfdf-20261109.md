# TUI 新轮次缺失历史轮次 —— 根因诊断（pa_01a1bfdf · 2026-11-09 18:35–19:01）

> 数据源：Tempo（`observability/tempo`，OTLP 由 edge 转发）+ 本机 TUI 运行日志 `~/.pony-agent/tui-logs/tui-runtime.log` + Rust 源码。
> 取证方式：dev team 四路并行（trace 取证 / 上下文链路测绘 / wave-pa174 回归审查 / 只读机制证明），Lead 交叉比对与抽验。
> 代码基线：HEAD `c4a5bd81`（0.1.186）+ 未提交工作区改动；**运行中的二进制 = 工作区构建**（`crates/pony-agent-core/Cargo.toml` 版本号仍为 0.1.159，故 `pony --version` 不能用于判定源码版本）。

## 0. 结论速览

用户报告成立：**今日 18:35 / 18:58 / 19:01 同一会话三轮请求全部只带 6 条消息、轮次序号恒为 0，历史零跨越**。

但**根因不在上下文组装、也不在 wave-pa174**。根因是**第二个 TUI 实例抢不到 `sessions.db.lock` 后静默降级只读**，导致轮次提交（两阶段暂存提交）在持久化环节整轮失败 → 内存 live 会话从未写入该轮 user/assistant → 下一轮组装时历史就是空的。

一句话链：**双实例争锁 → 只读降级 → `append_turn` 暂存提交整轮回滚 → 内存历史恒空 → 每轮请求 = 首轮基线（6 条）+ 轮次序号恒 0。**

## 1. Trace 证据（症状确证）

会话 `pa_01a1bfdf-2fba-7ff3-89b4-8dfcff5a4fcd`，同一 TUI 进程内连续三轮：

| # | traceID | 本地起始 | `agent.turn.id` | `agent.turn.sequence` | `llm.request.messages` | 首包 input_tokens | phase |
|---|---|---|---|---|---|---|---:|---|
| 1 | `93e6011f…` | 18:35:22 | turn-3a76f7c2 | **0** | **6**（全部 16 次 llm.chat） | 14760 | 无终态 |
| 2 | `39072f95…` | 18:58:10 | turn-ede16cd5 | **0** | **6**（全部 30 次） | 14543 | 无终态 |
| 3 | `7cbece28…` | 19:01:18 | turn-b3caead7 | **0** | **6**（全部 7 次） | 14539 | cancelled |

- `llm.request.messages` 语义 = 实际发给 provider 的消息条数（`turn_flow.rs:3023` `llm_request_shape`；属性定义 `otel/mod.rs:157`、写入 `otel/mod.rs:1620`）。
- 正常基线（有历史）应逐轮 +2：同日 0.1.158 构建同会话 4 轮为 `8/10/12/14`，且 turn≥2 前缀缓存命中 0.95+；另一正常会话 `pa_01a1bfc9` 为 `6→8→10`。
- `agent.turn.sequence` 取自「已登记 turn trace 条数」（`session/store.rs:1777`）⇒ 恒 0 与「历史恒空」同源。
- 反证检查全部命中反向：条数不增长、input_tokens 零增长（14543→14539）、无跨轮前缀缓存累积。

## 2. 根因链（每步带物理收据）

**① 两个 TUI 实例争同一会话库的单实例写锁**
- pid `3105690`（18:30 启动）持有 `~/.local/share/PonyAgent/sessions.db.lock`（`/proc/3105690/fd/14`）。
- pid `3131779`（18:33 启动，`/home/dm/.local/bin/pony`）**没有 lock fd**（只剩 db / db-wal / db-shm 三个只读连接）。

**② 抢不到锁 ⇒ 静默降级只读（含明确日志）**
- `crates/pony-agent-core/src/agent/sqlite_session.rs:295-306`：`acquire_single_instance_lock` 返回 `None` ⇒ `PRAGMA query_only=ON`，不 panic、不退出。
- 日志收据（整文件口径，日志无时间戳）：`single_instance_lock_denied` 239 行（其中 218 行含「本实例降级只读」）；`attempt to write a readonly database` 5025 次；`event flush failed` 43 次。

**③ 轮次提交走「两阶段暂存提交」，持久化失败即整轮回滚（关键机制）**
- `crates/pony-agent-core/src/agent/session/store.rs:540` 注释：*两阶段暂存提交（Staging Commit）：在克隆状态上计算新 turn，持久化成功后才提交到 live*。
- `store.rs:641`：`persist_commands_batch`；`:643` 为 `Failed`/`StaleEpoch` ⇒ `return Err(...)`；
- `store.rs:653`：**live 内存写入在这之后**（`:652` 注释「持久化成功，原子写入主内存」）。⇒ 失败轮的 user/assistant 消息**永不进入内存 history**。
- 日志收据：`append_turn error: Backend persistence failure: append_turn batch persist failed: Failed`、`event flush failed … pre-materialization event flush failed (buffer retained)`、`save_session_state_to_backend 失败：commit_boundary_failed（数据可能丢失）`。

**④ 下一轮组装取不到历史**
- TUI 每轮提交 `history: Vec::new()`（`bin/tui.rs:3387`，历史只能来自常驻 `SessionStore`，`bin/tui.rs:3469`）；
- 新一轮：`snapshot_at`（`store.rs:381-407`）→ `retrieve_context_state` → `build_request`；历史为空 ⇒ 请求 = 固定前言 + 当轮 user ≈ **6 条**；
- 同轮 initial / tool_followup 条数恒定是**观测口径**（两者度量同一 base `planning_request`，`turn_flow.rs:3055-3062` vs `:3259-3266`），不是丢历史证据。

**⑤ 三轮 trace 的 turn id 全部落在只读失败日志窗口内**（抽验）
- `turn:start id=turn-3a76f7c2…`（日志 3759625）、`turn-ede16cd5…`（3810977）、`turn-b3caead7…`（3840690）；
- 三处 `turn:start` 前 12 行内均有 `single_instance_lock_denied` + `commit_boundary_failed: attempt to write a readonly database`。
- 失败会话在磁盘上**没有任何 durable 事件目录**（`session-logs/…/pa_01a1bfdf-*` 不存在），故"从事件重建历史"也无源可依。

## 3. 为什么失败没被拦住

- 18:29 安装的 `/home/dm/.local/bin/pony` **不含** fail-closed 启动预检：`strings` 检索 `拒绝启动` / `startup_permission` / `enforce_startup_permission` 全部 ABSENT。
- 该守卫正是工作区里**未提交**的新文件 `crates/pony-agent-core/src/bin/tui/persistence_status.rs`（mtime 19:03:07，`git status` = `??`），其 `enforce_startup_permission`（tui.rs:11416）会在「锁被别的进程占着」时打印诊断并 `exit(1)`。
- ⇒ 事故窗口内运行的构建没有这道门，第二实例照常进入 TUI、照常接收输入、每轮在提交边界静默失败。

## 4. 排除项与独立隐患（不要与本根因混判）

| 项 | 判定 | 证据 |
|---|---|---|
| wave-pa174「从 durable events 重建历史」是回归？ | **否** | 该波次对历史只做加法；血缘跳过分支在真实库 0 命中；运行日志 `history_rebuilt_from_turn_events: 24` 正常触发 |
| 启动自愈预算（1200ms + 进程级 `OnceLock`）是本次根因？ | **否**（但确是独立隐患） | 该逻辑属**未提交**改动（`session/startup_repair.rs:41`）；运行日志中 `budget_exhausted` 仅出现在工具入参文本里，运行期 0 命中；只读跑测可稳定复现 `history=[]`（`PONY_STARTUP_SELF_HEAL_BUDGET_MS=0` + `p1_loader` FAIL） |
| native transcript 只回放最近 24 条？ | 机制存在，但不是本次 | `context.rs:115/829/1610` + `provider/mod.rs:4953`（native 非空即整份替代 input）；本次异常轮 messages=6 远小于 24，且 mode=`initial` 的 6 条与首轮同形 |
| 内存历史硬窗 24 条？ | 非本次 | `session/types.rs:17` + `store.rs:556`；表现为"近因丢失"，不会全空 |
| 压缩 / 80% 窗口截断 / 1:1 重试降级？ | 非本次 | 见 `/tmp/diag/context-code.md` §2 M9–M11 |

**独立隐患（建议单独立项）**：持久化失败时「丢弃本轮内存历史」是 fail-open 设计——数据完整性问题被降级成"上下文静默变短"，用户与模型都无感知。即使修好锁预检，I/O 抖动仍会重现同一症状。

## 5. 复现与验证

1. 事故复现（无需改码）：开两个 `pony`（同 `HOME`）→ 第二个实例日志出现 `single_instance_lock_denied`；在其内连续两轮对话 → trace 中 `llm.request.messages` 不增长、`agent.turn.sequence` 恒 0。
2. 反向验证：关闭第一个实例后重开 → 历史恢复（`messages` 逐轮 +2、日志出现 `history_rebuilt_from_turn_events`）。
3. 预算隐患复核（只读跑测）：
   `PONY_STARTUP_SELF_HEAL_BUDGET_MS=0 cargo test --offline -p pony-agent-core --test pa174_history_wiring_and_followup_contract p1_loader -- --nocapture`

## 6. 修复建议（按杠杆排序）

- 说明：本节所有 `path:line` 以 2026-11-09 19:2x 工作区为准；`session/store.rs` 正被并行会话编辑，行锚可能漂移，定位请以函数名（`append_turn_fallible` / `build_turn_input`）为准。
- **P0**：落地 fail-closed 启动预检（`persistence_status.rs` 已在工作区），并在只读降级发生时于 TUI 顶部常驻告警，杜绝"静默只读"。
- **P0**：锁归属可视化——退出/启动诊断显示持有者 pid、命令行、启动时间（守卫文案已有，缺的是让旧构建也生效）。
- **P1**：把「持久化失败」与「历史为空」解耦：失败时要么明确 fail-loud（本轮中止并提示），要么保留内存 live 历史并标记"未落库"，而不是静默回滚整轮。
- **P1**：启动自愈预算改为**每次 `load_store` 起算**（不要进程级 `OnceLock` 冻结），跳过必须可观测（UI/stderr 双写）。
- **P2**：补机器契约——目前全仓没有「新一轮请求必须携带既有历史轮次」的断言（`[UNVERIFIED]`：`docs/spec-runtime-history-consistency.md` 描述的是桌面前端）。建议在 `pa174_history_wiring_and_followup_contract` 增加「同会话 turn N≥2 的 `llm.request.messages` 必须 ≥ turn1+2」类 trace/请求级断言。

## 7. 证据索引与盲区

- trace 原始体：`/tmp/diag/tr/*.json`（82 条）、`/tmp/diag/{trace-evidence,context-code,history-git}.md`、`/tmp/diag/rca-review.json`（L3 复核：verdict=PASS，9 条 ADVISORY 全为引用卫生，无 BLOCKING）
- 日志锚点：`~/.pony-agent/tui-logs/tui-runtime.log`（3759625 / 3810977 / 3840690 / 3508878）
- 进程证据：`/proc/{3105690,3131779}/fd`（lock fd 有无）、二进制 `strings` 检索
- 盲区：① 18:30 之前那批失败会话（15:34 / 16:26）由哪个进程产生未能定位（进程已退出）；② `llm.request.messages` 只有条数、无内容级 payload（`gen_ai.prompt` 在 80 条 trace 中 0 命中），无法做"文本片段包含"级验证；③ 本报告未改动任何业务代码。
