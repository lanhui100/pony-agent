# ADR 0102：为日志格式补齐事件载体（provider transcript / 长期记忆 / history_state_evidence）

Status: implemented

## 背景：一条由阶段 2-D 引入的潜在数据丢失路径

ADR 0099 阶段 2-D 让「有日志的会话」只写轻量索引行（`sessions.session_data` 不再写整包 blob）。
轻量行的字段取舍见 `sqlite_session_commit.rs::index_only_row_payload`：内容字段一律清空，
理由是「它们随轮次增长，正是要消灭的增长源，且日志真源已经承载」。

**但这个理由对三个字段不成立** —— 日志命令流里没有承载它们的命令：

| 字段 | 是否随轮次增长 | 日志是否有载体 | 现状 |
| :--- | :--- | :--- | :--- |
| `provider_native_transcript` | 是 | ❌ 无 | 索引行清空 + 日志无载体 ⇒ **丢失** |
| `long_term_memory_entries` / `memory_write_evidence` / `memory_write_hook_trace_records` | 是 | ❌ 无 | 同上 |
| `history_state_evidence` | 是 | ❌ 无 | 同上 |

因此：**任何有日志的会话一旦产生上述状态，重启后即丢**（读路径「日志为准」拿不到，
索引行也不留）。今日尚未丢属侥幸——实测真实库 49 个有 transcript 的会话中 **0 个有日志**。

第二个佐证来自存量迁移（ADR 0099 迁移阶段）：副本 dry-run
`migrated=86 / mismatched=51`，51 例中有 50 例卡在 `provider_native_transcript`、
1 例卡在 `long_term_memory_entries` —— 迁移器拒绝落盘，正是因为它自证「合成流表达不了这些状态」。

## 决策

1. **新增事件载体**：`PersistCommand` 增加
   `AppendProviderTranscript { session_id, epoch, messages: Vec<Value> }`
   与 `UpdateSessionMemory { session_id, epoch, entries, evidence, hook_trace_records }`；
   `history_state_evidence` 归入既有 `UpdateSessionMeta` 的 patch（或同批新命令，按落码时的最小闭包选）。
2. **fold 侧对称实现**（追加 / 覆盖语义必须唯一且写进注释）：
   - transcript：**追加**（与 `store.rs` 的 `extend` 语义一致）；
   - 记忆四件套：**整份替换**（与 `update_long_term_memory_from_user_message` 的产物一致）；
   - `history_state_evidence`：追加。
3. **写路径**：`append_turn`（及任何写入 transcript / 记忆的路径）在原有命令批里追加对应命令，
   **日志先行**语义不变（日志写失败即整体 `Failed`，SQLite 零改动）。
4. **编码**：`log_command.rs` 补齐两类命令的编解码，保持 `{"seq","ts","cmd"}` 信封与自描述 `type`。
5. **兼容**：新增命令类型**不 bump** `EVENT_SCHEMA_VERSION`（结构不变，仅新增变体）；
   旧读方遇未知命令按既有契约 **fail loud**（`FoldError`），不静默跳过。
6. **迁移联动**：`log_migrate::synthesize_log_events` 补上这两类命令后，
   副本上 `mismatched` 应从 51 降到接近 0；届时重跑 dry-run 作为验收。

## 实施结果（2026-11-09，commit `8795d467`）

- 两个载体落地：`AppendProviderTranscript`（追加）/ `UpdateSessionMemory`（记忆三项整份替换 + evidence 追加）；
  语义差异写进命令 doc、`Folder::apply` 臂 doc、索引写入函数 doc 三处。
- `canonical_commands` 同步补齐 ⇒ `rebuild-index` 不再丢这些字段。
- 写路径只在原命令批里追加两条命令；日志先行语义不变（日志不可用 ⇒ `Failed` 且 db+wal sha256 未变）。
- **真实库 dry-run：`mismatched 50 → 0`、`migrated 50`**；apply 后 **140/140 会话全部转正**，
  二次 dry-run `migrated=0 skipped_has_log=140`（幂等），`sessions` 行数 140 未增删。
- 新套件 `session_event_carriers` 6/6；回归 8 套全绿；`--lib` 1404；`cargo check` 0 warning；
  `check-declaration` PASS。

### 遗留风险（已在实施中确认）
旧版本二进制读到含新命令的日志会 `FoldError::UnknownCommand` **fail loud**。
本地单机同版本读写无影响；**混版本部署（桌面旧版读新版日志）必须在发布说明中点名**。

## 验收

- 新事件载体的往返：写入 transcript / 记忆 / evidence → 重启（重新 `with_backend`）→ 读回**逐字段一致**
- **有日志会话的 transcript 不再丢**：构造一个产生 transcript 的会话（有日志），
  断言 `load_store()` 读回非空，且**索引行仍为轻量行**
- 存量迁移：副本 `migrate-logs --dry-run` 的 `mismatched` 从 51 降至 0（或每例都有明确结构性理由）
- 既有门禁全绿：`session_log_migration`(8)、`session_stop_blob_write`(7)、`session_read_from_log`(8)、
  `session_fold_rebuild`(8)、`session_log_wiring`(7)、`session_log_store`(6)、
  `session_bundle_export_import`(9)、`storage_stage0_*`(11)、`--lib`、`cargo check`、`check-declaration.sh`

## 风险

1. **契约变更面**：`PersistCommand` 是跨模块契约（`backend.rs` 定义，被 `sqlite_session` / `fold` /
   `log_command` / `log_migrate` / 测试消费）。新增变体会让所有 `match` 穷尽性检查报错 —— 这是**好事**
   （编译器强制每个消费点表态），但改动面约 5~6 个文件。
2. **旧读方 fail loud**：旧版本遇到新命令会 `FoldError`。对**本地单机**无影响（同一二进制读写），
   但若存在混版本部署（桌面旧版读新版日志），会读不出该会话 —— 需在发布说明中点名。
3. **体积**：transcript 与记忆可能很大。需在实现时给单命令 payload 设上限并记录日志，
   避免一次 turn 写出数十 MB 的日志行。

## Alternatives considered

1. **保持现状，把这 51 个会话永远留在 grandfather** —— 否决。数据丢失路径仍然敞开，
   且「删索引不丢数据」对这批会话永远不成立。
2. **让读路径从索引行继承这些字段（相当于回退 2-D 的瘦身）** —— 否决。这些字段正是随轮次增长的
   增长源，回退等于把 102 MB/turn 的写放大重新引回来；且「日志为准」会被局部破例。
3. **把 transcript / 记忆塞进 `UpdateSessionMeta` 的大 patch** —— 否决。`SessionMetaPatch` 是
   **单值元数据**语义，塞进入会造成「整份替换 vs 追加」语义混杂，且体积不可控。
4. **不进日志，另存独立文件（如 `transcript.jsonl`）** —— 否决。等于在会话目录里再造一份真源，
   破坏「一个会话一份 append-only 日志」的单一事实来源；且迁移/传输都要多处理一类文件。
