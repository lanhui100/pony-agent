# ADR 0113：TUI 双击 Esc 撤回（轮次级会话截断 + 写前 pre-image 文件还原）

Status: implemented

日期：2026-11-10
关联：[0099](0099-session-storage-event-sourced-refactor.md)（会话历史图 / checkout 真值路径，复用不改语义；
checkpoint 引用化见任务卡 PA-093，「按节点 checkout」抽象本次只读消费）、
[0063](0063-tui-ask-user-overlay-and-cancel-feedback.md)（覆盖层面板交互基线）、
[0071](0071-tui-streaming-input-queue-and-panels.md)（流式双击 Esc 停止，与本次非流式双击互不干扰）。
实施说明：[`.agents/notes/implemented/feature/2026-11-10-tui-double-esc-rewind.md`](../../.agents/notes/implemented/feature/2026-11-10-tui-double-esc-rewind.md)。

## 背景

Claude Code 的肌肉记忆是「连按两次 Esc → 撤回菜单」：把会话（以及它改过的文件）退回到某条用户消息
之前，并把那条消息回填到输入框，改完重发。本仓 TUI 此前只有三种 Esc 语义：流式中双击停止本轮、
静态下清空草稿、面板内关闭。**没有任何「回到过去」的入口**：`/clear` 换新 session 且不撤销文件，
`/resume` 只是切会话。

底层早已具备两个必要部件，但从未被 TUI 接线：

1. 会话侧：`HistoryNode`（每轮 `TurnCommitted`）+ `checkout_history_node`（事件水位回退、分支头前移、
   截断可持久），即 `restore_branch_head` / `fork` 同一套真值路径；
2. 文件侧：`WorkspaceSnapshotManager`（W32，仅有单测）与写工具的**读前写 CAS 门**——写路径本来就会
   读到旧内容，具备「落盘前留底」的天然时机。

## 决策

### D1 交互：非流式「双击 Esc」开面板，流式中双击语义不动

- 静态 Esc 保留原语义（撤销派发 / 清空草稿）**并上膛**；同会话 700ms 内的第二次才开面板
  （`RewindEscArm`：会话作用域 + 时间窗，命中即清膛）。
- 流式中双击 Esc 仍是「停止本轮」（500ms 窗口），两条判定互斥不叠加。
- 另给 `/rewind` 斜杠命令同入口（可发现性 + 键盘不可用时仍可达），并归入「会话变更类命令」，
  流式期拒绝（不得在飞轮次中途撕裂会话）。

### D2 面板：两段式（选轮次 → 选范围）

1. **轮次**：当前会话活动链上的已提交用户轮次，新→旧；行文案优先取该轮用户消息首行（比节点标题更
   接近用户记忆），右侧标出该轮抓到的可回滚文件数。
2. **范围**：`恢复代码与会话`（默认）/ `仅恢复会话`；范围段 esc 退回轮次段，轮次段 esc 关闭。

### D3 会话还原：只复用既有 checkout，不复制截断逻辑

撤回目标 = 目标轮次节点的**父节点**（状态 = 该轮之前）。父节点为空（撤回到第一轮之前）时，先由
`SessionStore::ensure_rewind_root_checkpoint` **幂等**补一个空历史 `Checkpoint` 根节点：把现有无父
节点挂到它下面、分支 base 指向它，再 checkout 到它。`HistoryCheckpointMode` 一律 `TranscriptOnly`
（文件由 D4 自己负责），避免用 `workspace_ref` 那套尚未接线的能力冒充「已回滚」。

### D4 文件还原：写工具落盘前的 pre-image，按轮次键存储

- 抓取点唯一：`tools::write_with_rewind_capture`（`workspace_write_file` / `workspace_edit_file` 的唯一
  写调用点替换为它，**行数中性**，规避 store/tools 的文件行数棘轮）。
- 键：`(session_id, turn_id)`（取自 `ToolExecutionContext.dispatch_context`；缺任一项不抓——没有轮次键
  就无从归属，绝不猜）。目录：`<data_local>/PonyAgent/rewind/<session>/<turn>/<sha256(abs_path)>.json`
  + `<hash>.bak`；元数据文件即幂等标记（同一轮同一路径只抓第一次）。
- **fail-open**：抓取失败只留一行 stderr，绝不阻断写入（撤回是便利能力，不是写门）。
  反向纪律：**还原失败必须显式上报**（`RewindRestoreOutcome` → 回执 → 状态行），不得当作成功。
- 还原口径：被撤销轮次按时间升序遍历，同一路径只认**最早**那份 pre-image（那才是区间起点之前的
  真实内容）；抓取时文件不存在 ⇒ 撤回复原时删除它。单文件 > 4MiB 只记 `oversized` 标记并计入降级，
  绝不写入猜测内容。
- 顺序：**先还原文件、后截断会话**——checkout 失败时用户至少拿回代码，且会话没有被半截断。

### D5 回填：撤回成功后输入框写入该轮用户消息

用户消息真值从核心取（`rewind_plan.user_message`，节点视角折叠后的最近一条非空 user 消息），
不依赖 TUI 内存消息的下标映射（历史回放/工具消息会打乱下标）。撤回成功后 TUI 走
`reset_session_scope` + `hydrate_state_from_session`（与 `/resume` 同口径重建消息区），再 `set_input`。

## 后果

- 行为：TUI 多了一条可发现、可取消、分两档的撤回入口；`/clear` 的「新会话」语义不变。
- 数据：新增本地快照目录（不改变 sessions.db schema、不新增网络出口、不上报）。**本次不做**磁盘配额
  与跨进程清理（旧轮次快照在撤回后成为孤儿，保留不还原；后续可加 TTL 清理）。
- 兼容：桌面端共用控制面新命令但未接线 UI；写路径只在既有 `fs::write` 外多一次幂等拷贝，
  无 turn 键（老调用方/子进程直调）时零行为变化。
- 已知边界：`workspace_run_command` 等**脚本内部**产生的文件改动不在快照范围（只能由模型经写工具
  的改动还原）；`oversized` 与抓取失败以降级回执显式暴露。

## Alternatives considered

- **A. 只用 git 回滚工作区**：非 git 工作区（大量用户目录）直接失效，且 `git checkout -- .` 会把用户
  自己的未提交改动一起抹掉——与「撤销 agent 这一轮的改动」语义不符。**否决**。
- **B. 复用 `WorkspaceSnapshotManager`（W32）做文件回滚**：按 `node_id` 抓取，但工具层只知道
  `turn_id`（节点 id 在 commit 时才生成），且它把备份存内存、无持久索引、无超限降级口径。
  **否决**（保留其作为未来「节点级」扩展点）。
- **C. 每轮开始全工作区快照**：大仓库不可接受（一次 turn 拷几 GB），且撤回粒度会变成「整棵树」，
  无法区分用户自己的改动。**否决**。
- **D. 只做会话撤回（不做文件）**：实现量最小，但用户的核心诉求是「把改坏的文件也退回去」，
  半个功能会立刻被要求补齐。**否决**（但把它保留为面板里的第二档，给「只想重说一遍」的场景）。
- **E. 撤回 = fork 新分支**：能保留历史，但用户心智是「回到过去」，fork 会留下两条对话线且
  输入框回填语义不清。**否决**（分支能力已由 `fork_from_history_node` 独立提供）。
- **F. 把类型与编排都塞进 `control_plane/mod.rs` 与 `session/store.rs`**：两文件都已在
  `scripts/file-size-baseline.json` 棘轮里超限。**否决**，新增逻辑落独立文件，仅做**行数中性**的可见性
  放开（`new_history_node_id` → `pub(super)`）与模块声明。
