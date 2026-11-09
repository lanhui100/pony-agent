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

### 真实库尚未应用

应用正在运行且持有 `sessions.db.lock`（实测 PID 3969145 的交互式 `pony`，`/dev/pts/7`）。
写操作须避开运行实例，**待退出后**在真实库执行 apply；回滚面 = 删除对应日志目录（blob 未改动）。

## 实测发现的三个缺陷（2026-11-09，阻断真实库 apply）

在一致性快照上做「先只读观察、再 apply、再观察」的对照实验，暴露三处需要先修的问题。

**实验**：快照日志根原有 2 个分组 `--home-dm-pony-agent--`(3) 与
`--home-dm-pony-agent-crates-pony-agent-core--`(1)；从 `/tmp` 运行 apply 后，
新增出 **`--tmp--`(88)**，且 `skipped_has_log=0`。

1. **`session_has_log` 用进程 cwd，而不是会话自己的 cwd**（阻断级）
   `log_sink::log_path` → `store.log_path(id, &self.cwd)`，`self.cwd` 是构造时的工作目录。
   于是「会话在 A 工作区创建、在 B 工作区打开」时判定为「没有日志」。
   后果：① 迁移看不到已有日志 ⇒ 在另一个 cwd 组**重复生成**一份；
   ② 阶段 2-D 的写路径会把这类会话当成 grandfather，继续写整包 blob（瘦身失效，且日志与 blob 双副本可能分叉）。
   **修法**：日志路径的 cwd 段应取**会话自身 cwd**（日志 header 里就存了 `cwd`），
   或按 id 跨分组查找；判定不要再依赖进程 cwd。

2. **迁移产物的 cwd 分组用运行时的进程 cwd**（阻断级）
   同源问题：迁移把 88 个会话写进 `--tmp--`，与它们原本的工作区归属不符。
   日志头里登记的是进程 cwd，等于给会话伪造了一个归属。

3. **迁移过程中的状态读取口径会随已写日志漂移**（中）
   `migrate_grandfather` 在循环里对每个会话调 `load_store()`，而 `load_store()` 会合并
   **刚刚写好的日志**；因此先迁移的会话会改变后续会话的读取结果，计数出现漂移
   （同一快照上 dry-run 50 / apply 52；早前热库副本更明显）。
   **修法**：进入写循环前**一次性快照全部源状态**，之后只读该快照。

结论：**在 1、2 修好之前不对真实库执行 apply** —— 否则会给真实会话造出重复日志与错误的工作区归属。

### 三个缺陷的修复状态（2026-11-09，commit `9d6541e0`）

| 缺陷 | 修法 | 验证 |
| :--- | :--- | :--- |
| D1 跨 cwd 判「没日志」 | `SessionLogStore::find_log_path/find_session_meta` 跨分组按 id 查；`session_has_log` 改用它 | 用例 9：外部分组写日志的会话 `has_session_log=true`，2-D 写路径产出 `__index_only` 轻量行 |
| D2 产物写进进程 cwd | `resolve_session_cwd`：workspace 注册表 root → 该 id 既有分组 → 进程 cwd，降级逐条记 `cwd_fallback` | 用例 11：可解析者落对应分组；死 workspace id 计入 fallback |
| D3 计数漂移 | 进写循环前一次性快照源状态 + 注册表 | 用例 12：同一快照两次 dry-run 计数完全一致 |

真实库**副本** apply 实证：迁移前 `--home-dm-pony-agent--`(2) + `--…crates-pony-agent-core--`(1)；
apply 后 **无 `--tmp--` 分组**、原有分组 `diff -r` 字节一致、`skipped_has_log=3`、
`sessions` 行 139 条未增删。

### 两个遗留的**外观**问题（功能无影响，已记录）

1. `default` 工作区 `rootPath=/home/dm/pony_agent`（下划线）与仓库 `/home/dm/pony-agent`（连字符）
   是两个不同字符串 ⇒ 未盖章会话（`workspace_id=None`）被归属到 `--home-dm-pony_agent--`（56 个），
   与既有 `--home-dm-pony-agent--` 分成两组。功能无影响（D1 已按 id 跨分组查找），但目录观感分裂。
   根因是注册表数据（`/home/dm/pony_agent` 目录本身是历史遗留），**建议单独清理注册表**，不在迁移里改。
2. 从**非应用常态 cwd**（如 `/tmp`）运行迁移时，唯一一个「workspace id 在注册表里不存在」的会话
   会落 `--tmp--`，已计入 `cwd_fallback`（不静默）。
   **操作建议：从应用常态 cwd（如仓库根）执行迁移**，分组最干净。

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
