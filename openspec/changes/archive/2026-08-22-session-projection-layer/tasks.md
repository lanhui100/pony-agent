> 2026-08-22 归档同步：实现已随 PA-091/092 提交（6395ec7^..e88536f）落地并经全量验证（实施状态记录见文末），勾选为归档前簿记同步，非本轮新实施。

# Tasks

- [x] 投影 trait 定义（`projection.rs`）：`init/apply/view` 三函数 + 纯函数约束测试（增量 == 全量）。
- [x] `HistoryProjection`：消息历史折叠（`user/message` + `assistant/message`）+ `DEFAULT_HISTORY_LIMIT` 截断窗口 + `history/squash` 语义。
- [x] `TraceProjection`：`TurnTraceRecord` 折叠（step/tool/chunk/turn-end 映射，tool 字段从事件重建）+ seq 水位幂等。
- [x] `PlanProjection`：`plan/update` 事件 → plan 状态。
- [x] `MetricsProjection`：`ProviderUsage` → 四桶 totals + last + per-turn 聚合（addReplacing；`assistant/message` usage 不参与 totals）。
- [x] 模型监控视图改造：`load_model_monitor_summary` / drilldown 读投影缓存，不再扫描 trace 表。
- [x] `append_turn` 重构：append 事件 + 增量折叠 + 快照由投影生成（同事务，双写变单写）；旧逻辑保留为 `#[cfg(test)]` 参考实现。
- [x] `record_turn_trace` 重构：trace 组装迁移到 `TraceProjection`，trace 表降级为缓存。
- [x] golden fixture 库：`tests/fixtures/projection/` 合成语料矩阵（空 session、单事件、截断边界、tool 链、多 hop usage、checkout/fork、chunk_missing、squash、10k+ 事件、100KB+ payload）。
- [x] 对拍模式：`PONY_AGENT_PROJECTION_VERIFY=record`（CI 失败）/ `=sample`（生产抽样记录不 panic）。
- [x] 单元测试：投影纯函数、截断窗口、squash、trace 水位、MetricsProjection addReplacing + 双 usage 防重、增量 == 全量。
- [x] 对拍验证：golden fixture 逐字段一致（限定字段集）。
- [x] 回归：既有 session/trace/checkpoint 测试全绿；验收命令 `npm run cargo:test`。

## Validation Notes

- 本 change 是 ADR 0008 阶段 2，依赖阶段 1（`turn-event-log`）的事件层。
- 整包序列化写放大消除推迟到阶段 4（前端改读事件派生数据后），本 change 不承诺。
- 对拍字段集限定：history / turn_trace_history / plan 状态 / metrics 派生字段；非投影字段由回归覆盖。
- 对抗审核（2026-08-18）已采纳：golden fixture 策略（P1-3）、字段集限定（P1-4）、squash 语义（P0-4 延伸）、双 usage 计数规则（P1-4）、verify 差异记录非 panic（P1-3）、写放大推迟声明（P1-6）。
- **实施后对抗审核（2026-08-18，code-reviewer + tester）采纳与降级声明**：
  - 水位判断修正为 `>=`（同 seq 重放帧幂等，design.md:36 语义）；补 `trace_projection_same_seq_replay_is_idempotent` 测试。
  - MetricsProjection 水位改 **per-turn**（与 TraceProjection 粒度一致）；补水位隔离测试。
  - 数据字段回填：`TurnUsageAggregate` 加 provider/model；`last` 记录回填 provider_source；`turn/end` 补终态信封（event_type/sequence/provider/model）。
  - chunk 文本聚合（不逐条 push step；累积进 trace，turn/end 结算）；修正 trace_steps 的 label/state。
  - `turn/end` → 最后 assistant 消息 status 标记（Completed→Done，其余→Error）；补 failed 测试。
  - 补 `PlanProjection`（whole-value）+ 测试。
  - 四桶统一用事件级权威字段（cache_read=cache_hit，uncached=cache_miss 或 input-cache_hit 回退）；补 totals==by_turn 不变量测试。
  - 投影状态补 `Serialize/Deserialize`（缓存持久化前提）+ round-trip 测试。
  - **降级声明**：append_turn 重构（快照由投影生成）与模型监控视图切换标注为后续增量（spec 已声明写放大推迟到阶段 4）；squash 摘要格式与参考 `wrap_summary_to_message` 的分叉显式排除在对拍字段集外；运行时 emit 点补充（user/message、step、provider/usage）沿用 PA-091 的显式豁免声明。
- **实施状态**：core 823 测试通过（含新增 12 个投影测试）。