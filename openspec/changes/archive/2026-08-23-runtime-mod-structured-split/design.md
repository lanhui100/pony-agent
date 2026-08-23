# Design: runtime-mod-structured-split

> v2（2026-08-23）：按双路对抗审核结论修订。审核记录：PA-098 任务卡"Current Progress"。v1 中"胶水 glob 即可保可见性""外部仅引用 3 类型"两项表述已被证伪并更正。

## 前置事实（全部经脚本/只读命令实测；切割一律标记动态定位，禁止硬编码行号）

- `#[cfg(test)] mod tests {`（当前 7063–7064 行附近）词法延伸至文件尾；**其闭括号是文件尾缩进行 `    }`（当前 16588 行），列 0 的 16587 行闭的是嵌套 parity——列 0 定位对 tests 闭合无效，Apply 以 EOF 为界不受影响**。
- `mod parity`（约 15574 行）写在列 0 但嵌套于 tests 内，路径 `runtime::tests::parity`，继承 cfg(test)。其上方 4 行列 0 文档注释随迁。
- 全文件**唯一块注释：4942–5205（~264 行死代码）**，内含两个伪装成方法的 fn（`start_turn_stream_uses_compat_sync_for_deepseek_tool_followup` / `..._uses_live_stream_...`，后者带 `#[cfg(any())]`）。它们不是方法，方法定位必须排除该区间。
- **外部依赖面 = 7 项**（修正 v1 的"3 项"）：`control_plane/mod.rs:29-32` 导入 `AgentRuntime, AgentRuntimeBuilder, RunTurnFacts, TurnInput, TurnResult, TurnStreamEvent, SUSPENDED_TURN_PHASE`；`bin/non_tauri_harness.rs:10` 导入 AgentRuntimeBuilder；src-tauri 探针 bin 导入 AgentRuntime。
- 生产区 113 个顶层定义中仅 9 个 `pub`、7 个 `pub(crate)`，其余 ~97 个私有。
- 兄弟子模块活依赖（裸名/glob 经父模块解析）：ingress_mediation.rs 引用 `apply_capability_argument_patches`/`normalized_arguments_from_summary`/`apply_planner_patches` 及 outcome 结构体族；turn_runner.rs 以显式 `super::` 引用 patch 函数族。
- turn_runner.rs 自带同名 `HookDispatchOutcome`(31)/`CapabilityMediationDispatchOutcome`(93)/`PlannerDispatchOutcome`(187)——今日相安无事仅因既有 6 子模块无 glob 胶水。
- mod.rs:74-75 `#[cfg(test)] use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};` 唯一消费者是三个 cfg(test) 注册表函数（6980/7007/7034 起）。
- 5560 行 `#[cfg_attr(not(test), allow(dead_code))]` 属于 build_stream_progress_trace_timeline，随迁（字节搬运天然覆盖）。
- 文件为纯 CRLF（16,588 CRLF / 0 LF）、无 BOM、尾部有换行；全文件 raw string 仅 2 处且均为单行（11018/11060）→ **整块 dedent 一级字节安全**（此为对 PA-097 design"严禁 dedent"结论的有据修订，依据即本条实测；两份 spec 的偏差在此声明）。

> 行号会漂移：所有切割按正则标记 + 内容守恒断言定位；本清单行号仅为审计坐标。

## §1 测试外移（含嵌套 parity）

1. 脚本定位 `#[cfg(test)]\nmod tests {`，断言其后再无任何顶层内容（EOF 为界）；整块（含 parity 及文档注释）dedent 一级写入 `runtime/tests.rs`；
2. mod.rs 头部至 `#[cfg(test)]` 前**逐字节保留**（不做 TrimEnd，含末尾空行），追加 `#[cfg(test)]\nmod tests;\n`；
3. 路径语义：`runtime::tests` 与 `runtime::tests::parity` 不变，`use super::*` 零变化；
4. 断言（三条硬性 throw + 写后复核）：切出行数 == 删去行数；头部前缀与原文逐字节一致；tests.rs 重缩进还原后与原块零差异；写盘后重读两文件做字节级重建比对；
5. 校验：`cargo check -p pony-agent-core` 通过；非测试区域 `git diff` 为空（除尾部声明段）；抽样运行 runtime tests 与 parity 过滤子集。

## §2 自由函数/类型子模块化

**单一事实源 = `tmp\pa098-targets.txt` 名字映射表**（按名字动态解析行号，歧义名报错）。design 表不再复制明细；门禁：实施前断言生产区 113 个定义逐一有归属（迁移或留守），不允许"等头部小助手"式模糊账。

关键分配（详见 manifest）：类型/常量/outcome 结构体族/`model_hop_trace_contents*`/`CANCELLED_TURN_MESSAGE`/`HOOK_FAILTURN_HANDLED_SENTINEL`/`SUSPENDED_TURN_PHASE` → types.rs；builder 族 → builder.rs；上限常量族+注册表+parse/build_error+`runtime_now_ms` → limits.rs（**74-75 行 cfg(test) use 随迁 limits.rs**）；patch 函数族（含 3 个 pub(crate) 活依赖）→ planner_patches.rs；blocked/skill 记录族 → blocked_records.rs（**4942–5205 块注释随 build_skill_tool_result 区间一并迁入，原样保留**）；images/timeline/transcript 族 → trace_timeline.rs；recovery 族 → tool_recovery.rs；流式支撑/去重/止损/cache/token 合并/指令规范化 → stream_support.rs。

机制（修正 v1）：迁移项默认可见性调整——**原私有项一律加 `pub(super)`（预期 ~97 处，批量登记不逐条例外）**；等价判据：private-in-parent ≡ pub(super)-in-child（均在 runtime 子树内可见，含 tests）。原 pub/pub(crate) 保持原关键字。mod.rs 加私有 `mod x;` + `pub(crate) use x::{外部7项中的对应者}` + 私有 `use x::{其余需在 runtime 命名空间解析的名字}`（服务兄弟子模块与 tests 的 glob 解析链；父模块私有 use 绑定对全部子孙可见，仓内先例 turn_prep.rs:3）。

新子模块统一以 `use super::*;` 开头。**禁止对既有 6 子模块建立任何 glob use**（E0659 歧义防线）。

## §3 impl AgentRuntime 分块迁移

- 留守 mod.rs：结构体本体、构造器（new/with_dependencies）、访问器、governed context、trace 管道四方法。
- `turn_stream.rs`：handle_stream_tool_turn、start_turn_stream 三连（**预登记残余热点 ~2.7k 行，如实写入收口摘要**）。
- `turn_sync.rs`：handle_sync_tool_turn、run_turn/run_turn_with_facts/run_turn_inner、sync_turn_event_id、emit_sync_turn_failed。
- `turn_control.rs`：should_cancel_turn、cancel_stream_turn、suspend_turn_for_control_outcome、build_suspended_turn_result、bind_ask_wait_for_request。
- **预登记可见性（非编译器例外）**：turn_control 前四方法的 13 个调用点实测全部位于未来 turn_stream/turn_sync 内 → 迁入即标 `pub(super)`；bind_ask_wait_for_request 唯一调用点在同文件 2399 → 保持私有。其余方法由编译器循环兜底。
- 方法边界定位必须排除 4942–5205 块注释区间（其中两个伪 fn 不是方法）。
- 方法可见性关键字逐字保留，仅按上述预登记/编译器兜底最小放宽并登记。

## 实施方式约束（防错核心）

1. 一律脚本切割；LLM 只定边界/写胶水/跑校验；标记匹配须具注释感知（至少显式豁免已知块注释区间）；
2. 每 § 结束独立可编译可测；`git status` 断言改动仅限 runtime 目录；工作树须无 staged 内容方可开始；
3. 编译器驱动修复循环为兜底；每轮修复登记清单（名字 → 处置）；
4. 脚本硬性要求：CWD=仓库根断言、EOL/BOM 前后一致断言、输出路径从输入路径派生（禁止硬编码仓库路径）、写盘后重读重建比对。

## 替代方案与否决理由

1. 方法内部再拆小（否决，另立任务）；2. `include!` 拼接（反模式）；3. 测试按主题再分目录（后续低风险任务）；4. AgentRuntime 结构体迁出 mod.rs（否决，门面保核心类型）；5. 对块注释做删除/清理（否决，超零行为变更范围，原样随迁）。

## 回滚

前置检查：`git diff --cached --quiet` 必须通过（无 staged）。执行：`git restore --source=HEAD --staged --worktree -- crates/pony-agent-core/src/agent/runtime/` + `git clean -f crates/pony-agent-core/src/agent/runtime/`。

**失效条件（reviewer-b P1-1，2026-08-23 增注）**：以上命令仅在"runtime 目录为唯一改动域"时保证安全。当前工作树同时承载 PA-097 Line B 对 `agent/session*` 的并发改动（HEAD 版 mod.rs 与 session/ 新布局的兼容性未经验证），单独回滚本任务不构成安全退出路径；如需回滚必须与 session 任务编排者协调（分层提交隔离或双任务协同回滚）。

## 验证策略

1. 基线先行（已完成 exit 0 全绿）；
2. 结构守恒双口径：**§1 = 内容守恒（模一次 dedent）+ 行数守恒 + CRLF/BOM 不变；§2/§3 = 函数体逐字节平移**；
3. 终门禁：`npm run cargo:test` 对比基线零新增失败；`npm run cargo:check` 干净；`cargo check -p pony-agent-core` **warning 集合对比基线（7 个存量）零新增**（按文件+符号归因）；`cargo doc -p pony-agent-core --no-deps` 通过；rustfmt 默认口径核验；mod.rs <500 行核验（不计块注释则更优）；
4. `--all-features` 不适用（Cargo.toml 无 [features] 段，已核实）。
