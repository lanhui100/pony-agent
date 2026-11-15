# Agent Note: 读路径折叠预检降级会永久吞掉「日志有、索引无」的会话

Status: implemented

## Problem

`/resume` 的列表来自 `control_plane.list_sessions()`，而 `list_sessions()` 读的是
`load_store()` 产出的**内存视图**：索引行 ∪ 日志 fold 结果
（`fold::merge_log_truth_into_sessions`）。日志 fold 有一条 B047 引入的**字节预检**
（`DEFAULT_READ_FOLD_LOG_BYTES = 32MiB`）：日志根下所有会话日志的字节总量超过预算时，
**一次日志 IO 都不做**，整轮降级为「只读索引视图」。

这条降级在本机是**每轮都触发**的（运行时日志实测）：

```text
[pony-agent][session] session_fold_precheck: log_bytes=1583MB byte_budget=32MB sessions=223
  — 日志总量超出读路径折叠字节预算，本轮整轮按索引视图（不读日志）
```

后果是一个**永久性**的可见性缺口：会话的权威内容在 `session.jsonl`，索引行则来自
`save_to_backend` 的提交点。任何「日志已经写了 turn、索引还没提交」的会话（进程被杀、
崩溃、提交边界之前退出）在索引里没有行；而预检又让日志永远不被读 ⇒

- `/resume` 里看不到它（列表只有索引行）；
- 索引也**永远不会自愈**：没有任何路径把「日志有、索引无」的会话写回索引。

B047 引入预检的动机（避免「读满 800ms 又被整轮丢弃」的纯损耗）仍然成立——问题不在
预检本身，而在于它把「无法用索引兜底的会话」和「索引已覆盖的会话」一起扔掉了。

## Decision

字节预检降级时，追加一轮**定向补折**（`fold::fold_missing_sessions_from_logs`）：
只折「日志里有、视图里没有」的会话，其余会话仍按索引视图（B047 的零损耗语义不变）。

补折的成本有三条硬上界，且全部确定性可判（同一批日志两次装载给同一结论）：

1. **候选集**：只取 `log_store` 里存在、而 `sessions` 视图里没有的 id；索引已覆盖的
   会话一次日志 IO 都不做（单测 `targeted_fill_skips_sessions_covered_by_index` 锁死）；
2. **单会话上限** `DEFAULT_READ_FOLD_SESSION_BYTES = 8MiB`：超大日志不补折，交给显式
   `pony resume <id>` 的定向装载（`load_session_targeted`）；
3. **总量 + 时预算**：与全量折叠同一口径（32MiB / 800ms），候选按日志 **mtime 降序**
   （近者优先）：预算耗尽时丢掉的是最老的会话，而不是用户刚在用的那个。

落位规则不新增第二份：全量折叠与定向补折共用 `fold_meta_into_view`（单会话折叠）与
`apply_folded_into_sessions`（索引合并），两条读路径对同一会话必给同一结果（B148 的
验收红线）。

降级名单语义随之收敛：`skipped_due_to_budget` 只留「本轮确实没读」的会话——补折成功的
会话从名单里剔除并去重，因此「所有缺失会话都被补回」的那一轮 `degraded()` 为假。

## Alternatives considered

- **把 32MiB 预算调大（或按库规模动态放大）**：不解决结构问题。本机日志 1583MB，任何
  合理预算都会被击穿；且预算一旦被击穿，代价从「丢若干会话」变回「白付秒级 IO」。
- **预检降级后照旧全量折叠（删掉预检）**：等于回退 B047。实测日志 682MB / 154 会话时
  折叠读到 800ms 只折完 10 个会话就被整轮丢弃，那 880ms 是纯损耗，启动期体验直接退化。
- **让索引投影兜底（把「日志存在、索引无行」也投影成占位行）**：占位行只有 id 与标题，
  没有内容；用户能看见却打不开（`/resume` 后续装载仍要读日志），比「看不见」更具欺骗性。
  且它把「索引可删可建」的承诺变成「索引必须永远完整」，与 ADR 0099 阶段 2-D 停写整包
  blob 的方向相反。
- **启动期后台线程异步补折、先出列表再刷新**：体验最好，但需要给 `load_store` 引入
  「视图可变 + 版本号」契约（列表在两次读取之间跳变，正是预检当初要消灭的不确定性）。
  留作后续方案，不在本次修缺陷的范围里。
- **只在 `pony resume <id>` 时定向装载（现状已支持）**：解决「打不开」，不解决「看不见」；
  用户没有 id 可输入，缺口依旧。

## Consequences

- 语义：`/resume` 不再因为库规模而丢掉索引未覆盖的会话；代价是大库启动时最多多付
  32MiB / 800ms 的日志 IO（且只在**确实存在**缺失会话时发生）。
- 可观测：新增一行运行期日志 `session_fold_targeted_fill: folded/ skipped/ elapsed=`
  （与既有 `session_fold_precheck` / `session_fold_slow` 同一命名族），用于判断补折是否
  真的发生、以及预算是否被击穿。
- 测试：`crates/pony-agent-core/src/agent/session/fold.rs` 新增 4 条单测（补回、索引覆盖
  跳过、单会话上限、总量上限），`session::fold` 19 条全绿。
- 残留：补折只覆盖「日志还在」的会话。日志已被删/从未落盘的会话不可恢复——这是数据面
  事实，不是读路径能修的缺陷。
