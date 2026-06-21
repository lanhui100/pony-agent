# Tasks: Split Session Trace Storage And Expand Targeted Persistence

## 1. Task-System And Spec Alignment

- [x] 1.1 新增 `PA-058` 任务卡，明确 P3 目标、范围与验收标准
- [x] 1.2 新增 OpenSpec change：`split-session-trace-storage-and-targeted-persistence`
- [x] 1.3 将本轮性能诊断、P1/P2 收口结果与 P3 风险边界转写为 proposal / design / spec / tasks

## 2. Backend Contract Preparation

- [x] 2.1 为 `SessionBackend` 设计 trace 级读写接口，含默认实现
- [x] 2.2 明确 live trace 与 history node trace 的 ownership contract  
- [x] 2.3 保持 `SessionSnapshot.turn_trace_history` 形状不变，禁止前端 contract 漂移  
- [x] 2.4 更新 `MemorySessionBackend` 实现新 trace 方法以覆盖测试
- [x] 2.5 定义具体的 Rust 方法签名，含参数类型与返回类型
- [x] 2.6 定义 trace migration state（`LegacyBlob / DualWrite / TraceTableAuthoritative`）及其持久化位置
- [x] 2.7 区分 backend 返回语义：unsupported / not found / write failed，避免默认 `false` 混淆

## 3. SQLite Schema And Load Path

- [x] 3.1 新增 `session_turn_traces` schema 与索引
- [x] 3.1.1 在 `save_store` 全量写清理循环中增加 `DELETE FROM session_turn_traces`
- [x] 3.2 `load_store` 支持合并双读（非二选一）：按 `turn_id` 去重，新表优先，保持插入序
- [x] 3.3 增加 legacy session reload 验证，确保旧 `session_data.turn_trace_history` 不丢
- [x] 3.4 在 cutover 前完成现有 session 的 trace backfill，禁止把 backfill 仅作为可选优化
- [x] 3.5 为 `TraceTableAuthoritative` session 禁用 legacy blob fallback，防止已驱逐 trace 被旧 blob 复活
- [x] 3.6 定义稳定顺序规则，确保 annotation / hook append 不会在 reload 后重排旧 turn

## 4. Hot Trace Write Migration

- [x] 4.1 `record_turn_trace` 切到 trace 级写入
- [x] 4.1.1 `upsert_session` 在 trace 级写入后跳过 `turn_trace_history` 序列化
- [x] 4.1.2 实现 `session_turn_traces` 的 `DEFAULT_HISTORY_LIMIT` 清理逻辑
- [x] 4.2 `annotate_turn_trace_terminal_event` 切到 trace 级更新
- [x] 4.3 `append_turn_trace_hook_records` 切到 trace 级更新  
- [x] 4.4 `append_failed_turn` 切到 trace 级写入
- [x] 4.5 新增 `SessionBackend::delete_session_traces` 方法
- [x] 4.6 新增 `replace_session_traces(session_id, traces)`，供 checkout / restore / fork / switch 在同一事务中整体替换当前分支 trace materialization

## 4.5 双写迁移窗口

- [x] 4.5.1 按固定顺序 rollout：`Off -> DualWrite -> WriteSeparate`
- [x] 4.5.2 在 `DualWrite` 窗口期内支持双向写入（新表 + 旧 blob），且包裹在同一事务中
- [x] 4.5.3 配置 `use_separate_trace_table` 三个 flag 状态：Off / DualWrite / WriteSeparate
- [x] 4.5.4 明确 `WriteSeparate` 的启用前提：backfill 完成、至少一个完整发布周期 dual-write、drift 检查通过

## 5. Compatibility And Recovery

- [x] 5.1 `load_turn_traces` 保持读面兼容
- [x] 5.2 history restore / checkout / branch switch 仍可读取正确 trace
- [x] 5.2.1 checkout 后产生新 trace 再 reload：双源合并仍保持完整
- [x] 5.3 monitor / control-plane 读取链路继续通过现有 snapshot surface 工作
- [x] 5.4 branch / restore 路径使用 `replace_session_traces` 清除非当前分支残留 trace
- [x] 5.5 单条坏 `trace_data` 行解析失败时，记录结构化错误并安全跳过

## 6. Verification

- [x] 6.1 dual-read SQLite reload tests（含合并双源、去重、排序场景）
- [x] 6.2 trace upsert / annotate / hook append / failed-turn roundtrip tests
- [x] 6.3 history graph / restore regression tests
- [x] 6.3.1 checkout 后写入新 trace + reload 一致性测试（分支隔离验证）
- [x] 6.4 真实性能对比：trace 热路径不再重写 session blob
- [x] 6.5 crash recovery 回归测试：双写中途崩溃后 reload 不丢 trace
- [x] 6.6 所有 roundtrip 测试针对 `SqliteSessionBackend` 覆盖（不限于 `MemorySessionBackend`）
- [x] 6.7 session 删除后 `session_turn_traces` 无残留的验证
- [x] 6.8 启动时可选完整性检查：记录 `session_turn_traces` 中的孤立行数
- [x] 6.9 fallback 抑制验证：`TraceTableAuthoritative` session 不会从旧 blob 复活已驱逐 trace
- [x] 6.10 边界顺序验证：相同 `updated_at_ms`、重复 annotation、hook append 后 reload 顺序稳定
- [x] 6.11 dual-write drift 观测验证：merge 覆盖计数、fallback 命中、prune 次数、flag 状态可读
- [x] 6.12 corruption 回归测试：单条坏行不会拖垮整 session reload
