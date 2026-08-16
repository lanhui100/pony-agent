# PA-090 Spec: 存量会话存储迁移（三源合并 + refs 保护 prune + 回滚）

> 修订版 v3（2026-08-15）：采纳 consultant v2 复审全部意见——ReplaceAll 全调用点 union（P1-5 真正闭合）、
> collect_trace_union 稳定顺序、顶层 trace 显式清空、refs 已写精确定义、--force 决策矩阵、
> 应用退出前置条件、prune 调用点、真实路径级回归测试。

## 1. 背景与目标

**背景**：PA-088 已完成新写路径，存量会话（LegacyBlob/DualWrite）保持原样：最大会话 43.77MB（historyNodes 35.54MB 冗余），打开时前端 JSON.parse 卡死。

**目标**：
1. 存量会话迁移为 TraceTableAuthoritative（剥离顶层+节点 trace + 生成 refs + 写表）
2. 三源合并（已有表 ∪ 顶层 blob ∪ 节点 trace），不丢任何 trace 最新版本
3. refs 保护 prune（只删无节点引用的 trace）
4. 迁移后 blob < 5MB，重启后 checkout/fork/branch 语义不变
5. 可回滚（备份 + materialize 回填）

## 2. 范围与非目标

**范围内**：
- `scripts/compact-sessions-db.mjs`：完整迁移脚本（重写，非旧脚本追加）
- `crates/pony-agent-core/src/agent/sqlite_session.rs`：refs 保护 prune + ReplaceAll union 最终防线
- `crates/pony-agent-core/src/agent/session.rs`：ReplaceAll 全调用点 union + collect_trace_union 稳定顺序
- 迁移后验证

**非目标**（归 PA-089）：完整拆表 / 增量写入 / 写队列 / 前端分页 / 多版本精确历史。

## 3. 技术方案

### 3.0 前置：ReplaceAll 全调用点 union（P1-5 真正闭合）

**问题**：Authoritative 会话的 ReplaceAll 若用顶层 trace（24 行截断）替换表，节点 refs 指向的 trace 被清空 → 重启 materialize 全部 missing。

**ReplaceAll 调用点全量清单**（必须全部统一为 union）：
1. `persist_session_and_trace_change` 主事务路径（session.rs:2359-2373）：**识别 ReplaceAll，Authoritative 会话改用 `collect_trace_union(session)` 再传后端**
2. `save_session_to_backend` 正常分支（session.rs:2333-2348）：用 union
3. `save_session_to_backend` NotFound 重试分支（session.rs:2399-2414）：用 union
4. fallback `persist_trace_change(ReplaceAll)`（session.rs:2431-2441）：已用 union ✓
5. SQLite `persist_session_with_trace_mutation`（sqlite_session.rs:1021-1029）：**最终防线**——Authoritative 会话不无条件信任传入的 traces，内部再校验/合并（或由调用方保证 union）

**collect_trace_union 稳定顺序（P0-3 闭合）**：
- 顶层 trace 数组原位顺序优先（merge 后原位替换）
- 节点独有 trace 按节点顺序追加（节点按 history_nodes 数组顺序）
- 同 turn_id 多版本：取 updated_at 最新；同时间取顶层 > 节点（source order）
- **禁止 HashMap::into_values 输出**（顺序不稳定）

**回归测试（真实路径级）**：checkout → 主事务持久化 → 重启 load_store → 节点 materialize 完整（不只测 helper）。

### 3.1 迁移脚本（per-session 事务 + 幂等）

**前置条件（不可绕过）**：
1. **应用必须退出**：脚本开头检测 Pony Agent 进程（Windows：`Get-Process pony-agent` 或等价）；wal_checkpoint busy 仅作辅助提示，**不作为唯一依据**（空闲时可能成功）
2. 备份：`VACUUM INTO '<backup>'`（一致快照）+ `PRAGMA integrity_check`；**dry-run 不建备份、不写库、不写标记、不执行 TRUNCATE checkpoint（不改变 WAL 状态）**

**每个 session（独立事务 BEGIN IMMEDIATE + try/catch/finally ROLLBACK + busy_timeout=5000）**：

1. **读取三源**（事务快照内）：已有表 ∪ 顶层 blob `turnTraceHistory` ∪ 节点 blob `historyNodes[*].turnTraceHistory`
2. **合并（TraceCandidate）**：
   - 时间戳以 trace_data JSON 内 `updatedAt` 为准（SQL 列不一致时告警，最终写表后校验一致）
   - 按 `(turn_id, updated_at_ms)` 去重取最新；时间相同：表 > 顶层 > 节点
   - **trace_order 保序**：顶层数组原位优先 → 节点独有按节点顺序追加 → 表独有追加末尾
3. **fail closed**：任一源解析失败 / turn_id 为空 / sessionId 不匹配 / updatedAt 缺失非法 → **整 session 回滚**（禁止 filter/continue 跳过）
4. **写表**：先 `DELETE FROM session_turn_traces WHERE session_id=?` 再逐行 INSERT（replace 语义，--force 无残留；失败时 DELETE 也回滚）
5. **生成 refs + 回填 turn_id**：
   - 节点：`turn_trace_refs = Some(refs)`；`turn_id` fallback（节点 trace 末条 → run_id → history 末条非空 turn_id），**候选必须经 canonical set 校验**，失败保留 None（不生成 dangling ref）
   - 顶层：`turn_trace_refs = Some(refs)`（**空集合也序列化为 `[]`**）
   - refs 的 `updated_at_ms` = 最终选中 canonical trace 的时间
6. **剥离**：清空顶层 `turnTraceHistory` + 全部节点 `turnTraceHistory`（**两者都清**，否则 blob 仍大）
7. **写 blob**：`UPDATE sessions SET session_data=?, updated_at_ms=?`
8. **置 Authoritative**：JSON 字段 `traceMigrationState` = `"trace_table_authoritative"`（**字段名 camelCase + 值 snake_case 双重大小写**）
9. **写幂等标记**：`store_metadata` key `storage_dedup.v1:<session_id>` = `{spec_version, completed, checksum, previous_state, migrated_at_ms}`（同事务）
   - checksum：按 turn_id 稳定排序后 `sha256(turn_id‖'\0'‖updated_at_ms‖'\0'‖sha256(trace_data 原文))`；UTF-8 字节、数字十进制；**基于最终表重新读取计算**（非内存对象）
10. **prune 调用点**：refs 写入后、同一事务内执行保护 prune（3.2）
11. 任一步失败 → ROLLBACK（blob 不剥离、表不写、标记不存在、prune 不生效）

**幂等跳过（四条件精确）**：
- marker 存在 ∧ checksum 匹配最终表 ∧ blob 内 `traceMigrationState == "trace_table_authoritative"` ∧ **refs 已写**（顶层 + 每节点 `turnTraceRefs` 存在；每个 ref 的 turnId 存在于最终表；ref 的 updatedAtMs 与 canonical trace 一致；无 dangling ref）
- 任一不满足 → 拒绝跳过，需 `--force`（重新 DELETE + INSERT）

**--force 决策矩阵**：
| 状态 | 无 marker | marker 匹配 | marker 失配 |
|---|---|---|---|
| 非 Authoritative | 迁移 | 迁移（--force 重跑） | 迁移（--force） |
| Authoritative + refs 完整 | 跳过（报告已最新） | 跳过 | 校验 refs，失配需 --force |
| Authoritative + refs 缺失/失配 | 需 --force 修复 | 需 --force | 需 --force |

**迁移后**：`VACUUM` + `wal_checkpoint(TRUNCATE)`（busy 降级仅 checkpoint 并告警）；VACUUM 失败不标记迁移失败。

### 3.2 refs 保护 prune（sqlite_session.rs）

- 只对 Authoritative 会话；只扫当前 session 的顶层 + 全部节点 refs
- refs 集合作为参数传入（调用方已有），**与 refs 写入同一事务**
- 只删无任何引用且超过软上限（128）的最旧记录（`trace_order ASC, updated_at_ms ASC, turn_id ASC`）
- fail closed：refs 缺失/null/结构异常/ref turn_id 为空 → 按未知引用处理（不删）；blob 损坏 → 零删除
- 默认关闭（迁移脚本触发一次），PA-089 常态化

### 3.3 回滚

- **精确恢复**：VACUUM INTO 备份
- **materialize 回填（降级）**：表 → 回填 blob 顶层 + 节点 trace → 按 `previous_state` 决定表去留 → 清 Authoritative → 删 marker；同事务；完成后 refs 完整性验证
- latest-wins 已丢弃旧版本，精确字节恢复用备份

### 3.4 latest-wins 预声明（P1-6）

fork 分歧 turn 以最新版为准（与 PA-088 一致）；materialize 等价性测试预声明排除分歧 turn。

## 4. 影响面与依赖

- **脚本**：compact-sessions-db.mjs（重写，Node ≥22，node:sqlite）
- **后端**：session.rs（ReplaceAll 全调用点 + collect_trace_union 稳定顺序）、sqlite_session.rs（prune + 最终防线）
- **依赖**：PA-088 完成（已满足）

## 5. 任务拆解

| # | 子任务 | 负责 | 依赖 |
|---|---|---|---|
| 1 | ReplaceAll 全调用点 union + collect_trace_union 稳定顺序 + 真实路径回归 | backend-dev | 无 |
| 2 | refs 保护 prune（同事务） | backend-dev | 1 |
| 3 | 迁移脚本重写（三源 + 事务 + 幂等 + 备份 + fail closed + 进程检测） | backend-dev | 1 |
| 4 | 迁移验证（dry-run + 实跑 + 重启回归 + 崩溃注入 + 真实数据） | tester | 2,3 |

## 6. 风险、回滚与迁移

- P0：状态值双重大小写（硬断言）；写表残留（DELETE+INSERT）；顺序（稳定规则）；幂等（四条件）
- P1：fail closed；进程检测；ReplaceAll 清空（全调用点）
- P2：旧版应用读新库（发布节奏：脚本随新版本同捆、迁移后禁止降级——release-facing）

**回滚**：备份恢复（精确）/ materialize 回填（降级，按 previous_state）

## 7. 测试计划

1. **materialize 等价性**：迁移前逐节点记录 `(turn_id, sha256(trace))`；迁移后真实 load_store 重开，断言节点 refs 解析内容一致（fork 分歧预声明排除）
2. **崩溃注入**：`PA_MIGRATE_CRASH_AFTER=<step>` + SIGKILL；marker 不存在、表与 blob 与迁移前一致；跨 session 崩溃后重跑只迁移剩余
3. **回滚闭环**：迁移 → 恢复备份 → 断言完全回到迁移前 → 再迁移成功
4. **marker 与 blob 不一致** → 拒绝跳过
5. **--force 且新 union < 现有表** → 旧行清空、表行数 == union 行数
6. **节点 trace 解析失败** → fail closed（不剥离、可恢复）
7. **trace_order 保序**：迁移后顶层顺序 == 迁移前
8. **进程检测**：app 运行中 → 脚本中止；dry-run 不建备份不写库不写 WAL
9. **状态值反序列化**：`traceMigrationState: "trace_table_authoritative"` 写入后 load_store 会话数不变；错误值脚本自检失败
10. **prune 单测**：被引用不删 / >128 删最旧未引用 / blob 损坏零删除 / legacy 不 prune / 悬空 refs 不保护
11. **真实数据**：拷生产 sessions.db（含 43.77MB 会话）到临时目录，迁移 + 重启回归 + <5MB 断言
12. **WAL 崩溃**：kill 后重开自动恢复
13. **ReplaceAll 真实路径回归**：checkout → 主事务持久化 → 重启 → 节点 materialize 完整（5 个调用点全覆盖）

## 8. 验收标准

1. 存量会话迁移后：blob < 5MB、顶层+节点仅 refs、表含全量 Union、状态 `trace_table_authoritative`
2. 迁移幂等可重跑（四条件跳过 + checksum + --force 决策矩阵）
3. 迁移事务原子（崩溃可恢复，fail closed）
4. 迁移后重启：checkout/fork/branch 语义不变（ReplaceAll 全调用点 union）
5. refs 保护 prune：被引用不删、损坏 fail closed
6. 回滚：备份恢复（精确）+ materialize 回填（降级）
7. 迁移后顶层 trace 顺序与迁移前一致

## 9. 审核记录

- v1（两路不通过）：状态值大小写 / merge_latest 不存在 / trace_order / 幂等跳过 / ReplaceAll 清空 / fork 分歧 / checksum / fail closed / 并发隔离
- v2（复审不通过）：ReplaceAll 5 调用点未全覆盖 / collect_trace_union 顺序不稳定 / 顶层 trace 未清空 / refs 已写无精确定义 / --force 冲突 / 进程检测不足 / prune 调用点
- v3（2026-08-15）：全部采纳（3.0 全调用点 + 稳定顺序 + 3.1 顶层清空/refs 精确定义/决策矩阵/进程检测/prune 调用点 + 7 测试 13 项）。待复审。