# Agent Note: 会话标题三线模型（用户改名 > AI 标题 > 派生标题）

Status: implemented

## Problem

此前会话只有一个标题字段 `title`，取值是**派生标题**：取首条真实用户消息的
首行、压平空白后截断 28 字（`store.rs::derive_title_from_history` 族）。三个真实缺口：

1. **不可读**：长 prompt 被砍成半句。本机实测大量会话标题停在
   `最近有一个正在修复/resume缺少历史会话问题，且实现...`，`/resume` 里同一目的的
   会话彼此不可区分——标题没有承担「选哪个」的职责。
2. **用户改名无处落地**：`title_override` 字段早已存在，但没有任何生成/展示链路认它；
   用户改名后一旦 `title` 被重投影就被覆盖。
3. **AI 概括无处存放**：想在首轮后补一条「这次会话在做什么」的短标题，就必须覆写
   `title`，而 `title` 同时是**派生真值**（可再生产物、归档/回放比对基准）。用可再
   生产物承载一次性的模型输出，会让「回放同一历史得到同一标题」这条不变量失效。

## Decision

### 1. 三线并存 + 单一投影点

`SessionState` 增 `ai_title: Option<String>`，与既有 `title_override`（用户线）、
`title`（派生线）并存。**优先级只在两处判定**，其余所有展示位只读投影：

- `types.rs::effective_title()`（types.rs:459）：`title_override > ai_title > title`；
- `types.rs::title_source()`（types.rs:470）：返回 `TitleSource::{User,Ai,Derived}`
  （types.rs:93），供列表/面板展示来源徽标。

`SessionOverview` 同步带出 `ai_title` + `title_source`，TUI `/resume`、桌面侧边栏、
CLI 都从同一投影取值，不各自拼优先级。

### 2. 生成侧独立模块，锁与时限纪律对齐 compaction

`agent/runtime/title_ops.rs`（309 行）承载生成，控制面入口
`agent/control_plane/title_commands.rs`（66 行）。硬约束：

| 约束 | 取值 | 理由 |
|---|---|---|
| 墙钟上界 | `TITLE_TIMEOUT_SECS = 30` | 标题是短任务，明显短于 turn 流式上界，TUI 不会永久挂在「正在生成标题…」 |
| 锁纪律 | 短读快照 → **无锁**网络调用 → 短写回写 | 网络调用期间不持 `sessions` 读写锁（与 `compaction_ops` 同构） |
| 上下文 | 最早 8 条消息 × 每条 ≤600 字 | 标题取决于「开场」，长文不得撑爆 prompt |
| 输出 | 归一化后 ≤40 字（`AI_TITLE_MAX_CHARS`） | 比派生标题的 28 宽，留给「动作+对象」 |
| 失败 | 空/纯壳一律 `Err(empty_title)` | **绝不用空标题覆盖历史**（不谎报成功） |

错误码前缀可辨识：`title_deadline_exceeded` / `no_user_message` / `empty_title`。

### 3. 触发面

- `/title`（tui.rs:10744）：后台线程重生成，成功/失败均回 `UiEvent::SessionTitleFinished`；
  `is_streaming` 时直接拒绝（`set_status_notice("本轮结束后可用 /title")`），不抢轮次。
- 首轮结束后 `maybe_generate_session_title`（tui.rs:11254）自动补一次；已存在
  `title_override` / `ai_title` 时不覆盖（幂等，避免每次启动把用户命名冲掉）。

### 4. 存储面贯通

`ai_title` 随 `SessionState` 进 blob 与规范化行，`sqlite_session_commit` / 稀疏投影 /
`store_history_view` / `fold` / `log_migrate` 全链路携带并回填旧库。
`SessionStore::set_ai_title`（store.rs:2385）同时把新标题镜像到冻结基准 `title`：
旧读路径（不认识 `ai_title` 的消费方）不会显示空白，同时 `effective_title()` 仍能
表达真实来源。

## Alternatives considered

- **只做用户改名，不做 AI 标题**：改名解决「用户知道自己在哪」，解决不了「用户不记得
  这堆会话谁是谁」；首轮后自动概括正是 `/resume` 的可读性缺口所在，故纳入同一次交付。
- **落盘时同步生成标题（不另起字段）**：把一次 LLM 调用塞进 turn 提交边界，既拉长提交
  又把网络失败面接进数据面；且标题是「锦上添花」，不该有能力影响会话可提交性。
- **直接覆写 `title`（不引入 `ai_title`）**：破坏「派生标题可再生」——回放同一历史应得到
  同一 `title`，而模型输出不稳定；同时无法表达来源徽标（用户分不清是自己改的还是 AI 起
  的），改名还会被下一次重投影吞掉。
- **各展示位自己拼优先级**（TUI 一套、桌面一套）：与「每个事实一个家」冲突，三条标题线
  的优先级一旦不一致，用户在不同界面看到不同标题。

## Consequences

- 收益：`/resume`/侧边栏标题从「prompt 前 28 字」升级为「一句话说清在做什么」；用户改名
  有了权威落点（`TitleSource::User` 可显示）；派生标题回归可再生产物，不再被模型输出污染。
- 代价：会话对象多一字段 + 一次可选的短 LLM 调用（30s 上界、只在首轮后与手动 `/title`）；
  旧库需迁移回填（迁移路径与新字段同提交）。
- 测试：`ai_title`/`title_source` 字面量在 10 个既有测试文件里衍生补齐（**零断言改动**，
  只补字段）；`title_ops` 归一化/空输出/超时口径有单测覆盖；`cargo test --lib`、
  `cli_acceptance`、`tui_acceptance`、`tui_six_fixes` 全绿（收据见 backlog B186）。
- 残留：AI 标题只生成一次（首轮后）或手动重生成，**不随会话漂移自动刷新**——避免长期
  会话里标题反复跳动；若用户希望「跟随最新内容」，需另开任务定义刷新策略（抖动/成本权衡）。
