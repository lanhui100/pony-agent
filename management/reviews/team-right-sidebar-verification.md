# 独立验收报告：Team 面板迁移至右侧边栏 + 任务生成次序排序契约

- 验收者：`verifier`（独立验收，未修改任何产品代码）
- 仓库：`/home/dm/pony-agent`
- 基线 HEAD：`418b06dfbbdcbe2b5b2d0b3a6af0b2a0c647558f`
- 冻结修订时间：2026-10-19 09:14:57 (+08:00)（此后前端/Rust 源码零改动；见 §2 哈希）
- 报告落盘：`management/reviews/team-right-sidebar-verification.md`
- 最终 verdict：**PASS**（findings 为空数组）

---

## 0. 验收对象清单（本轮待验收变更）

| # | 变更 | 物理收据 |
|---|---|---|
| 1 | `crates/pony-agent-core/src/agent/orchestration.rs`：`TeamTaskItem.created_at_ms: u64`、`team_task_create` 写入 `now_unix_ms()`、`team_task_list()` 按 `(created_at_ms, task_id)` 排序 | §3、§4 |
| 2 | `src/components/chat/AgentTeamPanel.vue` 删除；新建 `TeamRosterPanel.vue` / `TeamTaskBoardPanel.vue`；`HomeSidebar.vue` 挂载；`HomeWorkspace.vue` 移除旧面板 | §5、§6 |
| 3 | `src/stores/team.ts`：`sortMode` + `sortedTasks` + `setSortMode`；`src/types/team.ts`：`created_at_ms` | §5、§6 |
| 4 | （验收中途追加）`src/components/team/team-preview-fixtures.ts` 浏览器 dev 预览夹具 | §7 |

`git status --porcelain=v1`（验收结束时）：

```
 M Cargo.lock
 M crates/pony-agent-core/src/agent/orchestration.rs
 M src/components/HomeSidebar.vue
 M src/components/HomeWorkspace.vue
 D src/components/chat/AgentTeamPanel.vue
 M src/stores/team.ts
 M src/types/team.ts
?? crates/pony-agent-core/tests/team_task_order_contract.rs
?? management/screenshots/team_right_sidebar.png
?? management/screenshots/team_right_sidebar_collapsed.png
?? management/screenshots/team_right_sidebar_roster_expanded.png
?? management/screenshots/team_sort_created.png
?? management/screenshots/team_sort_progress.png
?? src/components/team/
?? tools/
```

`git diff HEAD --numstat -- '*.rs'`：

```
19	1	crates/pony-agent-core/src/agent/orchestration.rs
```

---

## 1. 静态门禁（原始命令 + 真实 Exit Code）

> 下列为**冻结修订（含夹具，09:14 之后零改动）**上的权威结果。夹具落盘前（09:07–09:08）另跑过一轮，结论相同，仅 `npm run build` 的产物内容不同（见 §7）。

| # | 原始命令 | Exit Code | 原始输出摘录 |
|---|---|---|---|
| G1 | `npx vue-tsc --noEmit` | **0** | 输出为空（仅 UNDICI 环境告警），无 TS 诊断 |
| G2 | `npm test` | **0** | `Test Files  90 passed (90)` / `Tests  966 passed \| 10 skipped (976)` / `Duration 40.71s` |
| G3 | `npm run build` | **0** | `✓ built in 2.90s`（`build` 脚本 = `vue-tsc --noEmit && vite build`） |
| G4 | `cargo check -p pony-agent-core --all-targets` | **0** | `Finished \`dev\` profile [unoptimized] target(s)` |
| G5 | `cargo test -p pony-agent-core --test teammate_e2e_communication_test` | **0** | `test test_teammate_bi_directional_communication_and_reply ... ok` / `1 passed; 0 failed` |
| G6 | `cargo test -p pony-agent-core --test team_task_order_contract`（本验收者新增） | **0** | `test test_team_task_created_at_ms_and_list_ordering_contract ... ok` / `1 passed; 0 failed` |
| G7 | `cargo test -p pony-agent-core --test pa110_teams_acceptance`（**既有失败**，见 §3） | **101** | `4 passed; 1 failed`，失败项 `test_ac01_subagent_dispatch` |

G1–G3 原始日志：`/tmp/verify2_vue_tsc.log`、`/tmp/verify2_npm_test.log`、`/tmp/verify2_npm_build.log`、`/tmp/verify2_gates_summary.txt`
G4–G6 原始日志：`/tmp/verify3_cargo_check.log`、`/tmp/verify3_e2e.log`、`/tmp/verify3_contract.log`、`/tmp/verify3_summary.txt`

---

## 2. 冻结修订哈希（验收证书锚点）

```
edc4da16a12dee2ec3837531d4cd522353d5701f75de292599ff251782080639  crates/pony-agent-core/src/agent/orchestration.rs
c3d2dd0d0b7bae0819be3915096c24d087f03ca3dd2d7d81ea63bbfe2530f8fa  crates/pony-agent-core/tests/team_task_order_contract.rs
5db978b5746047f4595118aa71d0cdf4435475e63e72f91ca6b8ca09d9f7c6f9  src/stores/team.ts
07ad3bac44443fd9fe0bbad20d896d633e6d22b2ae49aaaf1716584a7fa123e5  src/types/team.ts
1d142f4c797bede270bd6766ae02f4579ab8d3ad626e47af89570563a837fa16  src/components/HomeSidebar.vue
8c2bc541167f582dac24509ef9e11a35c75ff5b9076b43b2283e5887b3c7f67a  src/components/HomeWorkspace.vue
73f2e5d0dcb9cf9469524117e31d4739b51ae1290884a3b211795375c4f92664  src/components/team/TeamRosterPanel.vue
eea4c4695d88dd73deaf346aa730ba23f711c83d79d929f63b8f4221a8ecd0b6  src/components/team/TeamTaskBoardPanel.vue
c792c28bdbe8c2f6ae14b77bf55b4f9c4b88bb6129c233da998d36de78479b9c  src/components/team/team-preview-fixtures.ts
```

`crates/pony-agent-core/src/agent/orchestration.rs` mtime `2026-10-19 08:54:50`，早于本报告全部 Rust 门禁执行时间。

---

## 3. `pa110_teams_acceptance::test_ac01_subagent_dispatch` 既有失败——独立复核

**失败原文（工作树，`cargo test -p pony-agent-core --test pa110_teams_acceptance`，Exit Code 101）**：

```
failures:

---- test_ac01_subagent_dispatch stdout ----

thread 'test_ac01_subagent_dispatch' (3443214) panicked at crates/pony-agent-core/tests/pa110_teams_acceptance.rs:19:30:
subagent call should succeed: "capability_unavailable: subagent engine not wired (HostTurnEngine missing)"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    test_ac01_subagent_dispatch

test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

**独立复核（不改动工作树的 HEAD 基线对照实验）**：

```bash
git worktree add --detach /tmp/pony-head-baseline HEAD
cd /tmp/pony-head-baseline
CARGO_TARGET_DIR=/home/dm/pony-agent/target \
  cargo test -p pony-agent-core --test pa110_teams_acceptance test_ac01_subagent_dispatch
```

原始输出（`/tmp/verify_pa110_baseline.log`）：

```
Preparing worktree (detached HEAD 418b06d)
   Compiling pony-agent-core v0.1.126 (/tmp/pony-head-baseline/crates/pony-agent-core)
    Finished `test` profile [unoptimized] target(s) in 1m 06s
running 1 test
test test_ac01_subagent_dispatch ... FAILED
thread 'test_ac01_subagent_dispatch' (3458795) panicked at crates/pony-agent-core/tests/pa110_teams_acceptance.rs:19:30:
subagent call should succeed: "capability_unavailable: subagent engine not wired (HostTurnEngine missing)"
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.00s
EXIT_CODE=101
```

**结论：既有失败，与本轮改动无关。三项独立证据**：

1. **基线可复现**：在**未包含本轮任何改动**的 HEAD 工作树上，同一条用例以**相同 panic 原文**、相同 Exit Code `101` 失败。
2. **失败点不在本轮 diff 内**：错误字符串在任何修订中都源于 `orchestration.rs` 的 `subagent()` 入口（当前 [orchestration.rs:251-253](crates/pony-agent-core/src/agent/orchestration.rs#L251)）。`git show HEAD:crates/pony-agent-core/src/agent/orchestration.rs | grep -n` 显示该字符串在 HEAD 位于**第 241 行**；本轮 diff 在其上方净增 11 行，故当前位置 241+11=252 — 行号位移与 diff 行数严格吻合，说明该分支**逐字未改**。
3. **触发条件与排序无关**：失败根因是进程级引擎槽 `SUBAGENT_ENGINE: OnceLock<Arc<dyn SubagentTurnEngine>>`（[subagent_runtime.rs:37-48](crates/pony-agent-core/src/agent/subagent_runtime.rs#L37)）从未被 `set_subagent_engine()` 注入（`cargo test` 直接跑集成测试时不经过 `src-tauri` 的接线代码）。本轮 diff 只涉及 `TeamTaskItem` 字段、`now_unix_ms()` 与 `team_task_list()` 的 `sort_by`，不触碰子代理引擎槽。

---

## 4. 新增 Rust 排序契约测试（本验收者独立编写）

文件：`crates/pony-agent-core/tests/team_task_order_contract.rs`（新增，`c3d2dd0d…`）

### 4.1 断言清单（全部为真实断言，无吞异常、无跳过）

| 需求 | 实现位置 | 断言方式 |
|---|---|---|
| `created_at_ms` 非零 | `assert_ne!(item.created_at_ms, 0, …)` | 3 次连续创建逐条断言 |
| 随时间单调不减 | `pair[0].created_at_ms <= pair[1].created_at_ms` | 3 个任务两两相邻断言（每次创建间隔 `sleep(5ms)`） |
| 防「恒定值伪实现」 | `distinct.len() >= 2` | 时间戳取值集合必须多于 1 个 |
| `team_task_list()` 顺序 == `created_at_ms` 升序 | 先与**独立计算的期望序列** `sort_by(created_at_ms, then task_id)` 逐元素 `assert_eq!` | 全量元素严格相等 |
| 同毫秒次级键 `task_id` 升序 | `assert_sorted_by_created_then_id()`（`windows(2)` 全序）+ 同毫秒分组内 `pair[0].task_id < pair[1].task_id` | 条件断言，覆盖每个同毫秒分组 |
| 非 HashMap 随机序（确定性） | 连续 20 次 `team_task_list()`，每次与首次快照 `Vec<(task_id, created_at_ms)>` `assert_eq!` | 20/20 完全一致 |
| 先创建者必先出现 | 3 个顺序探针在返回数组中的下标严格递增 | `assert!(pos > prev)` |

设计要点（遵守工单约束）：`TEAM_TASKS` 为进程级全局单例，故**全部断言集中在同一个 `#[test]` 函数内**，且**不断言全局任务总数**（只断言 `len >= 本轮创建数`）。

### 4.2 原始运行输出（`/tmp/verify_order_contract.log`，带 `--nocapture`）

```
   Compiling pony-agent-core v0.1.126 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `test` profile [unoptimized] target(s) in 1m 01s
     Running tests/team_task_order_contract.rs (target/debug/deps/team_task_order_contract-c4bd7c481fb3057c)

running 1 test
[contract] burst tasks=64 distinct created_at_ms=1 ([1792372380593, 1792372380593, ... 64 项全同 ...])
[contract] same-millisecond groups observed = 1
test test_team_task_created_at_ms_and_list_ordering_contract ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

EXIT_CODE=0
```

**次级键分支被真实覆盖**：密集创建的 64 个任务**全部落在同一毫秒**（`distinct created_at_ms=1`），形成 1 个 64 元素同毫秒分组；分组内 `task_id` 严格升序断言通过。这不是「条件分支未命中」的空跑。

### 4.3 验收过程中发生的一次**工具性误报**（如实披露）

首次运行该测试时报 `error[E0609]: no field created_at_ms`（Exit 101）。根因是**验收者自身的操作错误**，不是产品缺陷：我把 §3 的 HEAD 基线实验的 `CARGO_TARGET_DIR` 指向了仓库共享的 `target/`，该次编译把 **HEAD 版本的 `libpony_agent_core.rlib`** 覆盖进共享产物，导致紧接着编译的契约测试链接到了不含新字段的旧 rlib。已通过 `git worktree remove --force /tmp/pony-head-baseline` + `cargo clean -p pony-agent-core` 清理并重编（详见 §10 副作用声明）。清理后 G6 稳定通过（连续两次运行均 Exit 0）。

---

## 5. 真实浏览器截图验收（agent-browser 0.27.0）

### 5.1 原始命令序列与真实输出

```bash
# 后台 job 启动 dev server（未使用 dev:web，该脚本在本仓库不存在）
$ npm run dev
  VITE v8.1.0  ready in 485 ms
  ➜  Local:   http://127.0.0.1:4176/          # vite.config.ts server.port=4176, strictPort

$ agent-browser open http://127.0.0.1:4176/
✓ Pony Agent
  http://127.0.0.1:4176/
$ agent-browser get url
http://127.0.0.1:4176/
$ agent-browser wait 2500
✓ Done

# 折叠态基线
$ agent-browser screenshot management/screenshots/team_right_sidebar_collapsed.png
✓ Screenshot saved to management/screenshots/team_right_sidebar_collapsed.png

# 展开「队友」
$ agent-browser find testid "team-roster-toggle" click
✓ Done
$ agent-browser get text "[data-testid='team-roster-list']"
总指挥 Lead
LEAD
前端实施者 Executor-Frontend 超长名称
BACKEND
验收 Verifier
QA
文档 Scribe
DOCS
运维 Ops
INFRA
$ agent-browser screenshot management/screenshots/team_right_sidebar_roster_expanded.png
✓ Screenshot saved to management/screenshots/team_right_sidebar_roster_expanded.png

# 展开「任务看板」（与队友互斥，队友自动折叠）
$ agent-browser find testid "team-task-board-toggle" click
✓ Done
$ agent-browser get text "[data-testid='team-task-list']"
搭建右侧栏面板 / 复用折叠面板样式，对齐 PlanPanel 观感 / 负责 EXECUTOR-FRONTEND / 已完成
排序契约对齐 / 以 created_at_ms 为生成次序排序键 / 负责 未分配 / 待开始
花名册面板 / 渲染队友状态点、名称与角色标签 / 负责 EXECUTOR-FRONTEND / 进行中
预览夹具隔离 / 仅 dev 浏览器生效，绝不写入 store / 负责 SCRIBE / 已完成
真机 IPC 校验 / Tauri 内验证任务字段完整回传 / 负责 未分配 / 待开始
双排序验收 / 断言两种排序顺序与状态分组均正确 / 负责 VERIFIER / 进行中
打包回归 / 构建并冒烟验证右栏渲染无报错 / 负责 INFRA-OPS / 待开始
$ agent-browser screenshot management/screenshots/team_right_sidebar.png
✓ Screenshot saved to management/screenshots/team_right_sidebar.png
```

排序切换取证（原始命令 + 原始输出）：

```bash
$ agent-browser get text "[data-testid='team-task-item']:nth-child(1)"     # created 模式
搭建右侧栏面板
复用折叠面板样式，对齐 PlanPanel 观感
负责 EXECUTOR-FRONTEND
已完成

$ agent-browser find testid "team-sort-progress" click
✓ Done
$ agent-browser get text "[data-testid='team-task-item']:nth-child(1)"     # progress 模式
花名册面板
渲染队友状态点、名称与角色标签
负责 EXECUTOR-FRONTEND
进行中
$ agent-browser get attr "[data-testid='team-sort-progress']" aria-pressed
true
$ agent-browser screenshot management/screenshots/team_sort_progress.png
✓ Screenshot saved to management/screenshots/team_sort_progress.png

$ agent-browser find testid "team-sort-created" click
✓ Done
$ agent-browser get text "[data-testid='team-task-item']:nth-child(1)"     # 回到 created
搭建右侧栏面板
复用折叠面板样式，对齐 PlanPanel 观感
负责 EXECUTOR-FRONTEND
已完成
$ agent-browser get attr "[data-testid='team-sort-created']" aria-pressed
true
$ agent-browser screenshot management/screenshots/team_sort_created.png
✓ Screenshot saved to management/screenshots/team_sort_created.png
```

**首项 subject 切换前后对比（机器原生 text 输出）**：

| sortMode | 首项 subject | 全量次序（in_progress → pending → completed 分组） |
|---|---|---|
| `created`（默认） | `搭建右侧栏面板` | 搭建右侧栏面板 → 排序契约对齐 → 花名册面板 → 预览夹具隔离 → 真机 IPC 校验 → 双排序验收 → 打包回归 |
| `progress` | `花名册面板` | 花名册面板(进行中) → 双排序验收(进行中) → 排序契约对齐(待开始) → 真机 IPC 校验(待开始) → 打包回归(待开始) → 搭建右侧栏面板(已完成) → 预览夹具隔离(已完成) |

### 5.2 截图产物

| 文件 | sha256 | 尺寸 | 内容 |
|---|---|---|---|
| `management/screenshots/team_right_sidebar.png` | `822c4218…` | 1280×633 | 右侧栏：队友(5) 折叠 + 任务看板(2/7) 展开，7 张卡片垂直排列 |
| `management/screenshots/team_sort_progress.png` | `bc7733b0…` | 1280×633 | 「按进展」高亮，列表重排（首项 花名册面板/进行中） |
| `management/screenshots/team_sort_created.png` | `c26520ce…` | 1280×633 | 「生成次序」高亮，列表回到创建次序 |
| `management/screenshots/team_right_sidebar_roster_expanded.png` | `80438f6e…` | 1280×633 | 队友面板展开，5 名队员 + 状态点 + 角色标签 + 超长名截断 |
| `management/screenshots/team_right_sidebar_collapsed.png` | `bce25b31…` | 1280×633 | 两块面板折叠基线 |

截图目视核验（`read_image`）：右栏两块面板标题、排序切换控件、垂直卡片列表均可见；**消息区顶部无任何 Team 面板**。

### 5.3 DOM/文本机器取证（`agent-browser eval` 结构化探针）

原始返回（`/tmp/verify_dom_probe_out.json`）：

```json
{
  "bodyHasLegacyTitle": false,
  "legacyOccurrencesInBody": 0,
  "legacyOccurrencesInAsides": 0,
  "asideCount": 2,
  "rosterInsideAside": true,
  "tasksInsideAside": true,
  "rosterPath": "button.flex.w-full < section.collapsible-shell.border-b < div.flex.min-h-full < div < div.h-full.w-full < div.relative.overflow-hidden < aside.flex.h-full < div.h-full.min-h-0 < div.max-h-[70rem].opacity-100 < div.gap-4.relative < section.flex.flex-col < section.flex.min-h-0 < main.relative.flex < div",
  "tasksPath": "button.flex.w-full < section.collapsible-shell.border-b < div.flex.min-h-full < div < div.h-full.w-full < div.relative.overflow-hidden < aside.flex.h-full < div.h-full.min-h-0 < div.max-h-[70rem].opacity-100 < div.gap-4.relative < section.flex.flex-col < section.flex.min-h-0 < main.relative.flex < div",
  "sortCreatedExists": true,
  "sortProgressExists": true,
  "rosterOpen": "false",
  "tasksOpen": "true",
  "memberItemCount": 5,
  "taskItemCount": 7,
  "viteErrorOverlayPresent": false,
  "rosterContainerText": "总指挥 Lead|LEAD|前端实施者 Executor-Frontend 超长名称|BACKEND|验收 Verifier|QA|文档 Scribe|DOCS|运维 Ops|INFRA",
  "firstTaskSubject": "搭建右侧栏面板",
  "sortContainerExists": true
}
```

逐条结论：

- **「队友」与「任务看板」在右侧边栏**：两块面板的 `data-testid` 祖先链均命中 `aside.flex.h-full`，且 `asideCount=2`（左侧会话栏 + 右侧边栏），面板归属右栏 `aside`。`✓`
- **消息区顶部不再出现「Agent Team 协同大盘」**：`bodyHasLegacyTitle=false`、`legacyOccurrencesInBody=0`、`legacyOccurrencesInAsides=0`。对照证据：该字符串在删除前的 [src/components/chat/AgentTeamPanel.vue](src/components/chat/AgentTeamPanel.vue) 第 14 行（`git show HEAD:…`），且全仓库 `src/`、`tests/` 已无 `AgentTeamPanel` 引用（`grep -rn "AgentTeamPanel" src/ tests/` → 无输出）。消息区模板 [HomeWorkspace.vue:1663](src/components/HomeWorkspace.vue#L1663) 现为 `<GoalBar/>` 紧接 `<Teleport>`，无 Team 面板。`✓`
- **排序切换控件存在**：`sortCreatedExists=true`、`sortProgressExists=true`（`data-testid="team-sort-created"` / `"team-sort-progress"`，源码 [TeamTaskBoardPanel.vue:88](src/components/team/TeamTaskBoardPanel.vue#L88) / [:99](src/components/team/TeamTaskBoardPanel.vue#L99)）。`✓`
- **数据量**：`memberItemCount=5`、`taskItemCount=7`。`✓`
- **无 Vite 错误覆盖层**：`viteErrorOverlayPresent=false`。`✓`
- **三态互斥（roster ↔ tasks 部分）**：原始 `data-open` 序列
  `[["team-roster-toggle","false"],["team-task-board-toggle","false"]]`
  → 点 roster → `[["team-roster-toggle","true"],["team-task-board-toggle","false"]]`
  → 点 tasks → `[["team-roster-toggle","false"],["team-task-board-toggle","true"]]`
  两块面板从不同时为 `true`。`✓`
  （`plan` 分支见 §9 未验证项）

### 5.4 Console Error = 0 取证（含仪器阳性对照）

工具确认：`agent-browser console` / `agent-browser errors` 存在专用子命令，可用；另有 `--json` 机器可解析输出。

**第一步：阳性对照（证明仪器真的会抓到错误）**

```bash
$ agent-browser eval "console.error('VERIFIER_CONSOLE_PROBE_ERROR'); 'injected'"
"injected"
$ agent-browser console | tail -1
[error] VERIFIER_CONSOLE_PROBE_ERROR          # console 通道确实捕获 console.error

$ agent-browser eval "setTimeout(()=>{throw new Error('VERIFIER_UNCAUGHT_PROBE')},0); 'scheduled'"
"scheduled"
$ agent-browser console | tail -1
[error] [pony-agent][window-error] {stack: "Error: VERIFIER_UNCAUGHT_PROBE\n    at <anonymous>:1:23", message: "VERIFIER_UNCAUGHT_PROBE"}
$ agent-browser errors
✗
```

**第二步：清理 → 重载 → 完整交互（展开队友 → 展开任务看板 → 切 progress → 切 created）→ 读数**

```bash
$ agent-browser console --clear
$ agent-browser reload && sleep 4
$ agent-browser find testid "team-roster-toggle" click
$ agent-browser find testid "team-task-board-toggle" click
$ agent-browser find testid "team-sort-progress" click
$ agent-browser find testid "team-sort-created" click
$ agent-browser console --json > /tmp/verify_console_final.json
$ agent-browser errors  --json > /tmp/verify_errors_final.json
```

`console --json` 直方图（node 解析原始 JSON）：

```
total messages: 14
by type: {"info":11,"debug":2,"warning":1}
type===error count: 0
type===warning count: 1
  WARN> [pony-agent][perf] long-task {name: "self", duration: 339, startTime: 369.09999990463257, page: "home", sessionId: "local-dev-session", ...}
```

`errors --json` 原始返回：

```json
{"success":true,"data":{"errors":[{"column":22,"line":0,"text":"Error: VERIFIER_UNCAUGHT_PROBE\n    at <anonymous>:1:23","url":null}]},"error":null}
```

**判定**：`Console Error = 0`（`type==="error"` 计数为 0）。`errors[]` 中唯一一条**可被文本指纹直接归因于验收者自身的阳性对照注入**（`VERIFIER_UNCAUGHT_PROBE`、`line:0 column:22`、`url:null`，即 `eval` 注入位置），且 `errors --clear` 对该缓冲不生效（`--clear` 后立即读取仍返回同一条，属工具行为）。**不存在任何来自被测应用的页面错误。** 唯一 warning 是应用自身的 `[pony-agent][perf] long-task` 性能监控日志，非错误。

### 5.5 dev server 生命周期与端口释放

```bash
$ job_kill <npm run dev 的后台 job id>
$ sleep 3; ss -ltnp | grep 4176
port 4176 FREE (no listener)
$ ps aux | grep "vite --configLoader native"
(none)
```

已确认 **端口 4176 无监听进程**，`npm run dev` 进程已完全退出（另有一个与本项目无关的 `job_copilot` vite:5174 长期进程，非本轮产物）。浏览器会话已 `agent-browser close` 关闭。

---

## 6. 视觉合规审查（独立评审意见，对照 PlanPanel.vue 逐行核对）

权威模板：[src/components/PlanPanel.vue](src/components/PlanPanel.vue)

### 6.1 「文件:行号 → 结论」对照表

| 规范项 | PlanPanel.vue（权威） | TeamRosterPanel.vue | TeamTaskBoardPanel.vue | 结论 |
|---|---|---|---|---|
| 折叠外壳 | `:164` `section.collapsible-shell border-b border-stone-200/60 pb-4` + `:data-open="open"` | `:43` 逐字一致 | `:55` 逐字一致 | ✅ 复用 |
| 折叠标题 | `:171` `text-[11px] uppercase tracking-[0.18em] text-stone-500` + `h-3.5 w-3.5` 图标 | `:50` 逐字一致（Users 图标） | `:62` 逐字一致（ListChecks 图标） | ✅ 复用 |
| 计数徽标 | `:176` `text-[10px] leading-[1.2] text-stone-400` | `:55` 逐字一致 | `:67` 逐字一致 | ✅ 复用 |
| 折叠箭头 | `:179-181` `h-3.5 w-3.5 shrink-0 text-stone-300 transition duration-200` + `rotate-90` | `:56-58` 逐字一致 | `:70-72` 逐字一致 | ✅ 复用 |
| 折叠体 | `:186-187` `collapsible-body` / `collapsible-content mt-2 space-y-2` | `:63-64` 一致 | `:77-78` 一致 | ✅ 复用 |
| 折叠动画 CSS | `:350-370` `grid-template-rows 0fr→1fr`、`260ms cubic-bezier(0.2,0.72,0.18,1)`、`opacity 180ms ease` | `:107-127` 逐字一致 | `:160-180` 逐字一致 | ✅ 复用 |
| 空态 | `:195` `px-1 text-[10px] leading-5 text-stone-400` | `:67` 逐字一致 | `:108` 逐字一致 | ✅ 复用 |
| 卡片 | `:218` / `:301` `rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2` | `:75` 逐字一致 | `:118` 逐字一致 | ✅ 复用 |
| 主体正文 11px | `:330` `text-[11px] leading-4` (+ `:331` completed→`text-stone-400 line-through`) | `:92` `text-[11px] text-stone-700`（**缺 `leading-4`**） | `:131-132` `text-[11px] leading-4` + line-through | ⚠️ 1 处 nit（见 §8 O3） |
| 次级说明 9px | `:264` `text-[9px] leading-3 text-stone-400` | `:95` `text-[9px] uppercase tracking-[0.1em] text-stone-400`（角色标签） | `:140` `text-[9px] leading-3 text-stone-400` | ✅ 尺度内 |
| 底栏元信息 | `:291` `text-[9px] uppercase tracking-[**0.12em**] text-stone-400` | — | `:147` `text-[9px] uppercase tracking-[**0.1em**] text-stone-400` | ⚠️ 0.02em 偏差（见 §8 O4） |
| 状态圆点 | `:323-328` `h-1.5 w-1.5 shrink-0 rounded-full **transition-colors duration-200**` | `:86-91` 同尺寸/形状，**缺 transition-colors** | `:124-129` 同尺寸/形状，**缺 transition-colors** | ⚠️ nit（见 §8 O3） |
| 状态点配色 | `:95-104` emerald-500 / amber-500 / stone-300 | `:22-34` emerald-500 / **emerald-400** / amber-500 / **rose-400** / stone-300 | `:31-39` emerald-500 / amber-500 / stone-300 | ✅ 均在 stone/amber/emerald/rose 内 |
| 字号尺度占用 | 12 / 11 / 10 / 9 px | 11 / 10 / 9 px（`grep text-\[12px\]` → 无） | 11 / 10 / 9 px（同） | ✅ 未越界（见 §8 O2） |
| tracking 取值 | 0.18em / 0.12em / 0.1em | 0.18em / 0.1em | 0.18em / 0.1em | ✅ 同族 |

### 6.2 颜色克制性（机器扫描）

```bash
$ grep -nEi "gradient|#[0-9a-f]{6}|bg-(red|blue|green|purple|sky|indigo|violet|fuchsia|cyan|teal|lime|yellow|orange|pink|slate|gray|zinc|neutral)-|..." src/components/team/*.vue src/components/team/*.ts
src/components/team/TeamRosterPanel.vue:75:          class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
src/components/team/TeamTaskBoardPanel.vue:118:            class="rounded-[0.55rem] border border-stone-200/80 bg-[#fbf8f3] px-3 py-2"
```

- **无 `gradient`（无渐变）、无高饱和色块、无阴影/ring/glow**：扫描仅命中规范指定的 `bg-[#fbf8f3]` 卡片底色。
- **配色族全部落在 stone / amber / emerald / rose**：Roster 用到 `emerald-500/400`、`amber-500`、`rose-400`、`stone-300/200/400/500/600/700`；TaskBoard 用到 `emerald-500`、`amber-500`、`stone-300/200/400/500/600/700`；排序按钮态用 `stone-700/600/400/300`。
- 唯二的 hex 字面量即 PlanPanel 同款卡片底色，非新增配色。

### 6.3 任务垂直排列（非 grid 多列）

- [TeamTaskBoardPanel.vue:114](src/components/team/TeamTaskBoardPanel.vue#L114)：`<div v-else class="flex flex-col gap-1.5" data-testid="team-task-list">` → 单列纵向堆叠。
- 文件内 `display: grid` 仅出现于 `:161/:172` 的 `.collapsible-shell` 折叠机制（`grid-template-rows: 0fr→1fr`），与 PlanPanel 的折叠实现逐字相同，**不是**多列任务网格。
- 截图 `team_right_sidebar.png` / `team_sort_progress.png` 目视：7 张任务卡片单列垂直排列。`✓`

### 6.4 迁移完整性

- `src/components/chat/AgentTeamPanel.vue` 已删除（`git status` → `D`）。
- [HomeSidebar.vue:106-107](src/components/HomeSidebar.vue#L106) 挂载两块新面板，`activePanel` 单一 ref（`"plan" | "roster" | "tasks" | ""`，[:33](src/components/HomeSidebar.vue#L33)）实现互斥折叠，`toggleRoster`/`toggleTasks` 位于 [:54-60](src/components/HomeSidebar.vue#L54)。
- [HomeWorkspace.vue:1663](src/components/HomeWorkspace.vue#L1663) 消息区顶部现为 `<GoalBar/>`，其后直接是 `<Teleport>`。
- `src/stores/team.ts` 新增 `sortMode`、`sortedTasks`（deleted 过滤 + 两种模式）、`setSortMode`；`src/types/team.ts` 新增 `created_at_ms: number` — 与 Rust 侧 DTO 字段名一致（serde `snake_case`，`created_at_ms` 无需重命名）。

---

## 7. 预览夹具隔离证据（`src/components/team/team-preview-fixtures.ts`）

**门控实现原文**（[team-preview-fixtures.ts:16-18](src/components/team/team-preview-fixtures.ts#L16)）：

```ts
/** 仅浏览器 dev 预览生效：Tauri 运行时（含 dev）永不启用。 */
export function shouldUseTeamPreview(): boolean {
  return import.meta.env.DEV && !isTauriAvailable();
}
```

`isTauriAvailable()` 原文（[src/lib/tauri.ts:12-18](src/lib/tauri.ts#L12)）：

```ts
export function isTauriAvailable() {
  if (typeof window === "undefined") {
    return false;
  }

  return Boolean(window.__TAURI__ || window.__TAURI_INTERNALS__);
}
```

**调用点门控**（store 真数据永远优先，夹具仅兜底）：

- [TeamRosterPanel.vue:14-20](src/components/team/TeamRosterPanel.vue#L14)：`teamStore.members.length > 0 ? teamStore.members : shouldUseTeamPreview() ? TEAM_PREVIEW_MEMBERS : []`
- [TeamTaskBoardPanel.vue:19-25](src/components/team/TeamTaskBoardPanel.vue#L19)：`teamStore.tasks.length > 0 ? teamStore.sortedTasks : shouldUseTeamPreview() ? sortTeamTasks(TEAM_PREVIEW_TASKS, teamStore.sortMode) : []`
- 夹具为纯常量 + 纯函数，**不写 store、不发 IPC**：`grep -n "useTeamStore\|safeInvoke\|invoke(\|fetch(" src/components/team/team-preview-fixtures.ts` 仅命中第 12、155 行的**注释**，无任何调用；文件 import 仅两条（`isTauriAvailable` 值导入 + `TeammateMember`/`TeamTaskItem` 类型导入）。

**生产构建实测（机器原生收据）**：`npm run build` 产物 `dist/assets/index-CGgI45E7.js` 中的原文片段：

```js
function tR(){return!1}
var nR=[{target:`lead`,name:`总指挥 Lead`,role:`lead`,status:`running`,description:`编排与最终裁决`}, ...]
```

结论（**基于构建期常量折叠的物理证据，而非推理**）：

1. `import.meta.env.DEV` 在 production 构建中被静态替换为 `false`，`shouldUseTeamPreview()` 被折叠为 **`function tR(){return!1}`** —— 即生产/打包产物里该函数**恒为 false**，`tR()` 之外无第二条通往夹具的路径；
2. 因此 `TEAM_PREVIEW_MEMBERS` / `TEAM_PREVIEW_TASKS` 在生产运行时**不可达**；Tauri 运行时（`npm run dev:tauri`：DEV=true 且 `window.__TAURI__` 存在 → `!isTauriAvailable()=false`）同样不可达；
3. **唯一可达环境是浏览器 dev**（DEV=true 且无 Tauri 全局）——与本轮截图场景一致。
4. 附带观察：夹具的只读常量数组**未被 DCE 摇树移除**，仍以几十字节死数据形式留在生产 bundle 中（见 §8 O1）。这是体积/清洁度问题，**不构成运行时数据路径污染**。

---

## 8. 观察项（非致命，不阻塞 PASS）

- **O1（代码卫生）**：生产 bundle 残留不可达的预览夹具常量（`dist/assets/index-CGgI45E7.js` 内 `var nR=[{target:\`lead\`…}]`）。运行时被 `tR(){return!1}` 硬隔离，无功能影响；建议后续把夹具移入 `tests/` 或 `import.meta.env.DEV` 动态 import 以彻底摇树。
- **O2（视觉一致性）**：两块新面板未使用 PlanPanel 的 `text-[12px]` 层级（新面板内无「卡片级标题」这一语义层，主体文本落到 11px）。所用 11/10/9px 全部取自同一尺度，未越界；如需与 PlanPanel 的标题层级完全对齐，可在卡片首行引入 12px。
- **O3（视觉 nit）**：`TeamRosterPanel.vue:92` 的队员名 `text-[11px] text-stone-700` 缺少 PlanPanel:330 的 `leading-4`；两块面板的状态圆点（Roster:86-91、TaskBoard:124-129）缺少 PlanPanel:324 的 `transition-colors duration-200`。
- **O4（视觉 nit）**：`TeamTaskBoardPanel.vue:147` 底栏元信息用 `tracking-[0.1em]`，PlanPanel:291 同语义层用 `tracking-[0.12em]`，差 0.02em。
- **O5（测试覆盖缺口）**：`tests/` 下无任何对 `TeamRosterPanel` / `TeamTaskBoardPanel` / `team-preview-fixtures` / store `sortMode`/`sortedTasks` 的单元测试（`grep -rln` → 无输出）。行为由本次浏览器 E2E + Rust 契约测试覆盖；建议补 `sortedTasks` 的 vitest 用例。
- **O6（无关遗留）**：`Cargo.lock` 工作区已存在 `pony-agent 0.1.151→0.1.152`、`pony-agent-core 0.1.125→0.1.126` 的改动，属上一次发布版本号同步遗留，与本轮排序契约无关，不影响任何门禁结果。
- **O7（文件权限）**：`src/components/team/*` 权限为 `0600`，仓库其他源文件为 `0644`（不影响构建与运行，仅为一致性提示）。

---

## 9. 未验证项（如实标注，不得视为已放行）

- `[UNVERIFIED: plan 分支的三态互斥]` 本会话为空白首页，[HomeSidebar.vue:100](src/components/HomeSidebar.vue#L100) 的 `v-if="hasPlanOrTasks"` 使 PlanPanel 未挂载（`data-testid="plan-panel-toggle"` 在 DOM 中不存在，`find testid` 报 `Element not found`），故 **plan ↔ roster/tasks 的互斥未能在浏览器物理触发**。已验证 roster ↔ tasks 互斥（§5.3）与 `activePanel` 单 ref 代码结构（HomeSidebar.vue:33/54-60/106-107）。
- `[UNVERIFIED: 空态渲染]` `team-roster-empty` / `team-task-empty` 未在浏览器触发（夹具恒返回数据）。仅有静态代码核对（Roster:65-71、TaskBoard:106-112）。
- `[UNVERIFIED: Tauri 真实运行时]` 未启动桌面端；「Tauri 下夹具不可达」的证据是构建期常量折叠 + `isTauriAvailable()` 实现，而非 Tauri 运行时实测。
- `[UNVERIFIED: 真机 IPC 排序]` 浏览器排序切换验证的是夹具数据 + 复刻排序函数；真实 IPC 数据路径下的排序由 Rust 侧契约测试（§4）独立覆盖，二者未在同一运行时串联验证。
- `[UNVERIFIED: 消息区以外的其他入口]` 仅核验了 Home 工作区；未遍历全部路由/会话态下 `AgentTeamPanel` 的历史挂载点（已确认 `src/` 内零引用）。

---

## 10. 验收过程副作用声明（如实披露）

1. **误操作导致共享构建产物被清理**：为做 §3 的 HEAD 基线对照，我用 `CARGO_TARGET_DIR=/home/dm/pony-agent/target` 在临时 worktree 中编译，该操作向共享 `target/` 写入了 HEAD 版本产物并覆盖了当前版本的 rlib（进而导致 §4.3 的 E0609 误报）。修复时执行 `cargo clean -p pony-agent-core`，**移除 52.9 GiB 构建产物**（`target/` 从 69G 降至 18G）。随后在冻结修订上重编并重跑全部 Rust 门禁（G4/G5/G6 均 Exit 0）。**未修改任何产品源代码**。副作用：其他成员若需跑 Rust 测试将触发一次冷编译。
2. **临时 worktree 已清理**：`git worktree remove --force /tmp/pony-head-baseline` 执行成功，`git worktree list` 现仅剩 `/home/dm/pony-agent`（main）与既有无关 worktree `/tmp/pa119-red`。
3. **一个 01:17 遗留的 agent-browser Chrome 会话（gpu-process 已累计 190 分钟 CPU）导致首次 CDP 全超时**；`agent-browser close` 后重建会话恢复正常，验收结束后已再次 `close`。
4. **dev server 已 kill，端口 4176 已释放**（§5.5）。
5. 验收者**未写入** `src/`、`crates/pony-agent-core/src/`、`src-tauri/` 任何文件；本轮验收者新增写入仅：`crates/pony-agent-core/tests/team_task_order_contract.rs`、`management/screenshots/*.png`、本报告。

---

## verdict

```json
{"verdict":"PASS","findings":[]}
```

> 说明：§8 的 O1–O7 均为**非致命观察项**（死代码体积、视觉 nit、测试覆盖缺口、无关遗留），不影响本轮契约正确性、门禁通过性与视觉合规判定，故 `findings` 为空数组。§9 的 4 项未验证内容已在报告中显式标注，未计入放行依据。
