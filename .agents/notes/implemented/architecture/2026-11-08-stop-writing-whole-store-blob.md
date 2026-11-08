# Agent Note: 停写整包 blob，SQLite 退为纯索引（ADR 0099 阶段 2-D）

Status: implemented

## Problem

2-B 之后读路径已合并日志真相，但**写路径仍在写整包 blob**（`sessions.session_data`）。
残留三个后果：

- 同一份状态存在两处（日志 + blob），靠 `updated_at_ms` 定序妥协，过渡逻辑本身就是潜在分歧源；
- blob 随轮次线性增长，「文件即真相」在成本上仍未兑现；
- 索引里始终躺着完整状态的第二副本，一旦有人误读它当真相，就会读到陈旧版本。

## Decision

1. **有日志的会话只写轻量索引行**：`conversation_id` / `title` / `updated_at_ms` / `version`
   + 哨兵 `"__index_only": true`（实测 763B，1 轮与 3 轮字节相等）。
2. **迁移策略：保持 `session_data NOT NULL` + 未知键哨兵**，不放宽为可空。
   理由：SQLite 不支持 `ALTER COLUMN`，表重建需复制 `sessions` 并重挂 PA-089 视图；
   而放宽为 `NULL` 会让既有 `row.get::<_, String>(...)` 在运行中撞 `InvalidColumnType`（真崩溃）。
   哨兵方案对老读者零崩溃面——`SessionState` 未声明该字段，serde 忽略未知键。
3. **无日志的存量会话继续走老路径写整包 blob**（grandfather 不破）。
4. **删除 2-B 的 `updated_at_ms` 过渡定序**：有日志会话**日志无条件为准**。

## 停写 blob 顺带暴露并闭合的真实写路径缺口

只把行写轻是**不够的**——日志当时并不承载图/元数据状态，直接切换会造成重启后光标回到旧分支、
trace 变空、重命名丢失。这不是测试口径问题，是**真数据丢失**。闭合项：

1. `ReplaceAll`（fork / switch / checkout / restore）→ 追加 `AppendTrace` / `UpdateHistoryNode` /
   `UpdateBranch` / `UpdateCursor` 命令流；成功即不写 blob，失败回退全量路径。
2. `rename_session` / `archive_session` / `unarchive_session` → 追加 `UpdateSessionMeta`。
3. 索引原样视图对轻量行用**索引自己的 `normalized_*` 投影**补全
   （`fill_index_only_rows_from_index_projection`，doc 写死「不读日志」不变量）；
   `rebuild_index` 的比对基准同样补全，`mismatched` 才有意义。
4. `build_context_observation` 外置载荷/引用（只落索引行）→ 从索引回填，否则重启丢。
5. 顶层 trace 的「当前分支视图」原先靠会增长的 `turn_trace_refs` ⇒ 改由日志 DAG
   （`parent_node_id` + `turn_id`）推导。
6. `backfill_orphan_sessions` 显式写整包 blob（契约 I4 要求，且它是迁移入口不是热路径）。

## Verification

- `[1] 轻量索引行：marked=true 1轮=763B 3轮=763B turn_count=3；对照组整包 blob=880B`
- `[2] blob 行 history=0，运行时读到 6 条消息 / turn_count=3 / title="问一"`
- `[3] 重启等价：`pa_2d_restart_logged` 消息数=4 / `pa_2d_restart_grandfather` 消息数=1`
- `[5] 存量会话 blob=3048B history=2（老路径未变）`
- `[6] dry-run EXIT=0 且副本 sha256 未变`
- `[7] 老库 NOT NULL 保留；老行=2359B，新行=768B（轻量）`
- 套件：`session_stop_blob_write` 7 / `session_read_from_log` 8 / `session_fold_rebuild` 8 /
  `session_log_wiring` 7 / `session_log_store` 6 / `--lib` 1393 / `storage_stage0_*` 11 全绿
- `cargo check` 0 warning；`check-declaration.sh` PASS

## 测试用例变更说明

按**该用例自带契约**移除 `session_fold_rebuild::legacy_blob_view_diff_is_limited_to_documented_gaps`
（其文案原文：「若这两类缺口已被补上…请同步删除本 allow-list 与本用例的说明」）——
缺口正是被本阶段关闭的。原位留注释指向新的守卫点 `tests/session_stop_blob_write.rs`，
其余 8 条用例断言一字未改。同时把 `fold.rs` 的一处单测
`read_path_merge_keeps_the_index_when_the_state_lane_is_newer` 反转为
`..._prefers_the_log_even_when_the_index_timestamp_is_newer`（它钉的正是本阶段要删除的过渡定序，
断言变强：索引时间戳更新时日志仍胜）。

## Alternatives considered

1. **放宽 `session_data` 为可空 + 写 `NULL`** —— 否决。SQLite 不能 `ALTER COLUMN`，需表重建并重挂
   PA-089 视图；且既有 `row.get::<_, String>` 会撞 `InvalidColumnType`，崩溃面真实存在。
2. **保留整包 blob、只删过渡定序** —— 否决。「日志无条件为准」只会让 blob 变成纯冗余写入，
   写放大与双副本分歧风险照旧，重构目的落空。
3. **顺带把 grandfather 存量会话也迁成轻量行** —— 否决。它们没有日志，一旦轻量化就**无真相可读**，
   必须先回填日志才能迁移，属独立的迁移事项。
4. **索引视图不补全、让相关用例判红再改** —— 否决。索引视图补全（仅用 `normalized_*`、不读日志）
   才是让 `rebuild_index` 的 `mismatched` 保持有意义的前提；判红改测试等于掩盖真缺口。
5. **先停写 blob 再补写路径缺口** —— 否决。顺序反过来会让中间态**真丢数据**（分支/trace/改名）。
   本阶段是「补缺口 + 停写」同批交付。
