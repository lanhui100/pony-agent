# PA-097 pony-agent-core 热点文件重构（P1 测试外移 + P2 session 目录化）

## Basic Info

- ID: PA-097
- Status: Done（2026-08-23 终门禁全绿 + Line B 双审核采纳归档）
- Priority: P1
- Complexity: B
- Owner: @orchestrator（dev-team 编排）
- Created At: 2026-08-23
- Updated At: 2026-08-23
- OpenSpec Change: `openspec/changes/core-hotfile-refactor/`
- Spec 状态: 已归档（`openspec/changes/archive/2026-08-23-core-hotfile-refactor/`）

## Final Gate Evidence

- **统一终门禁（合流树）**：`npm run cargo:test` FULL_EXIT=0 全绿；warning lib 7↔7 / test 47↔47 零新增；session 域 83/83（含 --test-threads=1 轮）；868↔868 --list 差分预言机为空；字面量守恒 raw 1/1 + 普通 1974/1974；隔离屏障 476 字节一致

## Closeout Record（2026-08-23）

- **Line A**：交割 PA-098（用户仲裁）。双审核独立证实内容机械等价后其域回滚重执行；包装剥离方案已转交。
- **Line B 最终形态**：session/{mod,types,backend,file_backend,store,tests} 六文件（mod.rs 55 行纯声明+重导出+cfg(test) 供给区）；原 session.rs 删除。可见性 26 处最小 pub(super) 经审核者逐项溯源验证（两处"疑似多余"经证据驳回：storage_path 被 tests 结构体字面量构造引用、default_session_title 被 types 的 serde 属性跨模块解析）；MemorySessionBackend 死重导出已删（P3-3 采纳）；文档三处按 HEAD 权威原文回正（P2×3 采纳）；SessionMap 私有 use 记录为设计偏差（零消费者不引入，P3-1 部分采纳）。
- **审核总账**：spec 2 路 18 条全采纳；Line A 双审 2 报告转交 PA-098；Line B 双审 P2×3 修复、P3 采纳 2/驳回 2（均附证据）/转交 1。

## Background

`runtime/mod.rs`（15,840 行，近 60 天 26 次提交）与 `session.rs`（10,916 行，23 次）是全仓库最热且最大的两个文件，内联测试占一半以上体积；会话域模型、持久化抽象、存储门面混居一文件。作为 AI agent 高频迭代的仓库，超长热点文件持续放大编辑定位成本、diff 噪音与合并冲突风险。纯结构性重构，零行为变化。

## Goal

1. P1：`runtime/mod.rs` 与 `session.rs` 的内联 `mod tests` 外移为独立测试文件（各瘦身约一半）。
2. P2：`session.rs` 转为 `session/` 目录模块（mod/types/backend/file_backend/store/tests），经重导出保持 `crate::agent::session::*` 全部既有路径不变，调用方零改动。

## Scope

> 🔖 **范围修订（2026-08-23，用户仲裁）**：runtime 域让渡给并行会话 PA-098（其独立完成的测试外移已通过本卡全部内容门禁：字面量多集守恒、结构断言、全量测试绿；仅行尾为 CRLF，commit 时被 autocrlf 归一化故入库无差别）。**本卡剩余交付物 = Line B（session.rs → session/ 目录化）+ 终门禁 + 归档。**

- 改动仅限：`crates/pony-agent-core/src/agent/session.rs`（转为目录）
- Non-Goals：runtime/* 全部文件（归 PA-098）；不改签名/行为/serde；不动 PA-096 未提交前端改动；不擅自 commit

## Acceptance Criteria

1. `cargo check -p pony-agent-core` 与下游 `cargo:check` 通过，调用方零 diff
2. `npm run cargo:test` 相对重构前基线零新增失败
3. P1 非测试区域字节级不变；session 拆分各子模块与原文件逐段对应
4. 可见性不放宽越界：原 pub/pub(crate) 等效保持，内部放开仅限 pub(super)/必要 pub(crate)
5. `runtime/mod.rs` ≈ 7,100 行以内；`session/mod.rs` 仅声明 + 重导出

## Review Plan

spec 双路对抗审核（架构边界 + 回归风险）→ 修订 → 双 engineer 并行实现（Line A ∥ Line B）→ 双 code reviewer 对抗审核 → 全量测试门禁 → 归档收口

## Current Progress

> ⚠️ 并发协调注记：检测到另一会话在本卡创建后追加了进度记录并将 runtime 拆分立为 PA-098；其间发生过一次双编排竞态（PA-098 卡"审核事故"即此）。当前 crate 文件状态以本卡 tasks.md 的字节级验证为准。

- 2026-08-23：立卡；openspec change 三件套创建；基线全量测试全绿（exit 0）
- 2026-08-23：spec 双路对抗审核完成，均【有条件放行】，18 条意见全部采纳（架构 6/6 + 回归 12/12，明细见 openspec change tasks.md §3 审核记录）
- 2026-08-23：**Line A 完成并验证**（编排者代执行——工程师子代理响应迟滞触发披露的降级模式；期间与 PA-098 会话发生文件竞态，已从 `git show HEAD:` 权威重建）：runtime/mod.rs 16,588→7,064 行（纯 LF），tests.rs 9,509 行（body 经 rustfmt，CR=0）；Gate1 head 字节守恒 ✓ / Gate2 字面量多集守恒（raw 2 + 普通 3,461）✓ / warning 归因与基线逐条相同零新增 ✓ / 测试编译通过 / runtime::tests 全命名空间 111/111 通过
- 进行中：Line B 双代码只读审核（正确性 + 边界/属性两路）
- **2026-08-23：Line B 实现完成**（工程师 c16532f3 接管收尾交付）：session/{mod,types,backend,file_backend,store,tests} 六文件落地（rustfmt 后 CR=0）；验证链 a–h 全绿——868↔868 测试清单差分空、session 83/83 双线程模式通过、字面量 1974/1974 守恒、warning 归因 7↔7/47↔47 零新增、default_storage_path() 隔离屏障 476 字节级一致；可见性 27 处最小 pub(super) 零违规。全树 check 当前红系 PA-098 runtime 施工中间态（172 错误全在其域），收敛后统一终门禁
- Status: In Progress → Review（Line B 段）
