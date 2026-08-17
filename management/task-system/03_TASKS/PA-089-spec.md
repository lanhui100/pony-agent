# PA-089 Spec: 会话存储规范化（拆表 + 增量写入 + 写队列 + 前端分页）

> v3（2026-08-16）：采纳 consultant v2 复审全部意见。
> 关键修复：Authoritative→DualWrite materialize 回 blob（P0-2 断点）、sessions 表名方案 A（并行表）、
> HistoryNode 快照模型、阶段 6a drain/freeze/barrier、回填逐字段契约、会话级 migration_state。

## 1. 背景与目标

**背景**：PA-088/090 已把最大会话从 43.77MB 压到 3.77MB，但存储模型仍是"单行大 JSON blob"：全量序列化写放大、无分页、单写者阻塞。

**目标**：规范化 schema + 增量写入 + 写队列 + 前端分页 + 平滑迁移（6 阶段，回滚能力随阶段递减）。

## 2. 范围与非目标

**范围内**：sqlite_session.rs、session.rs、control_plane、trace_persistence.rs、前端按需加载、迁移脚本。
**非目标**：不换引擎、不引入 DB 服务器、tool 大结果外置为可选阶段。

## 3. 技术方案

### 3.0 表名策略（P0 关键：方案 A 并行表）

**旧 `sessions` 表保留为 blob 表**（改名 `session_blobs` 推迟到阶段 6b），新增 `normalized_sessions` 等规范化表：
- 阶段 0-5：旧 `sessions`（blob）+ 新 `normalized_*` 表并行，旧代码零改动可运行
- 阶段 6b：确认稳定后，旧 `sessions` 改名 `session_blobs`（或删除），新表接管
- `PRAGMA user_version` 管理 schema 版本；`PRAGMA foreign_keys = ON`（连接级，DDL 含 REFERENCES ON DELETE CASCADE）

### 3.1 规范化 schema（v3 补全）

```sql
normalized_sessions (
    session_id TEXT PRIMARY KEY, workspace_id TEXT, title TEXT, summary TEXT,
    turn_count INTEGER, last_referenced_file TEXT,
    created_at_ms INTEGER, updated_at_ms INTEGER, state_version INTEGER,
    trace_migration_state TEXT,          -- 会话级（LegacyBlob/DualWrite/TraceTableAuthoritative）
    turn_trace_refs_json TEXT,           -- 顶层 refs
    provider_native_transcript_json TEXT,
    memory_json TEXT                     -- 记忆四件套（版本化 JSON）
);

turns (
    session_id TEXT, turn_id TEXT, ordinal INTEGER, phase TEXT, status TEXT,
    user_message_id TEXT, assistant_message_id TEXT,
    started_at_ms INTEGER, completed_at_ms INTEGER, created_at_ms INTEGER, updated_at_ms INTEGER,
    PRIMARY KEY (session_id, turn_id), UNIQUE (session_id, ordinal)
);

messages (
    message_id TEXT PRIMARY KEY, session_id TEXT, turn_id TEXT, ordinal INTEGER,
    role TEXT, content TEXT, reasoning_content TEXT, status TEXT,
    model_name TEXT, token_count INTEGER, attachments_json TEXT,  -- v3 补 attachments
    created_at_ms INTEGER,             -- nullable（源消息无此字段时 null）
    UNIQUE (session_id, ordinal), INDEX (session_id, turn_id, role)
);

turn_traces (
    session_id TEXT, turn_id TEXT, trace_order INTEGER, phase TEXT,
    provider_name TEXT, provider_model TEXT, provider_mode TEXT,
    session_summary TEXT, fallback_reason TEXT, error TEXT,
    input_tokens INTEGER, output_tokens INTEGER, total_tokens INTEGER,
    first_token_latency_ms INTEGER, turn_duration_ms INTEGER,
    updated_at_ms INTEGER, extension_json TEXT,   -- provider_call_records/hook_trace_records（版本化）
    PRIMARY KEY (session_id, turn_id)
);

trace_steps (session_id, turn_id, ordinal, kind, state, label, text, error, duration_ms, extension_json,
             PRIMARY KEY (session_id, turn_id, ordinal));
trace_timeline (session_id, turn_id, entry_id, sequence, kind, label, state, text, reasoning_content, duration_ms, extension_json,
                PRIMARY KEY (session_id, turn_id, entry_id), INDEX (session_id, turn_id, sequence));
tool_activities (activity_id PK, session_id, turn_id, timeline_entry_id, parent_activity_id, name, canonical_tool_name,
                 status, description, arguments_preview, result_preview, result_bytes, result_truncated, error_json,
                 duration_seconds, created_at_ms, INDEX (session_id, turn_id, created_at_ms));
history_branches (branch_id PK, session_id, base_node_id, head_node_id, forked_from_branch_id, forked_from_node_id, label, created_at_ms, updated_at_ms);
history_nodes (node_id PK, session_id, parent_node_id, branch_id, forked_from_node_id, kind, turn_id,
               turn_trace_refs_json, run_id, workspace_ref_json, summary, title, created_at_ms,
               snapshot_json,          -- v3：节点快照（history/transcript/memory/turn_count/last_referenced_file）
               INDEX (session_id, branch_id, parent_node_id));
history_cursor (session_id PK, visible_node_id, active_branch_id, branch_head_node_id, workspace_node_id,
                cursor_version, mode, checkout_mode, checkout_status);
```

**HistoryNode 快照模型（P0 关键）**：`history_nodes.snapshot_json` 存节点自己的 `history`/`provider_native_transcript`/`long_term_memory_entries`/`memory_write_evidence`/`memory_write_hook_trace_records`/`turn_count`/`last_referenced_file`——`hydrate_session_from_node`（session.rs:3346-3357）依赖这些字段，**不能从当前 session 代替**。snapshot_json 版本化 + 大小上限。

**message_id 派生**：`{turn_id}-{role}`（同 turn 多 role 追加 `-{n}`）；无 turn_id → `unknown-{role}-{ordinal}`；全量消息赋全局递增 ordinal。

**turns 重建规则（v3 补全）**：
- 按 turn_id 分组 + 每 turn 首个 user 消息 ordinal 确立
- 同 turn 多 user/assistant 消息：`user_message_id`/`assistant_message_id` 取**第一条**（关联表留待需要时）
- phase/status 优先级：trace（表）> message.status > 旧 blob 状态
- created_at_ms：表内最小消息时间或 null

### 3.2 增量写入（PersistCommand）

```rust
enum PersistCommand {
    AppendMessage { session_id, message },
    UpsertTurn { session_id, turn },
    AppendTrace { session_id, trace, trace_order },
    UpdateTraceTerminal { session_id, turn_id, terminal_patch },
    AppendHookRecords { session_id, turn_id, records },
    UpdateHistoryNode { session_id, node },
    UpdateCursor { session_id, cursor },
    UpdateSessionMeta { session_id, meta_patch },
    RemoveSession { session_id },
    PublishMetadata { key, value },
}
```
- **统一事务**：blob（旧 sessions 表）+ 规范化表同库，每个命令一个 `unchecked_transaction` 同时写两边——原子性天然可得
- **blob 兼容写入器**：命令本身不携带完整旧 blob，必须定义 blob 侧如何同步（阶段 3-5 双写期：blob 保持完整 DualWrite 语义）
- **SessionTraceMutation 兼容适配层**：`PersistCommand → SessionTraceMutation adapter`，旧 mutation 保留到阶段 6 后移除
- `save_store` 边界：迁移期双写 / 全量重建 / 数据修复 / 回滚恢复；正常 turn 热路径不得调用；**禁止 save_store 补偿写**

### 3.3 写队列（单写者 actor）

- 基于 `trace_persistence.rs`（1024 有界队列）扩展为通用持久化 actor
- coalesce：UpdateSessionMeta/UpdateCursor 保留最后；UpdateTraceTerminal 保留最后；AppendHookRecords 合并；其余按序
- 背压：durable 阻塞等待；diagnostic 丢弃；**禁止绕过 actor 直接写 SQLite**
- durable（消息/turn/节点）必须 ack；每 turn 收尾最后一个 durable 命令同步 ack（防崩溃尾损）
- 失败重试：指数退避 + dirty 集合重放

### 3.4 前端分页 API

```rust
list_sessions() -> Vec<SessionOverview>
load_session_overview(session_id) -> SessionOverview
load_messages_page(session_id, cursor, limit) -> MessagesPage   // { items, next_cursor, has_more, revision }
load_trace_window(session_id, turn_id, cursor, limit) -> TraceWindow
load_history_graph(session_id) -> HistoryGraph
```
- 游标不透明 token（内部编码 session/branch/ordinal/message_id/revision/direction）
- revision 复用 messageState delta 机制（防分页覆盖新消息）
- 前端迁移顺序：overview → 最近消息页 → graph → 滚动加载 → trace 独立请求
- 旧 `load_session_runtime_view` 保留兼容

### 3.5 迁移（6 阶段，v3 修正）

**执行主体与时序（v3 明确）**：
| 阶段 | 执行主体 | 必须条件 |
|---|---|---|
| 0 | 离线脚本 | 应用退出、backup、integrity_check、进程检测（fail-closed） |
| 1 | 离线脚本/受控启动 | 只做 DDL（normalized_* 表 + user_version），不触碰旧读写路径 |
| 2 | 离线脚本 | per-session 回填、checksum、幂等 marker |
| 3 | 运行时 | 迁移 phase 开启，所有写入口统一 DualWrite |
| 4 | 运行时/后台 | 新旧 loader canonical compare |
| 5 | 运行时 | 只切读，blob 继续双写 |
| 6a | 受控切换 | 写队列 drain + 切换点 backup + 停 blob 写 + 观察窗口 |
| 6b | 后续离线发布 | 验收窗口结束后删 blob 字段 |

**Authoritative→DualWrite materialize（P0-2 断点修复）**：
已剥离的 Authoritative 会话进入阶段 3 前，必须先：
1. 按顶层 refs + 节点 refs 从 trace 表 materialize 回 blob（完整 trace）
2. 校验 blob/table/refs 一致
3. 设置 session marker = `migration_dual_write`
4. 阶段 3-5 内 blob + 规范化表双写（**迁移阶段优先级高于 WriteSeparate 正常剥离逻辑**）

**回填数据源矩阵（P0-1）**：
| 目标表 | 数据源 |
|---|---|
| normalized_sessions / messages / turns / history_nodes / history_branches / history_cursor | blob `session_data` |
| turn_traces / trace_steps / trace_timeline / tool_activities | `session_turn_traces.trace_data`（逐字段拆解契约：TurnTraceRecord → 主表 + 子表 + extension_json） |

**回填 fail-closed（继承 PA-090）**：表行缺失/JSON 损坏/refs 指向不存在 trace → 整 session 回滚；Authoritative 会话缺 node ref → 硬性门禁（不迁移，报告）。

**回滚能力随阶段递减**：
- 阶段 0-4：三层齐全（migration_state 开关 / 规范化表保留 / backup）
- 阶段 5：层 3 +（双写下层 1/2）
- 阶段 6a：仅切换点 backup + materialize normalized→blob
- 阶段 6b：仅 backup

**阶段 6a 细化（P0-3）**：
```
6a-prepare：暂停/排空写队列，生成切换点 backup，最终一致性校验
6a-freeze：停止 blob 写入，规范化表继续写，进入不可降级观察窗口
6a-rollback：仅允许 materialize normalized→blob 或恢复切换点 backup
6b-delete：确认旧版本不可回滚后，离线删除 blob 字段
```

**migration_state 落库**：`store_metadata` key `storage.normalized.v1.phase`（全局）+ 会话级 marker `{migration_state, normalized_ready, shadow_passed, checksum}`（复用 `storage_dedup.v1:{sessionId}` 模式）。

### 3.6 阶段 2 回填逐字段契约（v6 定稿）

**唯一 authoritative DDL（P0-1/P0-2 闭合）**：复合主键 + 复合 FK + `raw_json` 列 + session-level evidence 字段，**与阶段 1 已提交 DDL 不一致 → 删除重建（normalized 表确认 0 行，方案 C）**。DDL v2 包含：
- messages：`PRIMARY KEY (session_id, message_id)` + `UNIQUE (session_id, ordinal)`
- tool_activities：`PRIMARY KEY (session_id, turn_id, activity_id)`
- history_nodes：`PRIMARY KEY (session_id, node_id)`；history_branches：`PRIMARY KEY (session_id, branch_id)`
- 每张 trace 子表加 `raw_json` 列（完整原始对象，版本化）
- normalized_sessions 加 `history_state_evidence_json`（独立列，不塞 memory_json）

**确定性规则（P0-4/5/6 闭合）**：
- `TurnTraceRecord.session_id = None` → **补当前 session**（与 blob 会话一致）；`Some(x) ≠ 当前` → fail-closed
- 同一 activity 多个 timeline variant → `timeline_variants_json`（**数组**，record-level 为 authoritative 第 0 项，其余按出现序）
- message_id 后缀 `-{n}`：按 `(session_id, turn_id, role)` 分组内 ordinal 递增
- **ordinal 全局分配（P0-6）**：先按"消息 turn（首个 user ordinal）→ trace-only turn（按 trace_order）→ 后续新增"排序后**统一重新编号**（不依赖各自来源序号），保证 `UNIQUE(session_id, ordinal)` 无碰撞

**checksum（P0-7 闭合）**：**版本化 SHA-256**（`sha256:v1:` 前缀）；字段编码 `类型标签 + 长度前缀 + 值`（防 NUL 歧义）；JSON 用 RFC 8785 canonical；REAL 用唯一 round-trip 十进制（`ryu` 格式）；覆盖全部规范化表按 `(session_id, 表名, 主键)` 排序；升级 canonical 规则时保留 spec version，禁止新旧算法直接比较。

### 3.7 阶段 3 materialize 原子协议（v6 定稿）

- **PersistCommand 带 `epoch: u64` 字段**：enqueue 在 admission gate 内校验 epoch；actor 执行前再次校验；**旧 epoch 命令拒绝 + 明确 ACK**（防 materialize 后迟到命令穿透）
- ref 有序校验：顶层 + **每个节点分别**校验有序序列（非 union）；`version` 的 canonical 对照源 = trace 表 `trace_order`（version 缺失时用 updated_at_ms + trace_order 兜底）
- 三处状态同步（blob traceMigrationState + normalized_sessions.trace_migration_state + 会话 marker）同事务
- **提交后内存刷新失败 → 保持 gate 关闭 + 强制重启**（不静默继续）
- marker 值统一：`{migration_state: "dual_write", normalized_ready: false, shadow_passed: false, checksum: ...}`（`dual_write` 是字段值非整个 marker）

### 3.8 阶段 6a/6b 完整状态机（v6 定稿）

**phase 转换表（含启动恢复动作）**：
| phase | 进入条件 | 崩溃重启恢复动作 |
|---|---|---|
| prepare | 开始切换 | backup manifest 存在 + checksum 匹配 → 继续；否则重建 backup 后继续 |
| frozen | prepare 完成 | 继续观察（trigger 已装） |
| observing | frozen + 观察窗口 | 继续观察 |
| retire_pending | 观察通过 | 检查 tombstone 表已建 + phase 已写 → 继续 6b；否则重做 |
| retired | 6b 完成 | 终态（tombstone 门禁常驻） |
| rollback_pending | 回滚决定 | 完成回滚 → rolled_back |
| rolled_back | 回滚完成 | 终态（恢复双写） |

**fencing（P0-8 修正）**：
- frozen 后：数据库 trigger `RAISE(ABORT)` 拒绝写旧 `sessions` 表
- **rollback 前同事务撤销/授权 trigger**（防自锁：rollback 需要写 blob 但 trigger 拒绝）
- **6b 防旧版重建 `sessions`**：**保留名为 `sessions` 的只读 tombstone 表**（或只读 view）阻止旧版 `CREATE TABLE IF NOT EXISTS` 重建——**不能依赖被 DROP 表上的 trigger**（DROP TABLE 自动删除附属 trigger，且 trigger 无法拦截 CREATE TABLE）

**backup identity**：外部 manifest（实例 ID + schema/user_version + 源 checksum + backup checksum + phase + timestamp）；观察窗口后恢复 backup 会丢 frozen 后规范化写（声明 RPO + 人工确认）。

## 4. 影响面与依赖

## 5. 任务拆解（分阶段）

| 阶段 | 内容 | 验收 |
|---|---|---|
| 0-1 | 备份 + 建表（normalized_* 并行表 + user_version + FK） | 表结构就绪，旧数据不受影响 |
| 2 | 回填（数据源矩阵 + 逐字段契约 + fail-closed + 幂等） | 规范化表与数据源一致 |
| 3 | Authoritative→DualWrite materialize + 双写（PersistCommand 统一事务） | 双写期数据一致 |
| 4 | 影子校验（canonical compare + 首个差异定位） | 产物一致 |
| 5 | 切读（写保持双写） | 规范化表权威 |
| 6 | 6a drain/freeze/barrier → 6b 删 blob + 写队列 + 前端分页 | 全量测试 + 实机基准 |

## 6. 风险、回滚与迁移

- P0：表名冲突（方案 A 并行表）；Authoritative materialize 回 blob；6a 切换点
- P1：HistoryNode 快照模型；回填逐字段契约；会话级 migration_state
- P2：旧版应用读新库（schema epoch）；队列尾损（收尾同步 ack）

## 7. 测试计划

1. **回填一致性**：规范化表 vs 数据源逐字段对比（含 HistoryNode snapshot_json）
2. **双写一致性**：双写期随机操作后 blob vs 规范化表一致
3. **影子校验**：新旧 loader 产物对比（消息逐条 + cursor + 拓扑 + 元数据 + canonical hash + 首个差异）
4. **写队列**：背压、失败重试、coalesce、durable ack、崩溃尾损
5. **分页**：游标 token 稳定性、边界、revision 防覆盖
6. **迁移**：dry-run/实跑/崩溃注入/幂等重跑/各阶段回滚验证（含 6a 切换点）
7. **回归**：core 全量 + 前端 vitest + 实机基准

## 8. 验收标准

1. 新 schema 落库（normalized_* 并行表 + 快照模型），旧 blob 双写期可回滚
2. 增量写入生效：turn 热路径不再全量序列化所有会话
3. 前端分页加载：打开大会话不一次传输全量数据
4. 影子校验：新旧 loader 产物一致
5. 迁移后数据可回滚（回滚能力随阶段递减已明确）
6. 全量测试全绿 + Windows 实机基准

## 9. 审核记录

### v1（两路有条件通过）：architect P1（扩展字段/适配层/coalesce/游标/索引）+ consultant P0（回填源/双写语义/回滚窗口）+ P1（schema 缺口/message_id/PersistCommand/migration_state/turns）
### v2（复审不通过）：P0-1 回填协议逐字段契约 / P0-2 Authoritative materialize 回 blob 断点 / P0-3 6a drain/freeze/barrier + schema 字段级核对 + 表名冲突 + 阶段时序矛盾
### v3（2026-08-16，最终复审：阶段 0-1 有条件通过，阶段 2-6 暂不通过）
采纳：3.0 方案 A 并行表、3.1 补 snapshot_json/attachments/trace_migration_state、3.5 materialize 回 blob + 6a 细化 + 执行主体时序表。
**放行边界**：
- 阶段 0（备份）：放行（backup + integrity_check + 进程检测，不改旧读写路径）
- 阶段 1（建表）：有条件放行——建表前须补：FK ON DELETE CASCADE 实际 DDL、nullable/NOT NULL/默认值、state_version/created_at_ms/history_state_evidence 落位、snapshot_json 完整字段契约（含 turn_trace_history 语义）、TurnTraceRecord/TraceTimelineEntry/TurnToolActivity 逐字段映射矩阵、schema version/phase/旧版本读写门禁
- 阶段 2-6：暂不通过——需补：回填逐字段契约（history_state_evidence/created_at/state_version/turns 并集）、materialize 原子协议（同事务 + 写队列 barrier + 缺 ref fail-closed + marker 值统一 dual_write）、6a 持久化状态机（prepare/frozen/observing/rollback_pending/rolled_back + writer fencing + rollback 前冻结规范化写）
**剩余风险**：memory_json 结构契约、snapshot_json 超限策略、tool_activities 去重键（record-level vs timeline-level）、message_id -{n} 计算规则、ordinal 作用域（session 内）
### v4（2026-08-16）：补全阶段 2-6 执行级契约——3.6 回填逐字段矩阵、3.7 materialize 原子协议、3.8 6a 持久化状态机。待复审。
### v5（2026-08-16）：采纳 v4 复审（B+C 混合方案）——复合主键、trace 三源合并、TurnTraceRecord 逐字段补全、子结构无损映射、turns ordinal fallback、checksum 定义、materialize 三处状态同步 + ref 有序校验 + barrier + 独立 marker namespace、6a prepare 崩溃恢复 + rolled_back 完成态 + backup manifest + trigger fencing。待复审。
### v6（2026-08-16，定稿）：采纳 v5 复审——唯一 authoritative DDL（复合主键 + raw_json + history_state_evidence 独立列，与阶段 1 DDL 不一致 → 删除重建方案 C，normalized 表确认 0 行）、session_id=None 补当前 session、timeline_variants_json 数组、ordinal 全局重新编号（防 UNIQUE 碰撞）、checksum 版本化 SHA-256 + 类型标签长度前缀 + RFC 8785 + ryu 十进制、PersistCommand 带 epoch + admission gate 双校验、ref version canonical 源 = trace_order、提交后内存刷新失败强制重启、6a/6b 完整转换表（retire_pending）+ 每 phase 崩溃恢复、rollback 同事务撤销 trigger（防自锁）、**6b 用只读 tombstone 表防旧版重建 sessions（trigger 无法拦截 CREATE TABLE）**。阶段 1 DDL 已同步升级（复合主键/raw_json/evidence 列）。