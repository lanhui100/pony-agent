# Tasks

- [x] `HistoryNode` 扩展 `event_seq_range`；`commit_history_node_from_live_state` 记录事件区间，新节点不再内嵌快照。
- [x] `sqlite_session.rs`：节点持久化引用化（`event_seq_range_json` 列 + ALTER 迁移）+ legacy 行兜底读取。
- [x] `checkout_history_node` 重构：append `checkpoint/checkout` 事件（含 branch_id）+ 投影水位回退（legacy 节点走旧路径）。
- [x] 投影水位回退实现：从 init 重折叠到目标 seq + 节点 → 投影状态缓存 `(node_id, seq, state)`；10k 事件重折叠基准断言（debug 2000ms / release 100ms）。
- [x] **缓存失效协议**：节点 commit 后事件区间不可变 → 缓存天然有效（无失效需求）；MetricsProjection 不进 session 内存态 → 无跨缓存发散面（降级声明，见 Validation Notes）。
- [x] **分支可见性**：投影折叠按 `checkpoint/checkout` + 血缘图推导可见分支集合（初始 = 目标节点分支血缘链），跳过非可见分支事件。
- [x] fork 重构：`fork/created` 事件 + 事件前缀共享 + 新事件携带 `branch_id`（flush 闭包从 active_branch_id 落列）。
- [x] 时间旅行机制：`snapshot_for_session_at(node_id)` 折叠到节点 seq（克隆会话，不污染内存态）。
- [x] `cursor_version` 退役：wire 新增 `event_watermark`（HistoryCursor + normalized_history_cursor 列）；`expected_cursor_version` 保持旧语义（兼容）；前端 `history.ts` 零改动（wire 向后兼容）。
- [x] 单元测试：引用化升级、checkout 折叠 + 水位、legacy 兜底、时间旅行、fork 可见性（branch_id 过滤 + 不复活）、restore/fork 引用化折叠、10k 基准（8 个 pa093_*）。
- [x] 集成验证：store 级集成（fork→switch→restore 序列，sqlite backend 事件流）。
- [x] 回归：core 831 测试通过（含新增 8 个 pa093 测试）。

## Validation Notes

- 本 change 是 ADR 0008 阶段 3，依赖阶段 2（`session-projection-layer`）的投影层与阶段 1 的 `branch_id` 列。
- fork 先做"同 session 事件流 + 分支水位 + branch_id 过滤"；独立 session + seedLength（dsh 模式）作为备选，产品需要跨会话隔离时再引入。
- 时间旅行只做机制，前端入口后续迭代。
- `GraphRun` 推进事件化移出本 change scope（标注为后续 change）。
- 对抗审核（2026-08-18）已采纳：分支可见性规则（P1-1）、metrics 缓存同步回退（P1-2）、double checkout 场景（P1-10）、cursor_version wire 方案（P1-6）、git 回滚语义声明（A5）、GraphRun 移出 scope（P2）。
- **实施后对抗审核（2026-08-18，tester 详实）采纳与降级声明**：
  - **P0-1 裁决（降级声明）**：tasks 第 5 条"缓存失效协议 + metrics 同步回退"——实现采用"节点区间不可变 → 缓存天然有效"模型（projection_cache key 含 end_seq，节点 commit 后区间不再追加，无失效需求）；MetricsProjection 不进 session 内存态（session 无 metrics 字段）→ 无"history 已回退、metrics 停在撤回前"的跨缓存发散面。任务书措辞基于"持久化缓存表"假设，实际实现为内存缓存 + 不可变区间，语义等价且更简。
  - **P0-2 裁决（降级声明）**：`expected_cursor_version` 保持 cursor_version 相等校验（design 所述"与旧相等比较语义一致"即现状）；`event_watermark` 已 wire 面世（HistoryCursor + 快照），水位校验语义切换留待前端全量升级后（旧客户端不传则跳过，行为与现状一致）。
  - **P0-3 采纳**：store 级集成测试（fork→switch→restore 序列）补齐；control_plane 层集成测试保持 legacy 路径（memory_only backend 无事件流，属既有覆盖）。
  - **P1-1 采纳**：restore_branch_head / fork_from_history_node 引用化折叠测试补齐。
  - **P1-2 采纳**：分支可见性初始集合修正为"目标节点分支血缘链"（非固定 main）——修复 fork 分支事件在无 checkout 事件流下不可见的问题；流内 checkout 事件按序重放更新集合。
  - **P1-3 采纳**：测试隔离——唯一 session_id（tag 派生）+ RAII guard（Drop 时 clear 全局通道）+ 断言存在性而非顺序。
  - **P1-4 采纳**：double checkout 幂等测试（视图一致）。
  - **P1-5 采纳**：持久化 round-trip 测试（event_seq_range_json 重启存活 + checkout 重建）。
  - **P1-6 采纳**：flush 失败保持 legacy（注入后缀）+ 未知节点 Err 路径。
  - **P2-1 采纳**：10k 基准双阈值（debug 2000ms / release 100ms）。
- **实施状态**：core 831 测试通过（含新增 8 个 pa093 测试）。