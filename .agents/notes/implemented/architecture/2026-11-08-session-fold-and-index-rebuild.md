# Agent Note: 从会话日志 fold 出状态并重建 SQLite 索引（ADR 0099 阶段 2-A）

Status: implemented

## Problem

阶段 1-B 之后，会话日志文件已是写入真源，但**读路径仍只读 SQLite**。这留下三个悬空：

- 日志里的事实无法被消费，「真源」只是写侧名义；
- 索引损坏或 schema 不兼容时没有恢复手段，「索引可重建」这半条承诺未兑现；
- 混版本部署下旧版仍会因索引 schema 冲突拒绝工作，而日志本身是兼容的。

## Decision

1. **`fold_session(events) -> SessionState` 是纯函数**：不读盘、不读时钟、不读全局状态；
   所有时间戳来自日志行的 `ts`。覆盖全部 `PersistCommand` 变体（含 `PublishMetadata`），
   未知/坏 cmd 返回 `FoldError`（**不静默跳过**）。
2. **语义与索引侧对齐**：`AppendMessage` 的 `message_id = {turn_id}-{role}` 去重、
   `meta_patch` 的 `Option<Option<T>>` 三态（`None`=不改 / `Some(None)`=清空）、
   `UpsertTurn`/`UpdateHistoryNode`/`UpdateCursor`/`UpdateBranch` 的 upsert 语义。
3. **`rebuild_index(opts)`**：扫 `session-logs/` 逐个 fold 并覆盖写索引；
   `dry_run` 只统计不写；幂等（连跑两次结果一致）。
4. **无日志会话一律不动**：存量会话没有日志文件，重建只能覆盖「有日志」的会话，
   其余原封不动并计入 `skipped_no_log`。这是硬约束——否则一次重建会清空全部历史会话。
5. **CLI**：`ponycli sessions rebuild-index [--dry-run] [--db]`，复用 `backfill` 的 `--db` 校验。

## 两个已知缺口（写成 allow-list 测试，未放宽断言）

1. **blob 视图 vs 命令派生投影的差异**：legacy blob 另有整包写产物（`{sid}-legacy-root` 节点、
   陈旧 trace refs），而命令流只派生 `normalized_*` 投影。因此对拍基准是「命令派生投影」
   （`load_store()` 在 `phase=retired` 下的视图），blob 视图的差异被钉成 allow-list，
   出现清单外差异即红。
   **真实库副本实测**：136 会话中 `scanned=1 rebuilt=1 skipped_no_log=135 mismatched=1`——
   唯一 mismatch 就是这类「日志只有 1 条 append_trace、没有 meta/消息命令」的会话。
2. **`AppendHookRecords` 口径分裂**：fold 按**追加**，索引侧 legacy 表按**替换**。
   阶段 2-B 切读路径前必须统一，否则 hook 记录会在两条读路径上不一致。

## Verification

在隔离 `git worktree`（HEAD 662e8807）上验证，因主仓当时被另一 lane 的 `tools/*` 拆分中断：

- `--test session_fold_rebuild` → 9 passed（D 节 7 类 + 撕裂尾/未知命令 2 条附加）
- `--test session_log_wiring` → 7 passed
- `--test session_log_store` → 6 passed
- `--lib` → 1387 passed
- `cargo check` → 0 warning
- `check-file-size` → 0 new violation
- `ponycli sessions rebuild-index --dry-run --db <真实库副本>`：sha256 前后一致、未生成 `-wal`（零写入）
- 「无日志会话未被覆盖」：造只有索引行的会话 → 重建后 `rebuilt=[pa_2a_logged]`、`skipped_no_log=1`，
  该会话 JSON 逐字节相同（`title`、`history`、`archived` 均未变）

## Alternatives considered

1. **只实现 fold，不实现 rebuild_index**：改动最小，但「索引可重建」的承诺继续悬空，
   且没有「无日志会话不被覆盖」这条护栏的测试落点。否决。
2. **重建时把无日志会话也清掉（以文件为唯一真相）**：语义最纯，但会**一次性删除全部存量会话**
   ——用户虽声明「无重要历史数据」，也不该由一次内部工具默认执行不可逆删除。否决。
3. **fold 直接复用 SQLite 的写入实现（把 fold 写成"回放到 SQLite"）**：能保证口径一致且省事，
   但它把 fold 变成有副作用的操作，失去纯函数可测试、可 off-thread、可对拍的特性。否决。
4. **在 fold 里静默跳过未知 cmd**：实现简单且不会红，但违反「未知事实必须 fail loud」——
   静默跳过会让重建结果悄悄缺内容。否决。
5. **等主仓恢复绿再验证**：会把工作卡在他人 lane 的进度上。改为隔离 worktree 验证 + 主仓恢复后重跑，
   两者都留证据。采纳（同时保留主仓恢复后重跑的要求）。
