# ADR 0099：会话存储改为「事件溯真源 + 单写者 + 墓碑删除」四阶段重构

Status: accepted

## 背景：当前存储模型已是现网缺陷源

真实库 `~/.local/share/PonyAgent/sessions.db`（392 MB + WAL 170 MB）只读取证：

| 事实 | 数字 | 含义 |
|---|---|---|
| `sessions` blob 行 | 71 行 / 5.35 MB | legacy 全量快照，唯一被读的真源 |
| `normalized_sessions` | 121 行 | 规范化表在双写 |
| **有规范化行但无 blob 行的会话** | **50 个（41%）** | **重启后不可见的孤儿会话，且持有真实事件** |
| `turn_events` | 48,002 行 / 43.7 MB | 完整 append-only 事件流，**只写不读** |
| `storage.normalized.v1.phase` | **键不存在** | 读路径恒走 legacy blob，规范化表零收益 |
| `storage.normalized.v1.epoch` | **键不存在** | 所谓乐观锁永不生效 |
| `store_metadata` 中 `turn_event_seq:*` | **~140 个键** | 每会话计数器塞进全局 KV，随整包写盘一起被覆盖 |
| flock / 单实例 | **0 处** | 桌面端与 TUI 可同时写同一 db（现场已发生） |

由此在近期暴露并已取证的四类缺陷全部源自同一个结构性原因——**写模型是「全库状态快照 + LWW 覆盖」**：

1. 每次 turn 构造整库 `PersistedStore` 整包 `INSERT OR REPLACE`（`session/store.rs:2430`、`:2464`），约 **5.1 MB × ≥20 次/turn ≈ 102 MB/turn**；事件流实测 135.6 事件/turn ≈ 129 KB/turn ⇒ **写放大 ≈790×**。
2. 整包覆盖导致跨进程 LWW：旧快照覆盖别端新 turn；已删会话被旧快照重新 `INSERT` 复活。
3. 移除 DELETE 后删除不再跨进程传播（ADR 0090 工作区改动的对称代价）。
4. `pony resume` 从规范化表恢复时 trace 取自 **legacy** `session_turn_traces`（`sqlite_session.rs:1960`），而 50 个孤儿中有 29 个只有 normalized trace ⇒ 恢复即半残，且自愈回写把残缺固化为「权威」。

## 目标

| # | 目标 | 可验收判据 |
|---|---|---|
| G1 | 写模型从「状态覆盖」改为「事件追加」 | 单 turn 写放大 ≤3×；追加 p95 ≤30 ms |
| G2 | 多端一致：TUI / 桌面端 / CLI / 未来远程 | 同一会话任意时刻单写者；冲突只剩「接管」一种可判决形态 |
| G3 | 删除单调不可复活 | 任一端删除后所有端永久生效（墓碑事件） |
| G4 | 可传输 | 事件流本身即复制单元，支持 `after_seq` 增量拉取与幂等重放 |
| G5 | 可迁移 | 会话可导出为可读目录包，供换机/分享/审计 |
| G6 | 不丢数据 | 50 个孤儿会话归零；存量 100% 可重建 |

## 非目标（本 ADR 明确不做）

- **N1 CRDT / 自动语义合并**：两条 assistant 回复没有 merge 语义，自动合并只会把错误藏得更深。正确解法是所有权与分支，不是合并。
- **N2 多主写入**：任一端皆可权威写 = 把不可判定的冲突塞进存储层。
- **N3 实时协同编辑**：单用户线性对话，收益近零。
- **N4 云端账号 / 端到端加密**：远程只需「单写者 + 可鉴权 API」，账号与加密是独立议题。
- **N5 存量历史重编码**：只保证旧库可读、可导出、可回落。
- **N6 附件内容寻址改造**：保持现有相对路径，只保证引用可解析与降级可见。

## 决策

### D1 真源：每会话一份 append-only 日志文件；SQLite 降级为可重建索引（2026-11-08 修订）

> **修订理由（原 D1 判「SQLite 追加表为真源」已推翻）**：原判断基于「单机单用户」前提，用「跨会话事务 / Windows 锁语义 / torn-tail 解析器成本」三条否掉了文件真源。但产品目标形态是**多端 + 远程分派 + 混版本共存 + 多 agent 并行**，这三条理由的权重整体反转——尤其「整库单值 `events.schema_version`，旧版本遇新版直接拒开整库」在混版本部署下是硬伤。
>
> 根本问题不是介质，而是**写单元粒度**：只要真源是「一个库装所有会话」，就必然存在「全库只有一支笔，笔归谁」的争用，于是被迫引入 daemon / 租约 / CAS 一整套机制，而阶段 0 两轮对抗审查证明这套机制极易漏。把写单元从「全库」降到「单会话」后，该争用**结构性消失**。

**真源 = 每个会话一份 append-only 事件日志文件**：

```
<root>/--<normalized-cwd>--/<encoded-session-id>/
  session.jsonl[.zst]        # 当前代际；append-only，逐批 fsync
  session.vN.jsonl[.zst]     # 格式升级后的不可变后继代际（旧文件逐字节不变）
  manifest.json              # 会话自描述：格式版本 / 来源端 / 分支头 / 附件清单
  attachments/               # 实体文件
```

- 文件头部自描述格式版本（`{"type":"session","version":N,"id":...}`）；读方按版本分类为 `current | migration-required | unsupported | malformed`，**不再是布尔化拒绝**。
- 追加语义：每批 append 后 `fsync`；崩溃只丢「撕裂尾行」，已提交前缀必保留；**永不重写已提交事件**。
- 事件记录携带 `(seq, type, payload, ts)`；`seq` 由**该文件的唯一写者**单调分配（文件级独占即单写者，无需全库协调）。

**SQLite 的位置改为「可重建索引 / 查询缓存」**：

- 索引内容：会话列表、标题、workspace、路径授权、用量账本、全文检索等**派生视图**。
- 索引可随时删除并从文件全量重建；索引损坏 ≠ 数据丢失。
- 这解决原 D1 反对文件真源的唯一硬理由（「跨会话查询需要 SQL」）：查询照旧走 SQL，只是它不再承担「唯一真相」的责任。

### D2 写者模型：写权随会话归属，不做全库租约（2026-11-08 修订）

- **一个会话的写者 = 创建它的那个 agent 进程**，且只有它写该会话文件（文件级独占锁 `flock`/`LockFileEx` 作为实现手段，而非架构机制）。
- **跨会话零争用**：不同会话是不同文件，多 agent 并行天然成立。
- **跨节点**：会话跟着 agent 走。远端 Linux 上的 agent 写远端文件；桌面端作为客户端**按需拉取**该会话日志（增量、从 `after_seq` 起），不做整库同步、不共享文件系统。
- **不需要 daemon 作为「唯一写者」**：daemon 从「架构必需品」降级为「可选的远程访问端点」（阶段 3 视需要再加）。
- **不需要 CAS / fence_token**：同一会话的并发写在结构上不存在。
- **混版本部署**：只影响单个会话文件的格式兼容，不波及全库。

### D3 删除语义：墓碑事件，不删行

`session_deleted` / `turn_deleted` / `message_redacted`，fold 遇墓碑视为不存在。物理清理只走既有的 `turn_events_archive` + `events.retention.days`（默认 90 天）。这是跨端删除传播唯一正确的实现方式。

### D4 投影：扶正既有 `normalized_*`，不新建体系

- 每投影行带 `(fold_epoch, folded_seq)`；`normalized_sessions.state_version` 语义改为 `folded_seq`。
- 增量 fold：只读 `seq > folded_seq`，单次 ≤512 行，超阈值先落 snapshot 锚点（复用 `normalized_history_nodes.snapshot_json`）。
- fold 必须是纯函数 `fold(events, from_seq) -> Projection`，可重放、可 off-thread。
- `fold_epoch` 与 `store_metadata['events.fold.epoch']` 不一致 / schema bump / 会话 degraded ⇒ 后台重建，重建期回落 blob 兜底。

### D5 传输：事件流即同步协议，目录包即迁移形态

- 远端：`GET /v1/sessions/{id}/events?after_seq=&limit=500` → `{events[], next_seq, head_seq, fold_epoch}`；上传带 `(writer_id, writer_seq)`，服务端 `INSERT OR IGNORE` 幂等。
- 断点续传以服务端 **`committed_seq`** 为准（不是 `received_seq`，否则崩溃后跳洞）。
- 不同 writer 的 seq 冲突**不合并**：按 `branch_id` 保留分叉，UI 呈现分叉；仅单值元数据用 LWW。
- 导出：会话目录包 = `manifest.json`（格式版本 / 来源端 / writer seq / 分支头 / 附件清单）+ `events.jsonl` + `attachments/`，对外封 `.tar.zst`，可只读打开、缺附件降级为占位、默认不并入用户级记忆与全局配置。

### D6 保持独立实体、不进事件流

`AttachmentAssetMap` / `session_attachment_index`（108 处引用）、`workspaces`、`path_authorizations.v1`、`usage_ledger`、`pending_asks`。凡不能 fold 出来的，保持独立表并在同事务级联。

### D7 格式契约治理：只借了 DSH 的常量，没借它的契约体系

`turn_event.rs:14` 的注释自述「借鉴 dsh SESSION_FORMAT_VERSION」，但**只借了单个常量，没有借配套的契约机制**。现状对比如下：

| 契约维度 | DeepSeek Harness | 本项目现状 | 差距 |
|---|---|---|---|
| 版本号 | 单一手写真源 `SESSION_FORMAT_VERSION` | `EVENT_SCHEMA_VERSION: u64 = 1`（`turn_event.rs:14`） | 等价 |
| 版本状态 | **三元分离**：writers / finalized 兼容基线 / released + evidence tag，三者独立且机械校验 | 只有一个数 | 缺状态语义 |
| 版本演进 | v0→v1→v2→v3→v4 **相邻迁移链**，构建期校验无缺口、顺序唯一、已接受段不可复用 | **无迁移链**，只认当前值 | 缺演进骨架 |
| 版本不匹配 | `readHeader` 返回 `current / migration-required / unsupported / malformed` **四态** | 严格 `version == current`，否则 `Err`（`sqlite_session.rs:2496`）⇒ **旧二进制见新版本直接拒绝打开整个库** | 回滚即不可读 |
| 损坏恢复 | `recovery: strict \| recoverable` 双模式；可恢复解码器丢弃未提交后缀，**且后续成功的 `turn/end` 会把原问题升级为致命** | `IGNORABLE_EVENT_TYPES = []` 全 fail loud，仅整会话 degraded | 缺分级与防静默截断 |
| 同版本新增 | 受「读取器兼容性评审」约束 + 快照目录校验 | 注释约定「新增事件类型不 bump」，**无机械门禁** | 口头契约 |
| 格式单一真源 | `session-format` + `session-format-catalog` 模块 + 生成器 | 散落三处：事件类型 `turn_event.rs`、DDL `sqlite_session.rs:254`、投影映射 `normalized_*` | 缺单一真源 |
| payload 值域 | `SessionFormatJsonValue` 无损 JSON 检查 | 无写入前校验 | 缺约束 |
| 文档一致性 | 双语 + `doc-standard.spec.ts` 机械校验记录 | 无 | 缺门禁 |

**本项目领先的一点必须保留**：`CorruptedEventTombstone`（PA-096）把单条坏事件固化为墓碑事件、放行主干投影，粒度比 DSH 的「丢弃未提交后缀」更细，可审计。这条不是补丁，应上升为契约。

**决策（并入阶段 2）**：

1. **版本三元分离**：`SESSION_EVENT_WRITER_VERSION`（唯一手写常量）/ finalized 兼容基线（冻结）/ released 版本 + evidence tag，三者独立记录并机械互校。
2. **相邻迁移链**：目目录化 `event-format-vN-to-vN+1`，构建期校验无缺口与顺序唯一，已接受段禁止复用。
3. **四态分类**：读 `store_metadata` 即返回 `current | migration-required | unsupported | malformed`，取代布尔化 `Err`，使「旧二进制打开新库」可降级为只读而**不是拒绝整个库**。
4. **recovery 双模式**：`strict`（CI/测试，任何异常 fail loud）vs `recoverable`（用户态，返回已接受前缀 + 墓碑），并强制 DSH 的**防静默截断规则**：恢复后缀后若出现成功的 `turn/end`，原损坏升级为致命错误。
5. **同版本新增机械门禁**：事件类型清单 + 版本化 fixture + 快照测试；新增类型未入目录或无兼容性评审记录 ⇒ CI 红。
6. **单一真源模块**：新增 `session-format` 模块收敛「事件类型清单 + DDL + 投影映射 + 版本常量」，由生成器产出，禁止继续在 6274 行文件中散写。
7. **无损 JSON 契约**：payload 写入前拒绝非有限数等不可无损往返的值。

### D8 从 DSH / claude-code 取什么、不取什么

| 来源 | 取 | 不取 | 理由 |
|---|---|---|---|
| DeepSeek Harness | append-only 事件溯源、崩溃只丢撕裂尾、代际不可变、可审计的逐行产物 | 每会话 JSONL 文件作**真源** | 跨会话单事务（workflow/team 一次提交同时改多会话 + 记账）在文件模型下要自写 2PC；Windows 无 flock、rename+fsync 语义不可靠；torn-tail 解析器是净新增状态机 |
| claude-code | 会话目录包的人类可读 / 可 diff / 可分享 | 弱持久化、无事务、无多端所有权 | 用目录包承载**导出与传输**，不用它承载**并发真源** |

## 四阶段实施（每阶段独立可回滚）

### 阶段 0 · 止血与收口（1–2 周，不改真源）

1. 23 个全库落盘调用点（18 处 `save_to_backend()` + 5 处 `save_session_state_to_backend(…)`）收敛为单一 `commit_boundary()`（单次 `BEGIN IMMEDIATE` 全库事务）。
2. ~~`sessions` 写加 session 级 `version` + CAS 守门~~ → **移交阶段 1**（见下「修订」）。
3. 引入文件锁 / 单实例，终结桌面端与 TUI 同写。
4. 处置 50 个孤儿：回填源**只能是 `turn_events ∪ normalized_*`**，绝不能以 `sessions` 表为源；无法回填的显式标记 `retired` 并在 UI 明示。
5. `busy` 重试改按 `ErrorCode::DatabaseBusy/DatabaseLocked` 判定（现 `contains("busy")` 对 `database is locked` 永不匹配）。

> **修订（2026-11-08，Lead 裁决，用户批准）：第 2 项（版本 CAS）整体移交阶段 1。**
>
> 两轮三路对抗审查（`.dev-team/review-stage0-l3{a,b,p}{,-r2}.json`）证明：CAS 在当前写模型里没有结构性前提——**单行写 API（`upsert_session(&SessionState)`）不携带写者身份与期望版本**，因此每条落盘路径都要单独补守门，而审查每轮都能找到新的未守门路径：
> - R1 六条 BLOCKING：DEFERRED 事务致 `BUSY_SNAPSHOT` 静默丢弃、`advance_versions` 恒定差一、`StaleVersion` 重试退化为 LWW、`INSERT OR REPLACE` 重置 version 的 ABA、`let _ =` 吞错致无 CAS 仍报 `Committed`、降级实例被 ask store 击穿。
> - R2 又四条：热路径单行写无 `WHERE version` 静默覆盖并发 turn、单行写 `INSERT OR REPLACE` 复活墓碑并把 version 归零、`StaleVersion` 前已写 trace、锁恢复后陈旧快照写。
>
> 结论：**CAS 与写者身份必须同批交付**——这正是阶段 1 `session_writers.fence_token` 提供的东西。阶段 0 只保留与 CAS 无关、且已被独立验证坚实的能力：单次 IMMEDIATE 整包提交（原子性）、单实例 flock、busy 判定与总预算 ≤3000ms、trace 源切换、孤儿回填与 `ponycli sessions backfill`、整包提交路径上的墓碑跳过（`versions` 侧表仅用于「判定会话已删除」，不做拒绝语义）。
>
> 已知遗留（阶段 1 前维持现状，**不新增回归**）：单行写路径仍为「最后写入者胜」；已删会话仍可能被旧快照复活。与 ADR 现状描述一致，不因本次重构恶化。

**门禁**：孤儿归零（双向集合差 = 0）；`npm run size:check` exit 0（见下「更正」）；逐会话回填校验和一致。

> **更正（2026-11-08，Lead 复核）**：参谋评审曾报「`size:check` exit=1、70 条全部『基线无此条目』⇒ 棘轮失效、重构无护栏」。复核结论为**误判**：该次调用了 `node scripts/check-file-size.mjs` 却漏传 `--baseline scripts/file-size-baseline.json`。按 `package.json` 的正确调用实测 `exit=0`，`0 new / 70 over limit / 70 grandfathered / 0 shrunken`；`session/store.rs` 5704 行 = 基线 5704（正好卡线未恶化）。**棘轮健康，不构成阶段 0 的前置阻塞。**

**阶段 0 实施状态（2026-11-08，已交付，commit `a5c269c7`）**

| 项 | 状态 | 证据 |
| :--- | :--- | :--- |
| 全库入口收敛为 `commit_boundary`（单次 `BEGIN IMMEDIATE` 整包事务） | ✅ 完成 | 23 个入口全部经边界；`storage_stage0_write_amplification` 2/2 绿 |
| 单实例 flock + 降级只读 + 恢复 | ✅ 完成 | `storage_stage0_*` 相关用例；降级实例零写入（行级 sha256 不变） |
| busy 判定 `ErrorCode::DatabaseBusy\|DatabaseLocked` | ✅ 完成 | `busy_classification` 4/4、`busy_locked_retry` 1/1 绿；`contains("busy")` 全仓 0 处 |
| trace 源切 `normalized_turn_traces` | ✅ 完成 | `recover_trace` 1/1 绿 |
| 孤儿回填 + CLI 入口 | ✅ 能力完成 | 副本实测 80→128 行、orphan 48→0、`checksum_mismatch=0`；`--dry-run` sha256 前后一致 |
| 整包提交路径墓碑跳过 | ✅ 完成 | `tombstone_no_resurrect` 2/2 绿 |
| 版本 CAS | ⏭ 移交阶段 1 | 见上文修订段 |
| **真实库孤儿归零** | ⏳ 待执行 | 真实库正被运行中实例持 flock；操作步骤见 `task-3` |

门禁实测（Lead 独立复跑）：6 套件 11 用例全绿；`cargo check` exit 0（0 warning）；`npm run size:check` exit 0（0 new / 70 grandfathered）；`cargo test -p pony-agent-core --lib` 1367 passed / 0 failed。

> 运维注意：备份会话库必须**三件套一起复制**（`sessions.db` + `-wal` + `-shm`）。只复制 `.db` 会得到不一致快照，回填会报 `database disk image is malformed`（已实测）。

### 阶段 1 · 会话日志文件真源（写入侧，2026-11-08 重新定义）

取代原「单写者租约 + 墓碑」设计（该设计要解决的问题已被 D1 修订消解）。

1. **端口**：新增 `SessionLogStore` trait —— `open_write(session_id, meta) -> LogWriter` / `open_read(session_id, after_seq) -> LogReader` / `list_sessions()` / `delete(session_id)`。本地实现写文件，远程实现走 HTTP（阶段 3），同一契约。
2. **目录布局**：`<root>/--<normalized-cwd>--/<encoded-session-id>/`，会话 id 单射转义为安全路径段；规范化 cwd 让项目目录可读、可 rsync/git 审计。
3. **写入**：`append(batch)` 追加 + `fsync`；失败回滚到先前字节长度；**已提交事件永不重写**。`seq` 由该文件唯一写者单调分配。
4. **崩溃恢复**：重启时丢弃不完整的最终行/撕裂的压缩尾帧，保留已提交前缀；读方补写合成 closer。
5. **格式自描述 + 四态分类**：header 携带 `{type, version, id, cwd, createdAt}`；`read_header` 返回 `current | migration-required | unsupported | malformed`，取代现在的 `version == current` 布尔拒绝。
6. **写者归属**：文件级独占锁（Unix `flock` / Windows `LockFileEx`）；抢不到 ⇒ 明确降级只读 + 日志，不静默写。
7. **Windows 硬成本专项**：原子发布用「临时文件 + `create_new` + rename」，`fsync` 后 rename 保证不产生半写文件；需在 Windows 上实测 rename 语义与杀毒软件干扰。
8. **压缩**：默认 `zstd` 帧 + 校验和；可配置 `none` 便于外部工具直接消费。

**门禁**：kill -9 演练后重启，已提交前缀 100% 可读且无重复；并发 8 个 agent 各写自己会话零冲突；单会话追加 p95 ≤10ms；撕裂尾行注入 100 例全部正确丢弃。

### 阶段 2 · 读路径切换 + SQLite 降为索引（2026-11-08 重新定义）

1. **fold**：从会话日志重建 `SessionState`，纯函数 `fold(events) -> SessionState`，可 off-thread。
2. **索引重建器**：`rebuild_index()` 扫全部会话文件，重建 `sessions`/`normalized_*`/`workspace`/`usage_ledger` 等派生视图；索引可删可建，删除不丢数据。
3. **读切换灰度**：先双读比对（文件 fold vs SQLite 现有路径逐字段 diff），diff=0 连续 7 天后切读；SQLite 仍写但降级为缓存。
4. ✅ **停写 blob（已完成，commit `e1249b5c`）**：有日志会话的 `sessions.session_data` 改为固定长度轻量索引行（763B，1 轮与 3 轮字节相等），仅保留索引字段 + 哨兵 `__index_only`；无日志存量会话继续走老路径（grandfather）。迁移策略保持 `NOT NULL`（不放宽为可空，避免 `row.get::<String>` 撞 `InvalidColumnType`）。2-B 的 `updated_at_ms` 过渡定序已删除——有日志会话**日志无条件为准**。同批闭合了因停写 blob 才暴露的写路径缺口（ReplaceAll/rename/archive 改走命令流、索引视图从 `normalized_*` 补全、observation 载荷回填、顶层 trace 改由日志 DAG 推导）。详见 [.agents/notes/implemented/architecture/2026-11-08-stop-writing-whole-store-blob.md](../../.agents/notes/implemented/architecture/2026-11-08-stop-writing-whole-store-blob.md)。
5. **孤儿处置**：不再需要「孤儿回填」概念——文件即真相；SQLite 侧缺行由索引重建自动补齐。

**门禁**：`fold-verify --all --fail-on-diff` 连续 7 天 diff=0；索引删除后重建，会话列表与逐会话内容 100% 一致；启动装载 ≤800ms。

### 阶段 3 · 远程与传输

实现 D5。本地优先；切换到「远程为真源」需四项同时成立：① 多设备同写占比 >5%；② ≥2 个必须服务端做的能力；③ 冲突率 <0.1% 且 RTT p95 <150 ms；④ 离线不再是产品硬需求。

## 验收阈值（阶段 2 起生效）

| 指标 | 阈值 |
|---|---|
| 单 turn 追加 | p95 ≤30 ms，p99 ≤80 ms |
| 写放大 | ≤3× |
| 每 turn 追加行数 | ≤200（现 135.6） |
| 单会话事件行 | ≤5,000，超限强制 compaction 成 snapshot |
| 增量 fold / 冷 fold | ≤5 ms / ≤50 ms |
| 启动装载 | ≤800 ms |
| WAL 稳态 | ≤64 MB 且每小时至少 1 次成功 checkpoint（现 170 MB = 42×） |

## 风险与护栏

1. **顺序不可颠倒**：现在直接切真源，会把 50 个孤儿会话从「重启不可见」升级为「不可逆丢失」。阶段 0 必须先行。
2. **回滚即不可读**：`events.schema_version` 是单值且 fail loud，旧二进制见 v2 会拒绝整个库 ⇒ 回滚窗口必须锁定「N-1 只读 v1」或双写 v1 视图。
3. **迁移前先在 `cp` 副本上演练 3 轮**，逐会话校验和不一致进 quarantine，不删除。
4. **`phase` 键必须与数据同事务写入**——现孤儿会话正是「写数据与写 phase 不同事务」崩在中间的产物。
5. 改动面：`session/store.rs`(5706L，19 调用点/76 pub 方法)、`sqlite_session.rs`(6274L)、`session/backend.rs`(389L)、`session/types.rs`(841L)、`file_backend.rs`(259L，漏改即双后端分叉)。Tauri 薄（`tauri_adapter.rs` 仅 10 处）、前端为事件推送（0 处 session invoke）。

## Alternatives considered

1. **保持整包快照，只加锁与 CAS**：改动最小（阶段 0 即此），但写放大 790×、跨会话事务一条 `BEGIN IMMEDIATE` 拖住全部、无法支撑远程增量传输。否决为终局，保留为阶段 0。
2. **每会话 JSONL 文件为真源（DSH 直搬）**：崩溃韧性与审计性最好，但跨会话原子性、Windows 锁语义、torn-tail 解析器三项成本高于收益；且本项目已有成熟的 `TurnEvent` 事件模型，无需重造。否决为真源，保留为导出格式。
3. **状态真源 + changelog（CDC 式）**：读写路径改动小，但 changelog 与状态双真源会重演本轮 `normalized_*` 双写无人读的分叉。否决。
4. **每条写入各自加锁 + 时间戳 LWW（不动模型）**：只能把 5 MB × 20 次变成 5 MB × N 次，不解决覆盖与复活。否决。
5. **远程为真源、本地为缓存**：当前无远程服务、离线是硬需求。延后至阶段 3 转折条件成立。

## 附：为什么是 SQLite 而不是 JSONL（历史事实，结论已被 D1 修订推翻）

> **注意（2026-11-08）**：本节记录的是修订前的判断依据，仅作历史留存。当前决策见 D1——真源改为每会话一份 append-only 日志文件，SQLite 降级为可重建索引。本节「介质不是根因」的判断仍然成立，但「因此继续用单库 SQLite」的结论已不适用：单库才是争用根源。

**本项目从未真正「选择」过 SQLite 而不是 JSONL。** ADR 0008 的三个候选方案（A 状态更新 / B 全量事件溯源 / C 混合演进）讨论的全是**数据模型**，存储介质一次都没有进入决策；`docs/analysis/deepseek-harness-architecture-comparison-2026-08-14.md` 同样只提「快照式存储」，未比较介质。SQLite 是继承来的既成事实，事件溯源改造是在它的**内部**做的。

介质对两边的适配性其实不同：

| 需求 | pony-agent | DSH |
|---|---|---|
| 产品形态 | 桌面（Tauri）+ TUI + CLI，多客户端并发 | CLI 单进程 |
| 跨会话结构化查询 | 必需（会话列表、搜索、workspace、路径授权、用量账本、`pending_asks`） | 仅靠扫目录 + `stat` |
| 跨实体原子事务 | 必需（session 写入需与 ledger / ask 原子） | 无此需求（每会话独立文件） |
| 部署与备份 | 「一个文件 = 全部数据」 | 无此诉求，反而要逐会话可读产物 |
| Windows | 必须支持；文件 rename+fsync 语义与锁不可靠 | Unix 优先 |
| 结论 | **SQLite 更合适** | **JSONL 更合适** |

**真正的教训不是介质选错，而是写模型与介质错配**：在关系库里硬套「整包状态覆盖」，才会同时得到 790× 写放大、跨进程 LWW、WAL 失控——这些在 JSONL 上同样会烂（整文件重写即可复现）。反之 SQLite 上跑 append-only 追加表，就是 JSONL 的等价物。

因此本 ADR 只抄 DSH 的**结构性优势**——「每会话一个物理单元 ⇒ 天然单写者边界与局部性」，用 `session_writers` 租约 + 追加表在 SQLite 内等价实现，而不搬它的介质。

## 介质边界：SQLite 在哪里会让路（修订后：SQLite 已退出真源位置）

> 本节讨论的是「单库 SQLite 作真源」时的容量边界，属修订前语境。D1 修订后 SQLite 只做索引，容量与争用问题随之降级为「索引重建耗时」问题。

结论：**对「本地 + 远程 + 多会话并行」这条路线，SQLite 不是阻碍——前提是介质被藏在端口后面，且三条护栏成立。**

不会成为阻碍的量化依据：单 turn 事件批 ≈135 事件 / 129 KB，单事务提交 p95 ≤30 ms ⇒ 理论 ≥33 turn/s。而并行 subagent 的写入节奏由 LLM 延迟支配（单 turn 数秒到数十秒），8 路并行的实际写入需求 **<1 tx/s**。瓶颈是模型延迟，不是本地 IO。读侧 WAL 允许 N 个并发读者，不受写阻塞（除 checkpoint 瞬间）。

会变成阻碍的两个真实场景（必须立法排除）：

1. **把 db 放网络盘 / 云同步目录（SMB / NFS / iCloud / Dropbox）**。SQLite 官方明确不支持。若未来「远程」被误解成「共享同一个 db 文件」，这是硬阻碍且会直接损坏数据。⇒ **远程同步只走事件协议，永不共享文件**（D5）。
2. **并发写者数 × 写事务时长超出单写者容量**。SQLite 每库仅一个写者；`subagent_runtime.rs:6` 的「独立建栈」分支已为每个 subagent 各建一个 `SessionStore` 写同一 db。⇒ 写事务必须 ≤50 ms、禁止在写事务内做 fold 或全量读；每会话写请求入队；若将来并行度上到数十路，预案是按 workspace 分片（代价：失去跨会话聚合，故列为最后手段）。

**介质可替换的关键设计**：定义端口 `SessionEventStore { append(batch) / read_after(session_id, seq, limit) / list_heads() / tombstone(session_id) }`。本地实现为 SQLite 追加表，远程实现为 HTTPS 客户端，二者同契约。这正是 DSH 真正的架构优势——**持久化是一个带明确后端契约的 seam**（`ctx.sessionPersistence` 服务，规定单写者、append-only、flush 屏障、撕裂尾永不可见），而不在于它选了 JSONL。本项目应抄这个 seam，而不是抄介质。

## 并行 subagent 的写者拓扑（已裁决）

**裁决：共用宿主 `SessionStore`（现有 `shared_sessions` 路径）。不各建栈，也不做会话级分片锁。**

事实基础（已核验）：

- 现状默认已是共用：`builder.rs:127` 把宿主 `sessions_arc` 注册进全局 `OnceLock`（首写者胜）；`subagent_runtime.rs:6` 注释「与宿主完全隔离」与代码实际行为**已失真，需修**。
- 各建栈分支 = 每个 subagent 各开一条连接写同一 db ⇒ 真实 `SQLITE_BUSY` 争用，且各持一份全库快照互相覆盖，是已取证的 LWW／会话复活类缺陷的**结构性放大器**。排除。
- **全局 `RwLock` 不跨 `.await` 持有**（已核查）：`control_plane/mod.rs` 0 个 `async fn` / 0 个 `.await`，9 个触碰 `sessions_rwlock` 的文件中 8 个全同步；唯一异步的 `upload_session_trace`（`query_commands.rs:60`）先用 `load_session_traces` 取走所有权数据（guard 在表达式结束即释放），再 `spawn_blocking(...).await`。因此 A 的代价是**线程阻塞**（时长 = 同步 SQLite 写），不是 async 死锁，更不涉及 LLM 延迟。

**为何不做会话级分片锁（原「方案 C」）**：SQLite 每库仅一个写者，分片锁只能并行 CPU 侧（序列化 / fold），**无法并行提交**；而 append-only 之后单 turn 仅 129 KB 序列化（亚毫秒级），并行收益趋零。收益与写模型改造呈**负相关**。成本侧：`SessionStore` 含 10 个跨会话共享字段（attachment 索引、workspace 注册表、path 授权、projection cache、backend、ask store…），51 处锁点、76 个 pub 方法，分片 = 重写可变性模型 + 引入锁序风险，与「写模型改造 + 真源切换」正交叠加。

**结论**：阶段 0 不动锁结构（只收敛提交边界、缩短事务）；阶段 1 单写者落地后，全局锁与单写者语义天然同构，分片收益归零。仅当出现「数十并行 subagent + 大事件量」的**真实压测证据**时才重议，且届时优先考虑按 workspace 分库，而非分片锁。

## 待决

- 终局单写者是「独立 daemon 进程」还是「桌面端即宿主、TUI 为客户端」——影响 TUI 秒开预算与 Windows 服务化成本，需阶段 1 前定稿。
- `usage_ledger` 与事件流的双记账一致性校验方式。
