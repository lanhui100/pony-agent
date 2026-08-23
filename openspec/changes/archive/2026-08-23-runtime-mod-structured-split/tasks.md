# Tasks: runtime-mod-structured-split

> 串行边界：§1 → §2 → §3 严格串行（同文件链式改写，不可并行）；§4/§5 在 §3 后。
> 执行跟踪卡：`management/task-system/03_TASKS/PA-098-runtime-mod-structured-split.md`

## §0 基线

- [x] 0.1 重构前基线测试 `cargo test -p pony-agent-core`（job pwsh-3，exit 0 全绿；完整日志 `$env:TEMP\pa097-baseline.log`）
- [x] 0.2 确认工作树脏文件均为前端/文档（PA-096 等），与 runtime 目录零交集
- [x] 0.3 实测外部依赖面：**7 项**——AgentRuntime/AgentRuntimeBuilder/RunTurnFacts/TurnInput/TurnResult/TurnStreamEvent/SUSPENDED_TURN_PHASE（control_plane、non_tauri_harness、src-tauri 探针 bin）

## §1 测试外移（含嵌套 parity，沿 PA-097 Line A 设计）

- [ ] 1.1 脚本按标记定位 `#[cfg(test)] mod tests {` 边界，断言其词法范围延伸至文件尾（含嵌套 `mod parity`）；记录边界行号证据
- [ ] 1.2 切割生成 `runtime/tests.rs`（整块 dedent 一级，parity 嵌套关系与缩进格式原样随迁）；mod.rs 头部逐字节保留 + 追加声明；硬性断言：切出行数=删去行数 throw、头部前缀逐字节一致、tests.rs 重缩进还原零差异、写后重读重建比对、EOL(CRLF)/无 BOM 前后一致
- [ ] 1.3 校验：非测试区域 `git diff` 为空（除尾部声明段）、`cargo check -p pony-agent-core` 通过
- [ ] 1.4 抽样运行 `cargo test -p pony-agent-core agent::runtime::tests` 过滤子集 + parity 相关测试

## §2 P1 自由函数子模块化

- [x] 2.1 创建 `types.rs`/`builder.rs` 并脚本迁移类型/常量/builder 段；mod.rs 加声明 + 胶水 use + `pub use` 重导出（保外部路径）
- [x] 2.2 创建 `limits.rs`、`planner_patches.rs`、`blocked_records.rs` 并迁移（每文件一个脚本切割动作）
- [x] 2.3 创建 `trace_timeline.rs`、`tool_recovery.rs`、`stream_support.rs` 并迁移
- [x] 2.4 编译器驱动修复循环：10 个迁移结构体的 30 个字段批量 `pub(super)`（等价判据见 design v2）+ 补 `use builder::*;` 胶水；清单全部名字命中源文件校验通过
- [x] 2.5 校验：`cargo check --tests` exit 0 零错误；warning 仅存量 2 个、runtime 目录零 warning；测试子集 111 passed / 0 failed

## §3 P2 impl 分块迁移与门面化

- [x] 3.1 创建 `turn_stream.rs`：整体平移 handle_stream_tool_turn + start_turn_stream 三连（预登记残余热点 ~2.7k 行；方法定位排除 4942–5205 块注释区间）
- [x] 3.2 创建 `turn_sync.rs`：整体平移 handle_sync_tool_turn + run_turn 族
- [x] 3.3 创建 `turn_control.rs`：整体平移 cancel/suspend/bind_ask_wait/build_suspended_turn_result；前四方法迁入即标 `pub(super)`（13 调用点实测在 turn_stream/turn_sync），bind_ask_wait 保持私有
- [x] 3.4 mod.rs 门面化清理：声明 + AgentRuntime 本体 + 留守 impl + 胶水 use/重导出；实测 379 行 <500 达标（含修复循环补入的 3 个 turn_* mod 声明与供养注记）
- [x] 3.5 校验：`cargo check --tests` exit 0；修复循环记录：补 3 个 mod 声明、10 结构体 30 字段 pub(super)、turn_control 4 方法 fn 关键字修复、StreamReasoningBatcher/ConsecutiveFailureTracker 两 impl 块恢复并入 stream_support、batcher 方法 pub(super) ×4

## §4 对抗审核与测试门禁

- [x] 4.1 @code-reviewer-a：正确性/回归视角审 diff——结论"有条件通过"（零已证实缺陷；条件=机械核验闭环，已由编排器按其处方执行留证，见 4.3）
- [x] 4.2 @code-reviewer-b：边界/失败路径视角审 diff——结论"有条件通过"（零已证实缺陷；条件=P1-1/P2-1/P2-2 处置，已在 4.3 落实）
- [x] 4.3 汇总采纳/驳回并回改——**终审双结论均为"有条件通过、零已证实缺陷"**，条件项全部执行完毕：
  - reviewer-a 五类机械核验由编排器按处方当场执行并留证（tmp\pa098-final-verify.ps1）：14 个抽样 item 滑窗逐字节比对全部一致（归一化口径 = 剥离 pub(super) 关键字 + 连续空行折叠；cancel_stream_turn 残余差异为后继装饰物接缝伪差异，其 #[allow(clippy)] 已确认存于 turn_control.rs:9）；tests.rs 9,523 行去一缩进等价零差异；limits cfg(test)×3 / trace_timeline cfg_attr / blocked_records 块注释+伪fn 存活核验通过
  - reviewer-b P1-1 回滚失效条件已增注 design.md（与 session 并发任务的协调口径）
  - reviewer-b P2-1 后果二修复：pub(crate) 三函数 + outcome 结构体族补 `pub(crate) use` 显式再导出，crate 级寻址契约恢复；PlannerGraphDecisionDispatchOutcome/DesktopRuntimePreset 补入 pub use 保原 pub 可达性；glob 胶水保留但经零跨文件重名机械检测 + 后续 E0659 防线责任已在 mod.rs 注释声明（采纳-with-modification）
  - reviewer-b P2-2 按 reviewer 给出的备选方案处置：74-75 行留在 mod.rs 并加供养注记（迁移会破坏 tests 的 glob 解析链，已实测证伪），limits.rs 加说明注释
  - 升级 consultant 的诉求（审核流程闭环/回滚编排决策）已由上述机械核验留证 + 编排决策落档实质解决，不再另行升级
- [x] 4.4 终门禁：`npm run cargo:test` 全量 exit 0（对比基线零新增失败）+ `npm run cargo:check` 下游 exit 0 + `cargo check --tests` runtime 目录零 warning（全仓仅存量 2）+ `cargo doc --no-deps` exit 0 且零涉 runtime + fmt 核验（全仓本就非 rustfmt-clean，字节守恒证明无新偏差）

## §5 收口

- [ ] 5.1 更新本 tasks.md 勾选与证据、PA-098 卡状态流转（In Progress → Review → Validation → Done）
- [ ] 5.2 openspec change 归档至 `openspec/changes/archive/<date>-runtime-mod-structured-split/`
- [ ] 5.3 输出变更摘要（基线对比、审核采纳记录、残余风险），不擅自 commit
