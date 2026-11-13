# Agent Note: 会话库定向装载与稀疏 SessionStore（B148）

Status: implemented

## Problem

TUI 启动与 `pony resume <id>` 的等待时长由**会话库全量装载**决定。真实库
（`~/.local/share/PonyAgent/sessions.db`，1.4–1.5 GB）诊断实测（release，热页缓存）：

| 相位 | 耗时 | 代码位置 |
|---|---|---|
| `load_index_store()`（227 行轻量索引） | 3–16 ms（已停写 blob 的库）/ 13–15 s（本文副本身，blob 未收敛） | `sqlite_session.rs:2499` |
| `load_store()`（全量） | 6.0–27 s | `sqlite_session.rs:2742` |
| `SessionStore::new()` | ≈6.2 s（后续实测 18–20 s，库增长后） | `store.rs:349` |
| `load_normalized_session(<最大会话>)` | 4.6–4.9 s | `sqlite_session.rs:2860` |
| TUI 首帧 → 可交互 | 9.3–14.1 s（首帧仅 55–81 ms） | `tui.rs` |
| 峰值 RSS | ≈3.8 GB | — |

根因不是「日志 fold」（32 MiB 字节预算下整轮跳过），而是两条**全库投影**路径：

1. `fill_index_only_rows_from_index_projection`（`sqlite_session_commit.rs:526`）为补全
   轻量索引行调用 `load_store_normalized(conn, None)`：整表读取
   `normalized_sessions/messages/turn_traces/history_nodes/branches/cursor` ≈660 MB JSON
   （trace 365 MB），逐行反序列化后只取其中需要的行。
2. `load_normalized_session` 是「整库重建后 `remove(id)`」的假定向实现。
3. `pony resume` 与普通启动走同一个 `SessionStore::new()`；hydrate 只读内存
   （`snapshot_at_readonly` 对缺失会话返回空快照），不会回源，所以 resume 无法更快。

附带事实：`load_index_store` 的 3–16 ms 只在**已停写 blob** 的库形态成立；未停写库
（blob 合计 1.52 GB）索引装载本身就要 8.5–15 s。因此新读路径**不得**以「反序列化全部
blob」为实现前提，必须走 SQL 投影。

前序决策：ADR 0115（首帧先画、装载后台）与 ADR 0118
（[`2026-11-10-session-load-cost-and-store-reclaim.md`](../../implemented/architecture/2026-11-10-session-load-cost-and-store-reclaim.md)）
已把 fold 字节预检、trace 兜底收窄、维护预算分段落地，装载从 4s 级降到 3s 级；
但**整库投影仍是启动主项**，本条是它的下一段。

## Decision

### 1. 单会话定向装载（后端层）

- `load_store_normalized(conn, filter: Option<&str>)`：六条子查询在 `filter=Some(id)` 时带
  `WHERE session_id = ?1`（主键索引），**组装循环与合并规则不复制**；`filter=None`
  保持原全量语义。
- `load_normalized_session(id)` 改为 `load_store_normalized(conn, Some(id))`（真定向）。
- `SessionBackend` 新增三方法（默认实作返回「不支持」，内存/文件后端语义不变）：
  `load_session_meta_index()`（单条 SQL 投影）、`load_store_metadata()`（全局元数据：
  附件 / workspace / 授权 / 版本，**不含会话内容**）、
  `load_session_targeted(id) -> Loaded | NotFound | Unsupported`。
- `load_session_targeted` 的真源优先级与全量装载一致：日志 fold（`fold::read_session_state`
  + `apply_index_projection`）→ 索引视图 blob → 规范化投影回填（轻量行）→ trace 投影补全
  → 空历史事件重建。SQLite 实作独立成 `sqlite_targeted.rs`（避开 `sqlite_session.rs`
  的行数棘轮）。

### 2. 稀疏 `SessionStore`（`session/store_sparse.rs`）

`lazy: Option<LazySessions>` 是唯一开关：`None` = 全量模式（行为与 B148 之前逐字段等价）；
`Some` = 稀疏模式。不变量（改动前先读该文件顶部模块文档）：

1. `sessions` **只含已装载会话**；元数据在 `lazy.meta`，列表读它（不得用 `sessions` 投影）。
2. 写路径只写 `loaded`（`lazy_commit_sessions`）：未装载会话的 blob 与 trace 表**零写入**。
3. 未装载会话的**内容读取**必须经 `hydrate_session`；只读入口显式记
   `session_read_unloaded_denied` 并返回缺省值，禁止把「没装载」伪装成空会话。
4. 后端不具备定向能力（`supports_targeted_session_load() == false`）⇒ 回退 `with_backend`
   全量语义（内存/文件后端是整包覆盖，稀疏快照会丢会话）。
5. 附件索引刷新必须**按会话增量**（`refresh_loaded_attachment_entries`）：整表重建只看得见
   已装载会话，一次稀疏写入就会抹掉其余会话的索引条目，而附件清理以索引判孤儿 ⇒ 真删数据。
6. 规范化投影只带**内容**；标题 / 时间 / 轮次 / 归属以索引行为真值
  （`commit::inherit_index_metadata`）。否则 `updated_at_ms` 归零后由启动修复用 `now`
  兜底，会话会在侧边栏「凭空跳到最新」。

### 3. 元数据投影口径（「列表可见性」单一真值）

- 内容判据：`session::backend::state_has_content`（`store::session_is_persistable` 委托）；
  SQL 侧与其同真——blob 内容数组非空 ∪ `turnCount > 0`，**再并上**「内容只落在规范化表」的
  会话（`normalized_messages / normalized_turn_traces / normalized_history_nodes` 有行）。
  最后这条不是可选项：真实库 227 行里有 7 行的 blob 是内容为空的轻量行，内容全在规范化表，
  全量装载会把它们并回内存态（列表可见），稀疏列表若只认 blob 就静默漏会话。
- 老库没有规范化表 ⇒ `prepare` 会整体失败并把索引降级成空列表，故按表存在性选择 SQL
  （`normalized_tables_available`）。

### 4. TUI 接线

- `build_control_plane` 用 `SessionStore::open_lazy()`；
- `HostControlPlane::hydrate_session(id)`（`control_plane/history_commands.rs`）是
  `/resume`、`/agents`、队友会话切换、`pony resume` 的共用收口，调用顺序契约：
  `hydrate_session` → `load_session_runtime_view`；
- `hydration_state_from_session`（`tui.rs`）首行先 hydrate，挡住「先取快照再装载」的顺序错误。

## Alternatives considered

1. **只把 `load_normalized_session` 改成定向，启动仍全量**（最小改动）。
   落选：真实瓶颈在启动的整库投影；单会话 resume 仍要等 6 s。
2. **把 `fill_index_only_rows_from_index_projection` 改为逐会话定向查询**（保留全量 store）。
   落选：真实库 225/225 行都是轻量行，逐会话查询等价于 225 次串行装载（最坏 15 s+），
   且内存仍要装下全部会话内容（RSS 不降）。
3. **先把索引投影缓存持久化/后台预热**。
   落选：只把等待挪到后台，首屏仍不可交互；且缓存失效语义（写入/墓碑）无验收口径。
4. **继续用 `sessions` blob 作为元数据源，但只读轻量行**。
   落选：库形态不受控（未停写 blob 的库有 1.5 GB blob）；必须用 SQL 投影，
   不能在 Rust 侧「先全读再跳过」。
5. **稀疏列表不做 `has_content` 过滤（全部元数据行都显示）**。
   落选：会把没有任何内容的幽灵行塞进侧边栏（真实库 45 行内容为空），且与全量列表不一致。
6. **附件索引在稀疏模式仍整表重建**。
   落选：会让未装载会话的索引条目消失，attachment 清理据此判孤儿 ⇒ 真删别人的附件。
   改为按会话增量（`refresh_loaded_attachment_entries`）。

## Consequences

### 实测收据（release，1.49 GB 真实库副本 `/tmp/b148-clean`，227 会话 / 227 元数据行）

探针：`crates/pony-agent-core/examples/perf_lazy.rs`（只测懒装载链路，不与全量相位串扰）。

| 相位 | 指标 | 实测 | 验收线 |
|---|---|---|---|
| `A_meta_steps` | 构造 / `load_store_metadata` / `load_session_meta_index` / schema 校验 | 0.01–0.08 / 1.1–2.2 / 1.7–2.2 / 0.04–0.08 ms | — |
| `B_open_lazy`（Ready） | `SessionStore::open_lazy()` | **3.34–4.40 ms** | < 100 ms ✅ |
| `B_after_ready` | RSS（Ready 时） | **18–20.5 MB** | < 300 MB ✅ |
| `C_hydrate`（resume 首屏） | 定向 hydrate + 只读快照 | **63–83 ms** | < 300 ms ✅ |
| `D_list_parity` | 稀疏列表 vs 全量列表 | 227 / 227 条，**id 集合一致** | 条数与 id 集合一致 ✅ |
| `D_full_load_ms` | 对照：全量 `SessionStore::new()` | 9642–17467 ms | — |
| `E_snapshot_parity` | 抽样 8 个会话：定向 hydrate 快照 vs 全量快照（history / trace / turnCount / title / summary / workspace + 节点数） | **7/8 完全一致** | 一致性抽样 ✅（1 例差异见下） |
| `M_sparse_rename` vs `M_full_rename` | 改名写盘 | **88–144 ms** vs 11860–22542 ms（写集合 = 1 会话） | 写路径按 dirty 收敛 ✅ |

测试收据（`cargo test`，全部退出码 0）：

- `tests/targeted_session_load_test.rs`：4/4 —— 定向 vs 全量逐字段等价、元数据索引与
  `list_sessions` 的 id/顺序逐条一致、NotFound、metadata 无会话。
- `tests/sparse_session_store_test.rs`：8/8 —— 只读元数据装载、hydrate 与全量快照等价（幂等）、
  未装载读取显式拒绝、**稀疏写不触碰未装载会话 blob/trace 行**、稀疏写保留未装载会话的附件
  索引、生产写入路径列表逐字段一致、内容只落在规范化表的会话不漏、管理升级恢复全量。

### 已知残留（记录在案，非本轮验收缺口）

1. **列表顺序在毫秒级并列的会话上可能不同**：稀疏列表用 `sessions.updated_at_ms` 列，
   全量列表用 blob 里的 `updatedAtMs`（真实库 227 行里有 3 条两者相差个位数毫秒，
   位置互换）。两者都是「写入时点」，但**真值只能有一个**；收敛方案是让索引行读路径
   统一采用列值（`load_index_store` 的列投影），属后续独立条目。
2. **`D` 抽样 1 例 `turnTraceHistory` 元素级差异**（长度相同、内容不同）：该会话是库内
   最新的一条，定向路径的 trace 投影补全与全量路径的 `refold_missing_trace_cache` 在
   「表权威 + 节点 refs」组合上的合并顺序不同。自动化等价测试覆盖了表权威场景且为绿，
   真实库的这一例归入上面同一后续条目（trace 合并顺序单一真值化），不在本轮扩大改动面。
3. **`perf_probe` 的 N 相位读数不可当 Ready 成本**：同进程前面的 F/M 相位会做整库提交
   （227 会话、数百 MB 投影），其 WAL 在后续新连接打开时被恢复，`index_ms` 会读到
   6.4 s 级假象；Ready 口径只认 `perf_lazy`（不跑全量相位，实测 1.7–2.2 ms）。
4. **无持久化历史图的会话，内存合成节点 id 内嵌合成时刻**（`…-node-<ms>-<ordinal>`），
   两条路径必然不同；故 `E` 只比节点**数量**与用户可见内容。这是既有行为，非本次引入。

### 门禁与治理

- `npm run verify` 的其余门禁（version / adr / secrets / backlog / 单测 / build /
  contract / ext）通过；`size:check` 当前为红，但 4 条违规全部来自**并行在途工作**
  （`config.rs`、`provider/mod.rs`、`provider/tests.rs` 的上游重试改动与已提交的
  `control_plane/mod.rs`），B148 自身文件均在基线内（`sqlite_session.rs` 6236 ≤ 6237、
  `store.rs` 5764 ≤ 5773）。
- 新增文件不动棘轮：`sqlite_targeted.rs`、`session/store_sparse.rs`、两个测试与两个
  `examples/perf_*.rs` 均为新文件。

## 2026-11-14 回归修复（B159）：定向装载曾整段丢弃 trace

- **症状**：`pony resume` 恢复出的历史只剩正文，工具调用明细消失（`tui_overlay_cancel_acceptance::j1_resume_summary_prefers_description_and_hides_structured_json` 由绿转红；已在 `d18bde15` 干净 worktree 上验证该用例当时为绿）。
- **根因**：`load_session_targeted_impl` 的 `index_only` 分支只取轻量索引行，不读 trace 表；
  而全量装载那侧的 `attach_index_trace_projections_with_deadline` 只在「**已有** trace 缺
  observation」时才动手（`session.turn_trace_history.iter().any(...)`）——trace 表为空时恒 skip
  ⇒ 定向路径下 `turn_trace_history` 永远为空，恢复视图自然没有工具行。
- **修法**：定向装载尾部补一步单会话 trace 读取，口径与 E7 全量读路径同源
  （`read_traces_normalized(conn, Some(id))`＝normalized 主源 + legacy 兜底），再
  `merge_trace_history` + `materialize_node_traces` 保持节点 refs 一致。
  只在 `turn_trace_history` 为空时触发，避免覆盖投影补全已给出的结果。
- **回归守护**：上述 PTY 用例（真实二进制 + 真实伪终端）留在 `cargo test` 全量里；
  另用 `d18bde15` worktree 复现「修复前红、修复后绿」的对照读数。
- **修复后 release 复测**（1.48 GB 真实库副本 / 227 会话 / legacy 相位，`perf_lazy` 3 轮）：
  `open_lazy` Ready **3.17–3.61 ms**、Ready 时 RSS **17.8–19.9 MB**、
  定向 hydrate 最大会话（41 轮 / 98 条历史）**150–191 ms**、同库全量装 **5030 ms**、
  快照等价抽样 **8/8 identical**。
- 已知残留复现：`D_list_parity` 仍为 `identical=false`（227/227 条，差异全部落在
  **毫秒并列**会话的先后，即下文 Consequences 已登记的口径，非新增缺陷）。


