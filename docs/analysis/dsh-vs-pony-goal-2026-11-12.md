# DSH vs Pony：goal（会话目标/长期目标）实现对比与优化清单

- 对比对象：DeepSeek Harness `de5d733a18`（`/home/dm/deepseek-harness`，2026-10-31 tag dsh-v0.2.1-rc.5）
- 对比对象：Pony `/home/dm/pony-agent`（工作区当前 HEAD，无 git 仓库，按文件 mtime 2026-11-12）
- 产出日期：2026-11-12
- 纪律：每条结论都带 `文件:行号`；未在源码中找到的能力一律写"无证据"，不按印象推断。

## 0. 结论摘要（先看这里）

1. **DSH 有完整 goal 子系统，不是"用别的机制替代 goal"**。它是 5 个包的分层实现：`packages/goal/goal`（领域 + 投影）、`goal-round-driver`（同会话续轮驱动）、`tool-goal`（模型工具 + 权限门）、`command-goal`（`/goal` 斜杠命令）、`packages/client/ui-goal`（GoalBar 等 4 个 UI 挂点）。
   - 另一条并行机制 `tool-ralph`（每轮起**全新子代理**）与 goal（同会话续轮）是**互补而非替代**，源码里明确分工：`packages/experimental/tool-ralph/src/index.ts:408`（"Use same-session goal tools for ordinary long-running objectives…"）。
2. **Pony 的功能面基本齐全**（工具三件套、`/goal` 命令、相位/激活、轮次预算、TUI 续轮循环、GoalBar），差距不在"有没有"，而在**领域层不设防**与**驱动层三处实现不一致**。
3. **P0 三条**（详见 §3）：
   - 领域层无状态迁移守卫，`resume` 能把 `completed` 目标复活为 `active`；
   - 领域层 `create_goal` 无"已存在"守卫，模型工具可静默覆盖进行中目标；
   - headless/graph 续轮路径用 `advance_goal_round`（失败轮也烧预算）且**无连续失败止损**，绕过已冻结的 PA-171 C2 契约。
4. **P1 六条**，其中两条属"看起来在工作、实际是死路径"：前端订阅的 `goal://changed` / `goal://round` 在 Rust 侧**没有任何发射点**；goal-round 消息靠**中文文案前缀嗅探**识别（`继续推进目标：`），改文案即破契约。
5. Pony 有 3 处**明显强于 DSH**，不要被"对齐 DSH"的直觉带偏：连续失败止损（2 轮）、单会话单续轮循环租约、失败轮不烧预算（TUI 路径）。这三项写进 §4「已覆盖、无需动」。

---

## 1. 两侧设计概览

### 1.1 DSH

| 维度 | 实现 | 证据 |
| --- | --- | --- |
| 真源 | 会话日志事件 `goal/change`（事件溯源），`goal` projection 由 fold 派生 | `packages/core/session/src/known-event-types.ts:41`；`packages/goal/goal/src/index.ts:162-169` |
| 事件模型 | 每次变更写**全量快照**（CAS revision+1）或 `clear` 墓碑；`decodeGoalChange` 严格校验字段集合 | `packages/goal/goal/src/fold.ts:134-172`；校验迁移矩阵 `fold.ts:187-268` |
| 轮次计数真源 | **只认已准入的 goal 轮消息**：`user/message` 且 `source.kind === 'goal'`，且 `round === roundsStarted + 1` 才递增 | `packages/goal/goal/src/fold.ts:313-331` |
| 激活语义 | `phase`（durable）与 `activation`（**进程本地**）分离；进程创建 agent 即 `disarmed` | `packages/goal/goal/src/types.ts:44-49,71-72`；`index.ts:255-257` |
| 续轮驱动 | `goal-round-driver`：`agent/status=idle` + 无竞争输入 + subagent 静默 + 落盘 checkpoint 后，`agent.followup()` 投递下一轮 | `packages/goal/goal-round-driver/src/index.ts:106-117,147-221` |
| 轮次提示 | 结构化 message source（`{kind:'goal', goalId, revision, round}`）+ `<goal_round>` 文案 | `goal-round-driver/src/index.ts:190-195`；`src/prompt.ts:12-26` |
| 上限 | 单一 `maxGoalRounds`，默认 **256** | `index.ts:243-245` |
| 超限行为 | **durable block**，理由 `round-limit` | `goal-round-driver/src/index.ts:183-186` |
| 终态 | `active/paused/blocked/complete`，`complete` 为终态；`clear` 只留墓碑 | `types.ts:44-49`；`index.ts:432-447` |
| 权限门 | `requireDirectHuman`（create/edit/pause/resume）；`complete/blocked` 额外允许"当前 goal 轮"；blocked 硬阈值 3 轮 | `tool-goal/src/authority.ts:80-118`；`tool-goal/src/index.ts:224,263,271,283,305-312` |
| 系统提示 | 独立 section `tool:goal`，order `TOOL_GOAL=2400`，阈值由配置注入文案 | `tool-goal/src/index.ts:189-193`；`packages/core/system-prompt/src/index.ts:151` |
| UI 推送 | 领域事件 `goal/changed` + projection `goal` 下发；UI 用 `useProjection('goal')` + 进程本地 activation 覆盖 | `index.ts:609-625`；`client/ui-goal/src/client/GoalBar.tsx:187-198` |

### 1.2 Pony

| 维度 | 实现 | 证据 |
| --- | --- | --- |
| 真源 | **进程内 Map + JSON sidecar** `goal_store.json`（key = session_id，每会话 1 个目标） | `crates/pony-agent-core/src/agent/goal.rs:89-90,97-119,137-170` |
| 并发写 | `flock` + `*.tmp` + rename，跨进程 merge 为**后写覆盖（per-session last-write-wins）** | `goal.rs:200-262` |
| 事件模型 | 无事件；变更即整条记录落盘；`clear` 为删除（无墓碑） | `goal.rs:698-720` |
| 轮次计数 | `rounds_started` 由宿主在**开轮/成功收官后** `+1`；TUI 成功才推进，graph 路径无条件推进 | `goal.rs:373-414,537-546`；`bin/tui.rs:8330`；`control_plane/mod.rs:1947` |
| 激活语义 | `activation` 与 `phase` **一起持久化**，`Armed` 落盘 | `goal.rs:29-40,579` |
| 续轮驱动 | **两套**：TUI `run_goal_rounds`（含租约/止损/可见停止）与 `control_plane` graph/planner 循环（仅 8 轮计数） | `bin/tui.rs:8203-8330`；`control_plane/mod.rs:1934-1963,2758-2787` |
| 轮次提示 | 中文纯文本 `<goal_round round="N" max="M">继续推进目标：…</goal_round>`，无结构化 source；识别靠**文案前缀嗅探** | `goal.rs:325-334`；`goal.rs:337-362` |
| 上限 | `max_goal_rounds` 默认 **50**；另有隐藏的"每线程 8 次自动续轮" | `goal.rs:582`；`planner.rs:23`；`bin/tui.rs:8277-8297` |
| 超限行为 | TUI：可见状态行停止（不 block）；control_plane：静默 `return Ok(response)` | `goal.rs:510-535,476-506`；`control_plane/mod.rs:1939-1944` |
| 终态 | `active/paused/blocked/completed`（注意 Pony 用 `completed`，DSH 用 `complete`） | `goal.rs:642-661`；`src/stores/goal.ts:18` |
| 权限门 | **无**：`execute_update_goal` 直接落库，无 direct-human / goal-round 校验，无 blocked 阈值 | `tools/orchestration.rs:517-543`；`goal.rs:655-663` |
| 系统提示 | 约束写在通用指令块文本里（软约束），另注入 `Active goal: …` 一行 | `context.rs:34`；`context.rs:1159-1168` |
| UI 推送 | Tauri IPC 拉取（`goal_get`）+ 订阅 `goal://changed` / `goal://round`（**后端无发射点**） | `src-tauri/src/lib.rs:408-472,1535-1538`；`src/components/GoalBar.vue:56-57,88-101` |

---

## 2. 逐项对比表

判定口径：`等价` / `差异（无实质影响）` / `Pony 更强` / `DSH 更强`。

| # | 维度 | DSH | Pony | 判定 |
| --- | --- | --- | --- | --- |
| 1 | 状态真源 | 会话日志 `goal/change`，fold 派生投影（`fold.ts:313-349`） | JSON sidecar，key=session（`goal.rs:89-119`） | 差异 |
| 2 | 崩溃/重启后一致性 | 日志重放自愈；非法事件记 `failure` 而非静默（`index.ts:146-159`） | sidecar 读坏则整库降级只读（`goal.rs:147-170,224-229`） | DSH 更强 |
| 3 | 多人/多端并发写 | 单写者事件日志，无合并问题 | 跨进程 merge 后写覆盖，可能回退另一端的 pause（`goal.rs:237-253`） | DSH 更强 |
| 4 | CAS 语义 | `expectCurrent` 校验 id+revision（`index.ts:456-466`） | 同：id + revision 校验（`goal.rs:629-640`） | 等价 |
| 5 | 每会话目标数 | 1（新建前必须 complete/clear，`index.ts:306-309`） | 1，但**可被静默覆盖**（`goal.rs:584-590`） | DSH 更强 |
| 6 | 相位集合 | `active/paused/blocked/complete`（`types.ts:44-49`） | `active/paused/blocked/completed`（`goal.rs:642-661`） | 等价（命名不统一） |
| 7 | 迁移守卫 | 显式 allowed 矩阵：pause←active、resume←{active,paused,blocked}、complete←{active,paused,blocked}、block←active（`index.ts:351-353,365-381,392-399,413-415`） | **无守卫**：任意相位可直接改成 paused/active/completed/blocked（`goal.rs:641-661`） | DSH 更强 |
| 8 | resume 与预算 | 预算耗尽时拒绝 resume 并提示先加 `maxGoalRounds`（`index.ts:375-379`） | 允许无穷 resume；仅靠循环内判据停（`goal.rs:645-650`） | DSH 更强 |
| 9 | blocked 理由结构 | `{code: lower-kebab, message}`，回放时严格校验（`types.ts:51-57`；`fold.ts:73-85`） | 自由文本 `Option<String>`（`goal.rs:38`） | DSH 更强 |
| 10 | blocked 门槛 | 硬阈值：goal 轮内 `roundsStarted < 3` 直接拒（`tool-goal/src/index.ts:305-312`） | 仅提示词软约束"至少 3 轮"（`context.rs:34`），代码无校验 | DSH 更强 |
| 11 | 工具变更权限 | create/edit/pause/resume 要求 direct-human；complete/blocked 另接受"当前 goal 轮"（`authority.ts:100-118`） | 无任何权限门（`tools/orchestration.rs:517-543`） | DSH 更强 |
| 12 | 模型 resume paused | **显式禁止**模型自行 resume 被人类 pause 的目标（`tool-goal/src/index.ts:279-285`） | 允许（`goal.rs:645-650` + schema 无限制，`tools/schema.rs:936-939`） | DSH 更强 |
| 13 | 激活持久性 | 进程本地，进程启动/加载即 disarmed（`types.ts:71-72`；`index.ts:255-257`；driver 加载时对已有 agent 全量 disarm `index.ts:448-452`） | 持久化 `Armed`，重启后直接满足续轮条件（`goal.rs:579,309-318`） | DSH 更强（安全性） |
| 14 | 续轮触发条件 | idle + 无竞争输入 + 子代理静默 + 落盘 checkpoint + 精确 revision 复核（`driver:106-117,147-221,380-395`） | TUI：租约 + 待答请求让位 + 止损判据（`tui.rs:8229-8300`）；graph：仅 8 轮计数（`control_plane/mod.rs:1939-1944`） | 差异（DSH 更严） |
| 15 | 轮次消息可辨识性 | 结构化 source `{kind:'goal',goalId,revision,round}`（`driver:190-195`；`fold.ts:313-331`） | 中文文案前缀嗅探（`goal.rs:325-362`） | DSH 更强 |
| 16 | 轮次上限默认 | 256（`index.ts:243-245`） | 50（`goal.rs:582`） | 差异（口味） |
| 17 | 上限被隐藏值截断 | 无；maxGoalRounds 是唯一上限（`driver:183-186`） | 有：每线程 8 次（`planner.rs:23`，`tui.rs:8277-8297`）→ 用户设 50 实际只跑 8 | DSH 更强 |
| 18 | 超限行为 | durable block + 理由 `round-limit`（`driver:183-186`） | TUI 可见状态行停止、不 block（`goal.rs:476-506`）；graph 静默返回（`control_plane/mod.rs:1939-1944`） | DSH 更强 |
| 19 | 失败轮是否烧预算 | 只有"已准入轮"计数，天然不烧（`fold.ts:329-331`） | TUI：Correct（`goal.rs:537-546`）；graph：`advance_goal_round` 无条件 +1（`control_plane/mod.rs:1947`） | 差异（Pony 内部不一致） |
| 20 | 连续失败止损 | 无等价判据；仅 `turn/end` 为 max-tokens/aborted 时 disarm（`driver:355-367`） | `MAX_CONSECUTIVE_GOAL_ROUND_FAILURES = 2` + 判据顺序冻结（`goal.rs:416-420,510-535`） | **Pony 更强** |
| 21 | 单会话单续轮循环 | 靠 attempt 状态机 + `readyToDrive` 串行化（`driver:147-152,224-248`） | 显式按 session 的 RAII 租约（`goal_lease.rs:19-74`） | **Pony 更强**（语义更直白） |
| 22 | 暂停的实时抢占 | 人类 pause 时 cancel 当前轮（`driver:296-312`） | pause 后靠下一轮判据生效；已启动轮不打断 | DSH 更强 |
| 23 | 停止原因可见性 | block/耗尽都有 durable 理由 + UI 可见 | TUI 9 种 `visible_line`（`goal.rs:476-506`）；graph 静默 | 差异 |
| 24 | 终态收口消息 | complete/blocked 后注入 `<goal_complete>/<goal_blocked>` 引导模型写给用户的收尾（`tool-goal/src/index.ts:318-330`；`wrapup.ts:17-40`） | 无 | DSH 更强 |
| 25 | `clear` 语义 | 保留墓碑 `{id, revision+1}` + 审计（`index.ts:432-447`） | 直接删除记录；`seenGoalIds` 之类审计不存在（`goal.rs:698-720`） | DSH 更强（审计） |
| 26 | `/goal` 语法 | `[<objective>|clear|edit <objective>|pause|resume]`（`command-goal/src/index.ts:16,35-45`） | `[<目标>|edit <目标>|pause|resume|complete|clear]`（`goal_command.rs:15,50-82`） | 等价（Pony 多 `complete`） |
| 27 | `/goal` 相位提示 | `commandHint` 按 phase+activation 给命令建议（`command-goal:60-75`） | `phase_commands` 同思路，但 `completed` 未阻止 `resume`（`goal_command.rs:262-270,176-192`） | 差异 |
| 28 | 斜杠命令附件 | `/goal` 支持附件随 objective 提交（`command-goal:117-123`） | 无证据（`goal_command.rs` 只处理文本） | DSH 更强（低价值） |
| 29 | 系统提示 | 独立 `tool:goal` section + 注入阈值 + 「resume 后需人类 resume」指引（`tool-goal/src/index.ts:113-124,189-193`） | 通用段落硬编码 + `Active goal:` 一行（`context.rs:34,1159-1168`） | DSH 更强 |
| 30 | 领域事件 → UI | `goal/changed` 事件 + projection 双通道（`index.ts:609-625`；`index.ts:162-169`） | 前端订阅 `goal://changed`/`goal://round`，**后端 0 发射点**（`GoalBar.vue:56-57,88-101` vs `grep -rn "goal://" crates src-tauri` = 空） | DSH 更强 |
| 31 | UI 组件 | GoalBar + /goal 命令输入节点 + 引用规则 + slot 注入（`ui-goal/src/client/index.ts:64-102`） | GoalBar.vue（264 行）+ `/goal` 文本命令 + 快捷键帮助项（`GoalBar.vue`；`slash-commands.ts:24`；`keyboard-shortcuts.ts:37`） | 差异 |
| 32 | 完成态展示 | 完成即隐藏 GoalBar（`GoalBar.tsx:86`） | 保留并显示"已完成"（`GoalBar.vue:48`） | 差异（口味） |
| 33 | blocked 展示 | 仅 tooltip 显示理由（`GoalBar.tsx:129,135`） | 常驻红色横幅（`GoalBar.vue:239-248`） | **Pony 更强** |
| 34 | 进度可视化 | 仅 `Rounds: N/M` 文本（`command-goal:90`） | `progressPercent` getter（`src/stores/goal.ts:19-25`） | **Pony 更强**（能力在，是否渲染需确认） |
| 35 | TUI 等价物 | 无（DSH 只有 web client） | 有完整 TUI 侧（banner/`/goal`/停止状态行，`bin/tui/goal_command.rs:35-66`；`goal.rs:476-506`） | **Pony 更强** |

---

## 3. 优化清单（可执行）

计数：**P0 × 3，P1 × 6，P2 × 5**（另有 2 条明确"不采纳"）。

### P0-1 领域层缺状态迁移守卫：`resume` 可把已完成目标复活

- **问题**：`update_goal` 只校验 id/revision，然后对 `phase` 无条件赋值。`completed → active`（`UpdateGoalAction::Resume`）、`completed → paused`、`blocked → paused`、`completed → completed` 全部放行；`/goal resume` 也没有相位门（`require_goal` 只判"存在"）。
- **证据**：
  - `crates/pony-agent-core/src/agent/goal.rs:629-640`（只校验 id / revision）
  - `crates/pony-agent-core/src/agent/goal.rs:645-650`（Resume 无条件 `phase = "active"` + `Armed`）
  - `crates/pony-agent-core/src/agent/goal.rs:641-644`（Pause 无条件）
  - `crates/pony-agent-core/src/agent/goal_command.rs:176-192`（`/goal resume` 无相位判断）
  - 对照 DSH：`packages/goal/goal/src/index.ts:351-353,365-381,392-399,413-415`（allowed 矩阵）
- **建议动作**：在 `update_goal` 内加显式 allowed 矩阵（pause←{active}、resume←{active,paused,blocked} 且预算未耗尽、complete←{active,paused,blocked}、blocked←{active}、edit←任意非 completed），非法迁移返回稳定错误码（如 `GOAL_INVALID_TRANSITION`）。`goal_command.rs` 复用同一判据，不再各写一份。注意：`goal_command.rs:128-142` 已有"D：completed 时 edit 等价新建"，那是**替换语义**，与"resume 复活"是两回事，不要一起改。
- **是否值得做**：**值得，P0**。当前可让已完成目标在用户无感知下重新自动续轮（resume 即 Armed），是直接烧钱路径，且与 §4 的"完成即终态"语义冲突。

### P0-2 领域层 `create_goal` 无"已存在"守卫，可静默覆盖进行中目标

- **问题**：`create_goal` 直接 `goals.insert(session, goal)`，不复位不报错。唯一守卫在 `/goal` 命令层（`goal_command.rs:102-108`），而模型工具 `create_goal` 走 `create_goal()` 直连，绕过守卫 → 进行中目标的 `objective/rounds_started/revision` 被整体替换。
- **证据**：
  - `crates/pony-agent-core/src/agent/goal.rs:584-590`（insert 前无 phase 判断）
  - `crates/pony-agent-core/src/agent/tools/orchestration.rs:461-486`（工具无守卫直连领域层）
  - `crates/pony-agent-core/src/agent/tools/schema.rs:886-899`（`create_goal` 描述未提"覆盖"）
  - 对照 DSH：`packages/goal/goal/src/index.ts:306-309`（`GOAL_ALREADY_EXISTS`）
- **建议动作**：领域层 `create_goal` 在 `phase != "completed"` 时返回 `Err("goal already exists with phase …")`；`goal_command.rs` 的 completed-替换分支改为显式"先 clear 再 create"或保留一条内部专用入口（`replace_goal`），避免命令层与工具层语义分叉。
- **是否值得做**：**值得，P0**。静默丢目标 = 丢轮次预算 + 丢 objective，用户只能从日志考古。

### P0-3 `control_plane` 两条续轮路径绕过 PA-171 C2 契约

- **问题**：graph 与 planner 两条循环用的是 `advance_goal_round`（**无条件 +1**），不是 `advance_goal_round_for_outcome`；且没有 `goal_round_stop_reason`（无连续失败止损）、超限与耗尽都**静默 `return Ok(response)`**。等于 PA-171 冻结的"失败轮不烧预算 + 连续 2 轮失败止损"只在 TUI 生效。
- **证据**：
  - `crates/pony-agent-core/src/agent/control_plane/mod.rs:1947`（`advance_goal_round`）
  - `crates/pony-agent-core/src/agent/control_plane/mod.rs:1939-1944`（8 轮/超限直接 return，无事件无日志）
  - `crates/pony-agent-core/src/agent/control_plane/mod.rs:2771`（planner 路径同缺陷）
  - 正确实现：`crates/pony-agent-core/src/agent/goal.rs:537-546`（按 outcome 推进）、`goal.rs:510-535`（止损判据）
  - 同契约的 TUI 侧：`crates/pony-agent-core/src/bin/tui.rs:8277-8330`
- **建议动作**：把 TUI 的 `run_goal_rounds` 判据抽成共享的 `goal_round_loop` 组件（租约 + 判据 + 计数 + 可见停止四件套），control_plane 两条循环改为调用它；最小改动版至少：改调 `advance_goal_round_for_outcome`、接入 `goal_round_stop_reason`、停止时发一条可见事件/警告日志（对齐 DSH `drive()` 的 block 语义）。
- **是否值得做**：**值得，P0**。这是"同一契约三处实现、两处未遵守"，属回归风险最高的地方；一次 429 在 graph 路径可重放多轮并烧光预算（`goal.rs:410-414` 注释记录的 16 轮事故原型）。

### P1-1 轮次消息无结构化来源，靠中文文案前缀嗅探

- **问题**：宿主注入的 goal 轮消息是纯文本，会话层用 `is_goal_round_prompt` 匹配 `<goal_round round="N" max="M">继续推进目标：…` 判定；改一个标点/换语言/加前缀就会把宿主编排消息误判为真人消息（进而误判 Ask 过期、污染会话标题）。
- **证据**：
  - `crates/pony-agent-core/src/agent/goal.rs:325-334`（文案真源）
  - `crates/pony-agent-core/src/agent/goal.rs:337-362`（前缀嗅探）
  - `crates/pony-agent-core/src/agent/session/host_continuation_goal_round_tests.rs:11-13`（测试同样依赖文案）
  - 对照 DSH：`packages/goal/goal-round-driver/src/index.ts:190-195`（`source: {kind:'goal', goalId, revision, round}`），fold 用 source 而非文本判定 `packages/goal/goal/src/fold.ts:313-331`
- **建议动作**：给消息增加机读字段（`TurnInput`/消息结构里加 `host_continuation: Some(GoalRound{goal_id, round})`），`is_host_continuation_prompt` 优先读字段、文本嗅探保留一个版本作兼容回退。
- **是否值得做**：**值得，P1**。它是 P1-6（轮次真源）与未来"事件化 goal"的前置改造，成本低（一个字段 + 一处判定）。

### P1-2 `goal://changed` / `goal://round` 是死通道

- **问题**：前端 GoalBar 订阅这两个事件并在收到后刷新，但 Rust 侧全局搜索 `goal://` **没有任何发射点**。实际刷新只靠 sessionId 变化与 turn 结束；长跑目标期间（一轮内多步、或另一端改了 goal）UI 不会更新。
- **证据**：
  - `src/components/GoalBar.vue:56-57`（事件名常量）、`src/components/GoalBar.vue:88-101`（订阅）
  - `src/stores/goal.ts:126-147`（注释声称事件驱动刷新）
  - 反证：`grep -rn "goal://" --include=*.rs crates src-tauri/src` → **空**
  - 对照 DSH：`packages/goal/goal/src/index.ts:609-625`（append 后 `emit('goal/changed')`）
- **建议动作**：二选一——(a) 在 `update_goal/advance_goal_round/clear_goal` 成功后由 Tauri 层 `emit("goal://changed", {sessionId})`、每次开轮 `emit("goal://round", {round})`；(b) 删除前端订阅与 `refreshForTurn` 里的事件注释，改为纯 turn-end 拉取，避免"看起来有推送"的误解。推荐 (a)，成本约 20 行。
- **是否值得做**：**值得，P1**（低成本、直接决定"轮次进度是否可信"）。

### P1-3 重启后自动续轮（activation 持久化 + 无 disarm-on-load）

- **问题**：`Armed` 随 goal 一起落盘，进程重启后 `active_goal_for_session` 直接返回 Armed 目标，宿主循环随即续轮——没有一次"人类说继续"的确认。
- **证据**：
  - `crates/pony-agent-core/src/agent/goal.rs:579`（创建即 Armed 并落盘）
  - `crates/pony-agent-core/src/agent/goal.rs:309-318`（读面只按 phase+activation 过滤）
  - `grep -rn "Disarmed" crates` 非测试命中仅 `goal.rs:643,652,660`（只有 pause/complete/block 三处），**无启动 disarm 路径**
  - 对照 DSH：`types.ts:71-72`（activation 明确 process-local）、`index.ts:255-257`（`agent/created` 即 disarmed）、driver 加载时对已有 agent 全量 disarm `goal-round-driver/src/index.ts:448-452`、提示词指引"resume 后需人类 resume"`tool-goal/src/index.ts:113-118`
- **建议动作**：启动/会话恢复时把非 terminated 的 goal 一律置 `Disarmed`（保留 phase 与轮次），由用户 `/goal resume` 或前端"继续"按钮重新 Armed。
- **是否值得做**：**值得，P1**（安全/成本）。需要产品确认：若团队刻意要"重启即续跑"，则应改文档并补一条显式配置项，而不是靠默认行为隐式决定。

### P1-4 blocked 无阈值、无结构，纯软约束

- **问题**：模型第 1 轮就能把目标标 blocked（只要求理由非空），提示词说"至少 3 轮"但代码不校验；理由只有一段自由文本，无机器可路由的 code。
- **证据**：
  - `crates/pony-agent-core/src/agent/goal.rs:655-663`（只判 `blocked_reason` 非空）
  - `crates/pony-agent-core/src/agent/goal.rs:38`（`blocked_reason: Option<String>`）
  - `crates/pony-agent-core/src/agent/context.rs:34`（软约束文本）
  - 对照 DSH：`tool-goal/src/index.ts:305-312`（`GOAL_TOOL_BLOCK_THRESHOLD`，默认 3，可配 `blockedAfterConsecutiveRounds`）、`types.ts:51-57` + `fold.ts:73-85`（code 必须 lower-kebab）
- **建议动作**：`UpdateGoalArgs` 增加可选 `blocked_code`，`goal.rs` 落库为 `{code, message}`（兼容旧数据：无 code 时填 `legacy`）；在工具层加阈值校验（复用 `goal.rs` 已有常量风格，新增 `MIN_GOAL_ROUNDS_BEFORE_BLOCKED = 3`），由配置覆盖。
- **是否值得做**：**值得，P1**。理由：blocked 是"停下来等人类"的唯一通道，模型误用会白停一整轮；code 化后前端可分类展示（DSH 的 UI 已按 reason.message 展示，`GoalBar.tsx:129`）。

### P1-5 模型工具无权限门：可自行 pause / resume / complete 任意轮

- **问题**：`create_goal/get_goal/update_goal` 直接落库，不区分"人类直说"与"模型在自己续轮里顺手改"，也不区分顶层 agent 与子代理。对照 DSH：create/edit/pause/resume 必须 direct-human；`complete/blocked` 才允许"当前 goal 轮"；且**显式禁止模型 resume 一个被人类 pause 的目标**。
- **证据**：
  - `crates/pony-agent-core/src/agent/tools/orchestration.rs:517-543`（无 authority 检查）
  - `crates/pony-agent-core/src/agent/tools/schema.rs:923-960`（schema 未表达任何权限约束）
  - 对照 DSH：`tool-goal/src/authority.ts:100-103`（requireDirectHuman）、`authority.ts:111-118`（completionAuthority）、`tool-goal/src/index.ts:279-285`（`GOAL_TOOL_RESUME_PAUSED`）
- **建议动作**：在 `execute_update_goal` / `execute_create_goal` 入口加最小判定：从 `ToolExecutionContext` 取"本 turn 是否由真人消息发起"与"当前是否处于 goal 轮"，pause/resume/edit 要求真人；`resume` 且当前 `paused` 且非真人 → 拒绝。
- **是否值得做**：**值得，P1**。与 P0-1 组合后，才能真正保证"人类暂停"不会被模型下一步撤销（目前 `pause` 后可被同一轮 `update_goal resume` 抵消）。

### P1-6 隐藏的"每线程 8 轮"截断 vs 用户可见的 `max_goal_rounds=50`

- **问题**：用户/模型设置 50 轮，实际每线程只跑 8 轮就停（`AutoContinueBudget`），且该上限不可配置、不出现在任何 schema/UI 文案里。
- **证据**：
  - `crates/pony-agent-core/src/agent/planner.rs:23`（`MAX_CONSECUTIVE_AUTO_CONTINUES = 8`）
  - `crates/pony-agent-core/src/bin/tui.rs:8277-8297`（8 轮判据参与 stop_reason）
  - `crates/pony-agent-core/src/agent/goal.rs:582`（默认 50）、`tools/schema.rs:894-897`（"默认 50"）
  - 对照 DSH：唯一上限 `maxGoalRounds`（`index.ts:243-245`、`goal-round-driver/src/index.ts:183-186`）
- **建议动作**：把 8 轮降级为"无 goal 时的普通自动续行预算"，goal 存在时只以 `max_goal_rounds` 为准；若要保留安全阀，做成 `max_autocontinue_per_thread` 配置并在 `get_goal`/`/goal` 回执里同时展示两个数。
- **是否值得做**：**值得，P1**。这是"配置了不生效"的信任问题，且 `AutoContinueBudget` 的可见文案会误导用户以为目标被放弃。

### P2-1 轮次耗尽应 durable block，而非只停一行

- **问题**：TUI 用 9 种 `visible_line` 停下（好），但 goal 仍是 `active`，UI 仍显示"进行中"；graph 路径更是静默返回。用户重开界面会看到"active + 50/50"这种自相矛盾状态。
- **证据**：`crates/pony-agent-core/src/agent/goal.rs:510-535`（判据只返回原因，不改 phase）；`crates/pony-agent-core/src/agent/control_plane/mod.rs:1939-1944`；对照 DSH `goal-round-driver/src/index.ts:183-186`（`block(..., {code:'round-limit'})`）
- **建议动作**：`BudgetExhausted` / `ConsecutiveFailures` 命中时调用 `update_goal Blocked(code=..., message=…)`，让相位与展示一致（blocked_reason 用机读 code 见 P1-4）。
- **是否值得做**：**值得，P2**（依赖 P1-4 的 code 化；否则会污染现有自由文本理由）。

### P2-2 终态收口消息缺失

- **问题**：模型在自己续轮里 `complete`/`blocked` 后，本轮照常结束，没有一条"用最终答复收尾"的指令；容易出现"标记完成但没写给用户看的结果"。
- **证据**：Pony 无对应实现（`goal.rs` 只写状态）；对照 DSH `tool-goal/src/index.ts:318-330`（`exec.deferContext(...)`）+ `tool-goal/src/wrapup.ts:17-40`（`<goal_complete>`/`<goal_blocked>` 文案，含"不要在这轮再调工具"）。
- **建议动作**：`execute_update_goal` 在 action=complete/blocked 且当前处于 goal 轮时，追加一条系统/用户侧的收尾提示（Pony 有 `defer_context` 类机制可复用，具体入口需在 `tools/` 或 `control_plane` 侧确认后接线）。
- **是否值得做**：**值得，P2**（体验），但需先确认 Pony 的 turn 内注入通道，避免造第二套注入机制。

### P2-3 `blocked_reason` 机读化（并入 P1-4 落地即可）

- 见 P1-4。若 P1-4 暂缓，至少让前端不要假设 `blocked_reason` 是纯展示文本（当前 `GoalBar.vue:239-248` 直接当字符串渲染，字段升级为对象会破前端）。

### P2-4 暂停的实时抢占（pause 时打断当前轮）

- **问题**：用户点"暂停"，正在跑的这一轮继续跑到自然结束，期间模型仍可改目标状态。
- **证据**：Pony 无 cancel-on-pause（`goal.rs:641-644` 只改状态）；对照 DSH `goal-round-driver/src/index.ts:296-312`（`goal/changed` + `operation === 'pause'` + 非模型自己发起 ⇒ `agent.cancel({kind:'user'}, {keepInbox:true})`）。
- **建议动作**：pause 成功后，若该 session 当前有运行中的 turn 且非该 turn 自己发起 pause，则中断该 turn。
- **是否值得做**：**值得，P2**（需谨慎：Pony 的中断语义与 Ask/审批待答状态耦合，建议单独开卡并补交互测试）。

### P2-5 goal 事件化 / 投影（架构级）

- **问题**：Pony 的 goal 不在会话日志里 → 无审计、无法回放、跨进程靠 JSON merge、UI 无投影可订阅。
- **证据**：`goal.rs:89-119`（sidecar）；`goal.rs:200-262`（merge 语义）；对照 DSH `known-event-types.ts:41`、`index.ts:146-169`（投影 + `failure` 降级）、`fold.ts:187-268`（迁移校验矩阵）。
- **建议动作**：不建议短期重写。折中方案：在会话事件流里追加一条只读 `goal/snapshot` 审计事件（append-only），sidecar 仍是权威读面；先获得审计/回放收益，迁移成本可控。
- **是否值得做**：**部分不采纳**：完整事件溯源改动面覆盖 `goal.rs`+`session`+`control_plane`+`tui`+前端，收益主要是审计与多端一致性，当前 Pony 的 sidecar 已能工作。**采纳折中方案（审计事件）**，不采纳"全量迁移到 fold 派生"。

### 明确不采纳（附理由）

| 项 | 来源 | 不采纳理由 |
| --- | --- | --- |
| 完成态隐藏 GoalBar | `GoalBar.tsx:86` | Pony 的"已完成 + 轮次 + 可执行命令（`/goal <新目标>`）"是有用的收尾面板（`goal_command.rs:262-270`），隐藏反而丢掉"下一步该做什么"的提示。 |
| `/goal` 携带附件（`command-goal/src/index.ts:117-123`） | DSH | Pony 的 `/goal` 是纯文本命令通道（`goal_command.rs:50-82` 只解析字符串），要支持附件得改命令通道与 Tauri 入参，收益低。若确有需求，让用户先发附件消息再 `/goal` 引用即可。 |
| 默认 `maxGoalRounds` 从 50 改 256 | `index.ts:243-245` | 纯口味；Pony 有 8 轮隐藏截断（P1-6）时改默认值无意义，应先修 P1-6。 |

---

## 4. 已覆盖、无需动（避免重复建设）

| 能力 | Pony 证据 | DSH 证据 | 说明 |
| --- | --- | --- | --- |
| CAS（id + revision） | `goal.rs:629-640` | `index.ts:456-466` | 语义等价，错误提示都可读 |
| 三工具集 `create/get/update_goal` | `tools/schema.rs:886-960`、`tools/orchestration.rs:461-543` | `tool-goal/src/index.ts:196-343` | 参数几乎一一对应；差距在权限门（P1-5），不在工具本身 |
| `/goal` 斜杠命令 + 相位命令建议 | `goal_command.rs:15,50-82,262-270` | `command-goal/src/index.ts:16,35-45,60-75` | Pony 多 `complete` 子命令，属增强 |
| 轮次文案单一真源 | `goal.rs:325-334`（注释明确"宿主与 TUI 必须共用"） | `goal-round-driver/src/prompt.ts:12-26` | Pony 的做法正确，问题只在**识别**方式（P1-1） |
| 失败轮不烧预算（TUI 路径） | `goal.rs:537-546` | `fold.ts:329-331`（天然） | 契约已冻结并有测试（`tests/pa171_goal_round_failure_contract.rs`），**不要改** |
| 连续失败止损（2 轮） | `goal.rs:416-420,510-535` | 无等价 | **Pony 独有优势**，DSH 只靠 turn 终态 disarm |
| 单会话单续轮循环（租约） | `goal_lease.rs:19-74`、`tui.rs:8229-8251` | 靠状态机串行化 | **Pony 独有优势**，不要为了"对齐 DSH"去掉 |
| 停止原因可见（9 种） | `goal.rs:433-506` | 只有 block 理由 | **Pony 更强**，保留 |
| 待答 Ask/审批让位 | `tui.rs:8266-8276` | 无直接等价（DSH 用 competingQueued + pre-step reject） | 两套机制目标相同，Pony 的更显式 |
| goal 进系统提示 | `context.rs:34,1159-1168` | `tool-goal/src/index.ts:189-193` | 位置不同（通用段 vs 独立 section），效果等价，无需为"独立 section"改造 |
| blocked 原因 UI 展示 | `GoalBar.vue:239-248`（常驻横幅） | `GoalBar.tsx:129`（tooltip） | **Pony 更好**，保留 |

---

## 5. 建议落地顺序（按依赖与收益）

1. **P0-1 + P0-2**（同文件 `goal.rs`，一次改完 + 补迁移矩阵单测）→ 消除"复活/覆盖"两条最贵的错误路径。
2. **P0-3**（control_plane 两条循环）→ 先改调 `advance_goal_round_for_outcome` 并接 `goal_round_stop_reason`（最小改），再考虑抽公共 `goal_round_loop`。
3. **P1-3**（disarm-on-load）→ 一行启动逻辑 + 一个测试；先确认产品口径。
4. **P1-2**（goal 事件发射）→ 20 行，立刻改善 UI 可信度。
5. **P1-5 + P1-4**（权限门 + blocked 阈值/code）→ 一起做，因为它们共享"本 turn 是否真人发起 / 是否 goal 轮"的判定上下文。
6. **P1-1 → P1-6**（结构化轮次来源 → 上限统一）→ P1-1 是 P1-6 与未来审计事件的前置。
7. **P2-1/2/4/5** 按需择机。

**每条改动的验收口径（建议）**：
- 领域层改动：`cargo test -p pony-agent-core goal` + `pa171_goal_round_failure_contract` 全绿；新增迁移矩阵用例（completed→resume 必须 Err）。
- control_plane 改动：新增"失败轮不推进 rounds_started"用例（对齐 `tests/pa171_goal_round_failure_contract.rs`）。
- 前端改动：`npx vitest run tests/GoalBar.spec.ts tests/goal-store.spec.ts tests/slash-commands-goal.spec.ts` 全绿。

---

## 附录 A：关键证据索引

### DSH（`/home/dm/deepseek-harness`）

| 主题 | 路径:行号 |
| --- | --- |
| goal 领域类型 / 相位 / 快照 / 激活 | `packages/goal/goal/src/types.ts:16-87` |
| 投影定义 + 事件折叠降级 | `packages/goal/goal/src/index.ts:146-169` |
| 默认轮次上限 256 | `packages/goal/goal/src/index.ts:243-245` |
| 创建即 armed / agent 创建即 disarmed | `packages/goal/goal/src/index.ts:255-257,303-318` |
| 迁移矩阵（pause/resume/complete/block） | `packages/goal/goal/src/index.ts:351-424` |
| 预算耗尽拒绝 resume | `packages/goal/goal/src/index.ts:375-379` |
| clear 墓碑 | `packages/goal/goal/src/index.ts:432-447` |
| 事件发射 `goal/changed` | `packages/goal/goal/src/index.ts:609-625` |
| 严格解码 / 迁移校验 / 轮次计数 | `packages/goal/goal/src/fold.ts:134-172,187-268,313-331` |
| 续轮驱动核心 | `packages/goal/goal-round-driver/src/index.ts:106-221` |
| 超限 block | `packages/goal/goal-round-driver/src/index.ts:183-186` |
| 人 pause 抢占取消 | `packages/goal/goal-round-driver/src/index.ts:296-312` |
| 加载时全量 disarm | `packages/goal/goal-round-driver/src/index.ts:448-452` |
| 轮次文案 | `packages/goal/goal-round-driver/src/prompt.ts:12-26` |
| 工具 + 阈值 + 收口注入 | `packages/goal/tool-goal/src/index.ts:189-193,305-312,318-330` |
| 权限门 | `packages/goal/tool-goal/src/authority.ts:48-118` |
| 收口文案 | `packages/goal/tool-goal/src/wrapup.ts:17-40` |
| `/goal` 命令 | `packages/goal/command-goal/src/index.ts:16,35-45,60-96,126-197` |
| 会话事件类型 `goal/change` | `packages/core/session/src/known-event-types.ts:41` |
| 系统提示 order `TOOL_GOAL` | `packages/core/system-prompt/src/index.ts:151` |
| UI GoalBar / 激活覆盖 | `packages/client/ui-goal/src/client/GoalBar.tsx:86,129-135,187-198` |
| 激活源（Remote + 事件排序） | `packages/client/ui-goal/src/client/activation-source.ts:29-60` |
| ralph 与 goal 分工说明 | `packages/experimental/tool-ralph/src/index.ts:408` |

### Pony（`/home/dm/pony-agent`）

| 主题 | 路径:行号 |
| --- | --- |
| GoalData / 激活枚举 / blocked_reason | `crates/pony-agent-core/src/agent/goal.rs:13-40` |
| sidecar 路径 / 懒加载 / 降级 | `crates/pony-agent-core/src/agent/goal.rs:97-170` |
| 原子写 + 跨进程 merge | `crates/pony-agent-core/src/agent/goal.rs:200-262` |
| 唯一读面（active + armed） | `crates/pony-agent-core/src/agent/goal.rs:309-323` |
| 轮次文案 + 文本嗅探 | `crates/pony-agent-core/src/agent/goal.rs:325-362` |
| 轮次推进（不增 revision） | `crates/pony-agent-core/src/agent/goal.rs:373-414` |
| 止损常量 / 停止原因 / 可见文案 | `crates/pony-agent-core/src/agent/goal.rs:416-535` |
| 按 outcome 推进（正确入口） | `crates/pony-agent-core/src/agent/goal.rs:537-546` |
| create（无守卫，默认 50） | `crates/pony-agent-core/src/agent/goal.rs:571-604` |
| update（无迁移守卫） | `crates/pony-agent-core/src/agent/goal.rs:617-696` |
| clear | `crates/pony-agent-core/src/agent/goal.rs:698-720` |
| 续轮租约 | `crates/pony-agent-core/src/agent/goal_lease.rs:19-74` |
| `/goal` 解析 + 状态机 | `crates/pony-agent-core/src/agent/goal_command.rs:15-224` |
| 续轮主循环（TUI） | `crates/pony-agent-core/src/bin/tui.rs:8203-8330` |
| 8 轮隐藏上限 | `crates/pony-agent-core/src/agent/planner.rs:23` |
| graph/planner 续轮（绕过契约） | `crates/pony-agent-core/src/agent/control_plane/mod.rs:1934-1963,2758-2787` |
| 系统提示约束 + Active goal | `crates/pony-agent-core/src/agent/context.rs:34,1159-1168` |
| 工具执行（无权限门） | `crates/pony-agent-core/src/agent/tools/orchestration.rs:461-543` |
| 工具 schema | `crates/pony-agent-core/src/agent/tools/schema.rs:886-960` |
| Tauri 命令 + 注册 | `src-tauri/src/lib.rs:408-472,1535-1538` |
| GoalBar（含死订阅） | `src/components/GoalBar.vue:48,56-57,88-101,239-248` |
| goal store（含 progressPercent） | `src/stores/goal.ts:5-147` |
| 斜杠命令与快捷键帮助 | `src/lib/chat/slash-commands.ts:24,41-50`；`src/lib/chat/keyboard-shortcuts.ts:37` |

## 附录 B：本次核对用的检索命令（可复现）

```bash
# DSH 侧
cd /home/dm/deepseek-harness && git log -1 --format='%H %ci'
grep -rn "goal" packages/core/session/src/known-event-types.ts
grep -n "export function applyGoalEvent\|state.roundsStarted = source.round" packages/goal/goal/src/fold.ts
grep -n "1,1" /dev/null; grep -n "GOAL_TOOL_BLOCK_THRESHOLD\|requireDirectHuman(ctx, execution)" packages/goal/tool-goal/src/index.ts
grep -n "round-limit\|prompt-rejected\|queue-failed" packages/goal/goal-round-driver/src/index.ts

# Pony 侧
cd /home/dm/pony-agent
grep -rn "Disarmed" --include=*.rs crates | grep -v test      # → 仅 goal.rs:643,652,660（无启动 disarm）
grep -rn "goal://" --include=*.rs crates src-tauri/src        # → 空（前端订阅无发射点）
grep -rn "goal_round_stop_reason\|advance_goal_round_for_outcome" --include=*.rs crates | grep -v "agent/goal.rs"
grep -n "advance_goal_round(" crates/pony-agent-core/src/agent/control_plane/mod.rs
```

> 说明：`grep -rn "goal://" --include=*.rs crates src-tauri/src` 在本次核对中返回**空结果**（退出码 1），这是 P1-2 的关键反证；其余检索均退出码 0。

## 附录 C：Lead 复核（抽检 3 条 P0/关键结论 + 1 条新增）

抽检方式：不采信摘要，直接读源码原文。

| 被检结论 | 复核结果 | 直接证据 |
|---|---|---|
| P0-1 `resume` 可复活已完成目标 | **成立**。`update_goal` 的 match 分支无任何前置相位判定：`Resume` 无条件写 `phase="active"` + `activation=Armed`，对 `completed` 同样生效 | `crates/pony-agent-core/src/agent/goal.rs:641-661` |
| P0-2 `create_goal` 无「已存在」守卫 | **成立**。`goals.insert(session, goal)` 不做既有记录判定，`/goal <新目标>` 即静默替换进行中目标 | `goal.rs:584-593` |
| P1-2 `goal://changed` / `goal://round` 死通道 | **成立**。全仓 `.rs` 零命中，只有前端监听侧 | `src/components/GoalBar.vue:56-57`；`src/stores/goal.ts:134` |
| 事实修正 | 本文 §0 称「Pony 无 git 仓库」不成立：Pony 是 git 仓库，复核时 HEAD 为 `23091fa3` | `git log --oneline -1` |

### 新增：P2-6 会话删除不清理目标条目（本文其余条目未覆盖）

- **问题**：`clear_goal` 仅有 `/goal clear` 一个调用点；`delete_session`（`src-tauri/src/lib.rs:1106-1110`）不触碰 goal sidecar，`goal.rs` 也没有「按会话删除」API。
- **证据**：`grep -rn "clear_goal" --include=*.rs crates src-tauri` 除 `goal.rs` 自身外仅命中 `goal_command.rs:213`；`goal.rs` 公开面只有 `create_goal/get_goal/update_goal/clear_goal/advance_goal_round*`（`goal.rs:571,607,617,698,373,537`）。
- **影响**：`goal_store.json` 随会话删除单调增长；会话 id 若被复用则读到上一会话的残留目标。
- **建议动作**：`delete_session` 路径调 `clear_goal(session_id)`，并加断言「删除会话后 `get_goal` 返回 `None`」。属独立小改，可与任一批次合并。
- **优先级**：P2（无安全面，只有数据卫生面）。

### 本轮前端交付的机器证据（与本文同批）

| 能力 | 证据 |
|---|---|
| `/goal` 斜杠命令前端接线（语义真源在 Rust `agent::goal_command`） | `src/lib/chat/slash-commands.ts:20-27`；`HomeWorkspace.vue:1570-1589`；`src-tauri/src/lib.rs:468-473`；`tests/slash-commands-goal.spec.ts` 9 passed |
| `Ctrl/Cmd+G` 直达 Goal 面板（无目标时给空态引导） | `HomeWorkspace.vue:1730-1737`；`src/stores/workspace-panels.ts:14,32`；`tests/workspace-goal-shortcuts.spec.ts` 6 passed、`tests/goal-panel-and-shortcuts.spec.ts` 4 passed |
| `Ctrl/Cmd+H` 快捷键帮助面板（任何焦点可呼出，清单真源与 handler 同源） | `HomeWorkspace.vue:1716-1729`；`src/components/ShortcutsHelpModal.vue`；`src/lib/chat/keyboard-shortcuts.ts:39-78`；`tests/shortcuts-help-registry.spec.ts` 9 passed |

定向门禁（本轮实跑）：`npx vitest run`（7 文件 / 48 用例）exit 0；`npx vue-tsc --noEmit` exit 0。
