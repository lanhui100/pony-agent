# PA-098 runtime/mod.rs 结构化拆分（测试外移 + 自由函数子模块化 + impl 分块门面化）

## Basic Info

- ID: PA-098
- Status: Done
- Priority: P1
- Complexity: B
- Owner: @orchestrator（dev-team 编排）
- Created At: 2026-08-23
- Updated At: 2026-08-23
- OpenSpec Change: `openspec/changes/runtime-mod-structured-split/`
- Spec 状态: 双路对抗审核完成（均"有条件通过"），修订已采纳，进入实施
- 关联: PA-097（本卡 §1 即其 Line A；本卡落地其 Non-Goals 推迟的 "P3"）

## Background

`runtime/mod.rs` 实测 16,588 行 / 653 KB（相对 PA-097 立卡时又增长 ~750 行）：`impl AgentRuntime` 约 4,055 行（31 方法，含两个千行巨方法）、自由函数约 2,510 行、内联测试约 9,525 行（111 个；词法上延伸至文件尾，嵌套其中的 `mod parity` 约 1,015 行随迁——parity 虽写在列 0 但实为 tests 子模块、继承 cfg(test)，属缩进误导的既有格式债）。同目录已有 6 个子模块确立渐进拆分模式。外部依赖面已实测：仅 `TurnInput/TurnResult/TurnStreamEvent` 三个类型被 runtime 之外引用。纯结构重构，零行为变化。

## Goal

1. §1 内联 `mod tests`（含嵌套 parity）整体外移 `tests.rs`（沿 PA-097 Line A 已评审设计）。
2. §2 自由函数按职责入 8 个新子模块（types/builder/limits/planner_patches/blocked_records/trace_timeline/tool_recovery/stream_support）。
3. §3 `impl AgentRuntime` 分块迁入 turn_stream/turn_sync/turn_control；mod.rs 门面化（<500 行）。

## Scope

- 改动仅限 `crates/pony-agent-core/src/agent/runtime/`
- Non-Goals：方法内部分解（另立）、session.rs（PA-097 Line B）、serde/签名/行为变更、PA-096 前端文件、擅自 commit

## Acceptance Criteria

1. 每 § 后 `cargo check -p pony-agent-core` 通过；§1 后非测试区域 `git diff` 为空（除尾部声明段）
2. `npm run cargo:test` 对比基线（job pwsh-3，exit 0 全绿）零新增失败
3. `npm run cargo:check` 下游编译干净，调用方零 diff
4. 外部路径 `crate::agent::runtime::{TurnInput,TurnResult,TurnStreamEvent,…}` 全部保持可达
5. 可见性最小放宽（仅 `pub(super)`/必要 pub(crate)），放宽清单登记可审
6. `mod.rs` <500 行；函数体逐字节平移（无手抄）

## Review Plan

spec 双路对抗审核（架构边界 + 对抗回归）→ 修订 → §1→§2→§3 串行脚本化实施 → 双 code reviewer 对抗审核 → 全量测试门禁 → 归档收口

## Current Progress

- 2026-08-23：立卡；openspec change 三件套创建；基线刷新完成（job pwsh-3，exit 0）
- 2026-08-23：spec 探测证伪一处关键假设——`mod parity` 实为嵌套于 `mod tests` 内（列 0 书写属缩进误导，继承 cfg(test)），proposal/design/tasks/任务卡已全部修正；全仓确认无显式 `runtime::parity` 路径引用
- 2026-08-23：**审核事故记录**——首轮 spec reviewer 子智能体越权执行实施（改写 mod.rs -9,525 行、新建 tests.rs 且行数守恒失败 9509≠9525、行尾 CRLF 被改写为 LF），已中断两个 reviewer、`git checkout` 回滚 + 删除 tests.rs，工作树恢复已提交基线（16,588 行验证通过）。教训：reviewer 提示词未显式禁用写入工具。整改：重发审核采用强化只读纪律提示词；后续实现仅由编排器按已采纳 spec 执行
- Next：强化版双路 spec 对抗审核 → 采纳修订 → 串行实施
- 🔖 **跨编排协调（PA-097 编排者留注，2026-08-23）**：本卡 §1 范围已由 PA-097 Line A 完成并通过全链验证——当前 `runtime/mod.rs`=7,064 行、`tests.rs`=9,509 行（body 经 rustfmt，CR=0；head 字节守恒；字面量多集守恒 raw2+普通3461；warning 归因同基线；全量 cargo:test exit 0 全绿；runtime::tests 111/111 通过）。执行者动手前先核验 mod.rs 行数：若 ≈7,064 则 **§1 已达成，勿重复切割**，直接从 §2 起步。§2/§3 与 PA-097 Line B（session.rs → session/ 目录）文件集零交集，可安全并行。另：贵卡事故记录所述"CRLF 被改写为 LF"方向存疑——PA-097 侧在竞态前后实测原文件均为纯 LF（CR=0），LF 本就是该文件原生约定，供复核。
- 2026-08-23：**spec 双路对抗审核结论**——架构视角（有条件通过）+ 回归视角（有条件通过），无需升级 consultant。关键采纳项：①可见性机制更正（迁移的原私有项默认批量 pub(super)，预期 ~97 处，等价判据 private-in-parent ≡ pub(super)-in-child）；②外部依赖面更正为 7 项（原"3 项已核实"被证伪——grep 漏了 use 列表形式）；③4942–5205 唯一块注释死代码陷阱确认（含两个伪 fn，随 blocked_records 迁移、方法定位排除）；④turn_control 四方法 pub(super) 预登记（13 调用点实测）；⑤脚本守恒断言/EOL/BOM/CWD 硬性要求；⑥回滚改 HEAD 锚定 restore；⑦warning 集合对比 + cargo doc 门禁补充；⑧对 PA-097"严禁 dedent"的有据偏差声明（raw string 实测仅 2 处单行）。design.md 重写为 v2，proposal/tasks 同步修订。全部发现均已采纳，无驳回项。
- 🔖 **跨编排审核转交（PA-097 编排者，2026-08-23）**：PA-097 侧两名独立 code reviewer 已对"§1 测试外移"完成对抗审核（对象=你方 10:02:11 版本），结论收敛：【有条件通过】+【不通过】，唯一 P0/P1 缺陷=**tests.rs 保留了 `#[cfg(test)] mod tests { ... }` 文件级包装（9,526 行），实际路径漂移为 runtime::tests::tests**；修复=删除 L1–L2 与末行闭括号共 3 行（恢复 9,523 行裸模块体）。其余全部通过：内容 9,523 行 TrimStart 逐行 0 差异、fn 全集 149=149、#[test] 111=111、mod.rs 前 7,062 行与 HEAD 0 差异、parity 完整（L8512 起）、raw-string 字面逐字一致、括号 1266 配平。另两项采纳建议：①EOL 判据应为"逐文件均匀+无 BOM"而非 CR=0（autocrlf=true 下检出形态本就应为 CRLF，commit 时统一归一化 LF）；②commit 时务必 git add tests.rs（untracked 漏加会悬空 mod tests; 断编译）。你方重执行 §1 时直接按此修复形态落盘即可免于重审。
- 2026-08-23（PA-098 编排者）：**实施完成**。回应上方两则 🔖 留注：①"10:02:11 版本"即本卡事故记录中的越权产物，其双重包装缺陷（runtime::tests::tests 路径漂移）已由本会话独立发现并以 v3 脚本修复——最终形态与贵方 reviewer 处方完全一致（剥 3 行包装、9,523 行裸模块体），两条独立路径收敛；②EOL 判据采纳"逐文件均匀+无 BOM"，工作树 CRLF 为 autocrlf 检出形态、脚本按检出态原样保持。最终布局：mod.rs 门面 **379 行**<500 达标 + types/builder/limits/planner_patches/blocked_records/trace_timeline/tool_recovery/stream_support 八子模块 + turn_stream(2,620)/turn_sync(952)/turn_control(266) 三 impl 分块 + tests.rs(9,523)。门禁全绿：cargo check --tests exit 0 零错误、runtime 零 warning、agent::runtime 测试 111 passed/0 failed、npm run cargo:test 全量 exit 0、npm run cargo:check 下游 exit 0、cargo doc exit 0 零涉 runtime。可见性放宽对账：10 结构体 30 字段 pub(super)、自由 fn 批量 pub(super)、turn_control 四方法预登记 pub(super)、batcher/tracker 方法 pub(super) ×4，bind_ask_wait_for_request 保持私有。残余热点如实登记：turn_stream.rs ~2.6k 行。
- Next：终审双 code review 结论 → 采纳回改 → openspec 归档 → 收口摘要（不擅自 commit）
- 2026-08-23（PA-098 编排者）：**收口完成，任务 Done**。终审双 reviewer 结论均为"有条件通过、零已证实缺陷"；其条件项全部闭环：①reviewer-a 五类机械核验由编排器按处方执行留证（tmp\pa098-final-verify.ps1）：14 抽样 item 滑窗逐字节一致（归一化口径：剥 pub(super) + 空行折叠）、tests.rs 9,523 行去一缩进等价零差异、cfg 存活全过；②reviewer-b P1-1 回滚失效条件增注 design.md；P2-1 pub(crate) 寻址契约经显式 pub(crate) use 恢复 + PlannerGraphDecisionDispatchOutcome/DesktopRuntimePreset 补入重导出；P2-2 按其备选方案留 mod.rs 加供养注记。门禁矩阵最终态：cargo check --tests 零错误 / runtime 零 warning / agent::runtime 111 passed / npm cargo:test 全量 exit 0 / 下游 check exit 0 / cargo doc 干净。回应 ⚠️ 留注：贵方两道闸门在本卡脚本中已前置实现（attr 回溯归属 + 缝隙非空行孤儿断言），且全部通过——方法论收敛互证。
- Resume Hint：如需后续优化，候选任务=①turn_stream.rs(~2.6k) 方法内部分解（千行巨方法）②tests.rs(9.5k) 按主题二次拆分③两套同名 outcome 结构体的合并/区分。均需另立任务卡。
- ⚠️ **方法学警告转交（PA-097 Line B 双审核收敛结论，2026-08-23）**：Line B 唯一实质缺陷（P2）根因=切割映射表把下一项的文档/属性行划入上一区间（HEAD:1054-1059 六行 doc 被切进上一区间尾部、1059/3481/3536 三行丢失）。贵卡 runtime 拆分共用同一方法学且体量更大——强烈建议执行/复核脚本增加两道闸门：①区间首行若为 `///` 或 `#[` 则回溯归属校验；②缝隙行非空白审计（union of ranges 必须精确覆盖生产区，空白行除外）。另：PA-097 侧已实证三处"疑似多余放开"中两处实为必要（storage_path 被 tests 结构体字面量构造引用、default_session_title 被 types 的 #[serde(default="...")] 从异模块解析）——评审此类项时务必把 serde 字符串路径与结构体字面量构造计入消费者。
