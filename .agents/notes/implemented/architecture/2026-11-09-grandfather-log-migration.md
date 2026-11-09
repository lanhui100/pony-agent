# Agent Note: 存量会话日志回填（grandfather 转正，ADR 0099 迁移）

Status: implemented

> 范围说明：迁移**能力**已交付并验证；真实库 **apply** 待应用退出后执行（写操作需避开运行中实例），
> 另有 51 个会话待事件载体补全后才能迁移。两项均记录在「真实库实测结果与未完成部分」。

## Problem

阶段 1–2 之后，新会话的真源是日志文件，但**存量会话只有索引 blob、没有日志**。
实测真实库：139 个索引会话里仅 3~4 个有日志 ⇒ 约 **136 个 grandfather 会话**。

后果：「删了索引不丢数据」这条承诺对这批会话永远不成立，两条读路径长期并存。

## Decision

**方案 A：从既有状态反推事件流，合成日志**（用户批准）。

1. `synthesize_log_events(state) -> Vec<PersistCommand>`：固定顺序
   `PublishMetadata → UpdateSessionMeta → AppendMessage* → UpdateHistoryNode* → UpdateCursor →
   UpdateBranch* → AppendTrace*`。
2. `migrate_grandfathered_sessions`：合成 → 写临时目录 → **fold 回来自证一致**（`session_parity_issues`
   逐字段）→ 一致才原子 `rename` 发布；不一致 ⇒ `mismatched`，**不落盘、blob 逐字节不变**。
3. **合成流自证身份**：日志 header 增加可选
   `"source": {"kind":"migrated-from-blob","migratedAtMs":N,"appVersion":"…"}`；
   正常写入路径**不写**该字段（`skip_serializing_if`），旧读方零影响，`read_header` 仍归 `Current`。
4. **保留 blob 作回滚面**：迁移只写日志目录，绝不删/改 `sessions` 行。回滚 = 删日志目录。
5. **幂等**：已有日志的会话跳过；`--force` 只重跑迁移产物，**永不覆盖原始过程流**。
6. CLI：`ponycli sessions migrate-logs [--dry-run] [--db] [--force]`，
   退出码 `0` 成功或无事可做 / `2` 参数或库非法 / `5` 存在 `mismatched` 或 `failed`。

## 关键取舍：宁可拒绝迁移，也不落一份"看起来一致"的坏日志

`mismatched` 不是失败兜底，而是**诚实边界**。反例（测试 [4]，最小可证伪）：
同一 `turn_id` + 同 `role` 的两条消息会被日志的去重语义（`message_id = {turn_id}-{role}`）
折叠成一条 —— 这类状态**合成流无法表达**，此时落盘会让「日志为准」的读路径静默少一条消息。

## Verification

```
[1] pa_3b_legacy_a: 合成流 fold 后与 blob 逐字段一致（history=4 nodes=2 branches=1 traces=1）
[2] 第一次 migrated=[...]；第二次 mode=apply scanned=2 migrated=0 skipped_has_log=2 mismatched=0 failed=0
[3] 原始日志 sha256 前后一致（--force 后仍一致）
[4] mismatched：pa_3b_unfaithful → history.len: fold=5 index=4（不落盘）
[5] dry-run：日志根 0 项指纹一致，db+wal sha256 一致
[6] 转正证明：DELETE sessions 行后仍读到 4 条消息 / nodes=2 / traces=1
[7] 合成 header 带 source；正常写入会话首行**无** source 键
[8] read_header="current"；bundle events=11 → 新目录 fold 与源逐字段一致
```
套件：`session_log_migration` 8 passed；回归 6 套全绿；`--lib` 1396；`cargo check` 0 warning；
`check-declaration` PASS。

## 真实库实测结果与未完成部分

### 干净副本上的完整实测（2026-11-09，应用运行中故用副本）

用 `git worktree` 在 HEAD 构建 CLI 后，对**已 checkpoint 的干净副本**执行：

| 步骤 | 结果 |
| :--- | :--- |
| dry-run ×2 | `scanned=140 migrated=90 mismatched=50 failed=0` —— **两次完全一致（确定性）** |
| apply #1 | `migrated=90 skipped_has_log=0` ⇒ 生成 90 个会话日志 |
| apply #2 | `migrated=0 skipped_has_log=90 mismatched=50` —— **幂等成立** |

### 一条实测陷阱（值得记录）

对**运行中实例的热库**直接 `cp` 三件套得到的副本**不可用**：同样的 dry-run/apply 在一次拷贝上
依次给出 `migrated=90 → 88 → 2` 的不稳定计数。原因是被拷贝的 WAL 未完成恢复，首次写入过程会
改变后续读取所见。**结论：迁移必须在应用退出后的库上执行，或对已 checkpoint 的副本执行**；
计数不稳定是拷贝口径问题，不是迁移器缺陷（干净副本上两次 dry-run 完全一致）。

### 50 例 blocked 的根因：日志格式缺载体（非迁移器缺陷）

`provider_native_transcript`(50) / `long_term_memory_entries`(1) / `history_state_evidence`
在 `PersistCommand` 里**没有对应命令**。

这同时暴露一条**由阶段 2-D 引入的潜在数据丢失路径**：有日志的会话，索引行把这些字段清空
（`index_only_row_payload` 全 `Vec::new()`），日志又未承载 ⇒ 一旦某会话产生 provider transcript，
重启后即丢。今日未丢属侥幸——实测 49 个有 transcript 的会话中 **0 个有日志**。

修复方向已立提案：[ADR 0102（proposed）](../../../docs/decisions/proposed/0102-session-log-event-carriers-for-transcript-and-memory.md)。
完成后 50 例可迁、140 个全部转正。

### 真实库已应用（2026-11-09，应用退出后）

用户关闭 TUI、锁释放后，在**真实库**执行（从仓库根运行）：

| 步骤 | 结果 |
| :--- | :--- |
| 迁移前 | 日志 4 个（`--home-dm-pony-agent--` 3 + `--…crates-pony-agent-core--` 1） |
| dry-run | `scanned=140 migrated=86 skipped_has_log=4 mismatched=50 cwd_fallback=1 failed=0` |
| apply | 同上；生成 **86** 个会话日志，总计 **90** 个 |
| 迁移后分组 | `--home-dm-pony-agent--`(31) / `--home-dm-pony_agent--`(56) / `--…crates-pony-agent-core--`(1) / `--home-dm-ponyllm--`(1) / `--tmp-pa-approval-demo--`(1)；**无 `--tmp--`** |
| 幂等复核 | 第二次 dry-run：`migrated=0 skipped_has_log=90` —— 幂等成立 |
| 索引未动 | `sessions` 行数 140 → 140（未增未删、blob 未改，回滚面完好） |

**转正率 90/140**；其余 50 个因日志格式缺载体被如实拒绝（见上文）。

**第二轮（补齐载体后，同日）**：ADR 0102 落地 `AppendProviderTranscript` / `UpdateSessionMemory` 后，
真实库 `dry-run` 变为 `migrated=50 skipped_has_log=90 mismatched=0`，apply 后
**140/140 会话全部转正**；二次 dry-run `migrated=0 skipped_has_log=140`（幂等），
`sessions` 行数仍 140（blob 未改，回滚面完好）。

### 应用后发现的一处待办：34 个会话「日志 vs 索引视图」不一致

在真实库上跑 `ponycli sessions rebuild-index --dry-run` → `mismatched=34`（占 90 个有日志会话的 38%）。

抽查确认**不是日志侧缺数据**，而是**索引视图仍是旧的、且 `normalized_*` 投影不完整**：
三个应用自己写的既有日志对比结果——

```
pa_01a1bdad…  blob: hist=0 nodes=0 | log: msgs=4 traces=2 nodes=2
pa_01a1bdfc…  blob: hist=0 nodes=0 | log: msgs=6 traces=3 nodes=3
pa_01a1ba70…  blob: hist=0 nodes=0 | log: msgs=0 traces=1 nodes=0
```
blob 早已是轻量索引行（2-D 所致），真实内容在日志里。因此这 34 条的正解是
**`rebuild-index` 用日志回填索引**（日志是真源，索引是派生），而不是修日志。
建议在确认无并发放写后执行一次 `ponycli sessions rebuild-index --db …`，
并以 `mismatched=0` 作为验收。

### 关于 `rebuild-index` 对齐索引：**暂不执行**（2026-11-09 实测结论）

用户同意后先跑 dry-run，结果出乎预期：`scanned=140 rebuilt=140 mismatched=49`。
进一步核对 mismatched 的索引行形态：**3 个是轻量行、46 个仍是整包 blob**（即刚迁移的那批）。

抽查 `pa_01a1b478…`：

| 侧 | history | nodes | traces | transcript |
| :--- | ---: | ---: | ---: | ---: |
| blob（索引行） | 24 | 20 | **0** | 34 |
| 日志命令流 | 24 append_message | 20 nodes | **19 append_trace** | 1 条命令（携带整份） |

计数在 history/nodes/transcript 上一致，差异集中在 **trace 面**——这与
`trace_migration_state = legacy_blob` 的既有约定一致（blob 的 `turnTraceHistory` 为空，
trace 在独立表里）。迁移的 `session_parity_issues` 知道这条豁免，而 `rebuild-index` 的比较器
显然**没有应用同一套豁免**。

因此：**不能盲目 `rebuild-index --apply`** —— 它会把 46 个仍是整包 blob 的索引行按 fold 结果重写，
而这些行的 trace 语义（表 vs blob）与 fold 输出并不等价，属于「用比较器缺陷去覆盖数据」。

**下一步（已作为 task-13 提出）**：给 `rebuild-index` 增加 `--explain <session-id>`，
逐字段打印差异（字段名 / fold 值摘要 / 索引值摘要），先定性再决定是否 apply。

### `rebuild-index` 比较器修复与 `--explain`（2026-11-09，commit `8adc47ad`）

**先纠正前一段的诊断**：那 49 个 `mismatched` 的根因**不是 trace 面**，而是 **`workspace_id`**
（日志无载体、fold 侧恒为 `None`）；trace 面本来已被 `merge_trace_history` 按 `legacy_blob` 约定合回索引视图。

修法与结果：

| 项 | 内容 |
| :--- | :--- |
| 唯一字段清单 | `session_parity_differences()` |
| 唯一豁免清单 | `LOG_UNCARRIED_STATE_FIELDS = ["workspace_id", "last_commit_watermark"]` |
| 派生使用者 | `rebuild-index` 的 `mismatched`、迁移比对、`--explain`；`MIGRATION_PARITY_EXEMPTIONS` 退化为别名（有测试断言两侧相等） |
| 语义归一 | 比对 trace 前剔除 `buildContextObservation` **载荷**（保留 ref）—— PA-095 外置投影会造成永久假阳性；ref 漂移仍算差异 |
| 新命令 | `ponycli sessions rebuild-index --explain <id>`：逐字段摘要（条数 + 首条前 80 字符 + 首处差异字段名），被豁免字段单独 `ignored` 行，无差异打印 `no difference`，与 `--dry-run` 互斥，只读 |
| 真实验证 | 一致性快照上 `mismatched 49 → 4` |

**残留 4 例是真差异，未豁免**（rebuild 会抹掉它们，`mismatched` 拦得对）：

1. 2 例（`pa_01a1ba70…` / `pa_01a1be18…`）：日志里**没有** `update_session_meta` / `update_branch` /
   `update_cursor` 命令（只有 trace / transcript 类），fold 侧因此缺 title/branches/cursor。
2. 2 例（`pa_01a1bdad…` / `pa_01a1bdfc…`）：`legacy_blob` / `dual_write` 会话的 trace **终态注解**
   （`emittedAtMs` / `eventId` / `eventType` / `eventVersion`）与 **hook 记录**只写索引表、不进日志。

⇒ 后续任务：让 `annotate_turn_trace_terminal_event` / `append_hook_records` 在非 authoritative 会话上也发
`UpdateTraceTerminal` / `AppendHookRecords`；并让「只写 trace」的会话补发 meta/cursor/branch 命令。
补齐后 `rebuild-index` 的 `mismatched` 才有望真正归零。

### 一条独立缺陷线索（值得单独开卡）

新增用例并进既有 `session_fold_rebuild.rs` 时，会抬高该测试二进制的进程内并发，**偶发触发既有单实例
flock 竞态**（`single_instance_lock_denied` → `attempt to write a readonly database`，6 轮里红 2 轮，
失败的总是既有用例）。搬到独立测试二进制后 6/6 轮稳定。属 `acquire_single_instance_lock` 在高并发下的
降级行为问题，**本次未改**。

## Alternatives considered

1. **方案 A 合成日志 + 自证一致** —— 采纳。可审计（header 标记）、可回滚（保 blob）、可幂等，
   且不一致时拒绝落盘而不是掩盖。
2. **方案 B 不迁移**（老会话永远走 grandfather）—— 否决。「删索引不丢数据」对 136 个会话永远不成立，
   两条读路径长期并存，成本永久化。
3. **方案 C 迁完即删 blob** —— 否决。不可回滚，且合成逻辑一旦有缺陷就没有退路。
4. **把 51 例强行标记为一致并落盘** —— 否决（本质是伪造一致性）。迁移后「日志为准」会让这些会话
   静默丢 transcript / 记忆，最坏情况比不迁移更糟。
5. **直接给这些字段开「索引独有投影」继承** —— 否决。与 2-D 的瘦身目标直接冲突（这些字段随轮次增长），
   等于回退 2-D。正确解法是让日志格式承载它们。
