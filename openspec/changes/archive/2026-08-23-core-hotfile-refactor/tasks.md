# Tasks: core-hotfile-refactor

> 并行边界：§1 与 §2 操作不相交文件集合，可并行；§2 内部 B1→B2 串行。

## §0 基线

- [x] 0.1 启动重构前全量基线测试 `npm run cargo:test`（job pwsh-2），记录红绿名单
  - 证据：exit 0 全绿；编译 6m05s；存量 warning 7 个（control_plane/mod.rs ×4、session.rs ×3、turn_flow.rs ×1 中 unused var）——无 deny(warnings)，warning 非门禁项
- [x] 0.2 确认工作树脏文件与本变更的交集：PA-096 前端/文档文件零交集；Cargo.lock 被基线构建良性同步（0.1.78→0.1.79，字节快照存 scratch，收口显式决策）；会话中途出现的 .gitignore/01_TASK_BOARD.md/vite.config.ts 属并行工作，不触碰

## §1 Line A：runtime/mod.rs 测试外移（@implementation-engineer-a）

> 切割脚本 `.tmp/p1p2-scratch/cut-runtime.ps1` 已 DryRun 验证（断言全绿：bare-LF、无 BOM、moved=9,523 行）。整块逐字搬移，严禁 dedent；仅对新 tests.rs 跑 rustfmt。

- [x] 1.1 执行切割（编排者代执行，见事故记录）：mod.rs → 7,064 行 LF；tests.rs 由 HEAD body 权威重建
- [x] 1.2 `rustfmt --edition 2021 runtime/tests.rs`：9,523 → 9,509 行；CR=0 保持
- [x] 1.3 Gate1 head 前 7,062 行与 HEAD 字节一致 ✓；Gate2 字面量多集（raw 2 / 普通 3,461）逐字节一致 ✓；warning 归因 7 条与基线逐条同位置同符号，零新增 ✓
- [x] 1.4 cargo check exit0 / --no-run exit0（测试代码编译通过=模块路径等价性获证）/ --list 路径前缀不变（868 测试）/ runtime::tests 全命名空间 **111/111 通过** + session attachment 冒烟 3/3 ✓

> **事故记录（Line A 执行竞态，已恢复）**：实现工程师子代理响应迟滞，中断前其异步收尾曾把 mod.rs 整文件重写为 CRLF 并自删半成品 tests.rs——`git diff` 因 autocrlf 归一化显示假性"干净"，唯字节级断言（行尾 CR=13）暴露真相。处置：编排者从 `git show HEAD:` 权威重建 head+body，统一还原该文件原生 LF 约定（仓库本身 LF/CRLF 混用，按"逐文件保持原约定"原则），全目录 CR 扫描确认无其他污染。教训已入简报：任何写回必须 CR 计数断言；autocrlf 环境下 git diff 不具备 EOL 审计能力。

## §2 Line B：session.rs → session/ 目录（@implementation-engineer-b）

> 开工条件：§1 全部勾选且全量基线刷新通过（不打中间 commit，遵守"未要求不提交"；A/B 隔离靠路径域回滚，见 design.md）
>
> ⚠️ **执行模式变更记录（2026-08-23）**：runtime 域经用户仲裁让渡 PA-098（Line A 双审核报告已转交其卡）。Line B 由编排者降级代执行切割本体（工程师子代理迟滞），随后工程师恢复响应并接管收尾——双方曾互认对方为"并行写入者"，已对齐。**交接快照见文末附录 A。**

- [ ] 2.0 全量基线刷新（post-Line A 全绿确认 ✓ pwsh-10 exit 0）+ warning 归因集合 ✓
- [x] 2.1 （B1）测试体外移完成：body=6,220–11,532 共 5,313 行，字节级与规格一致
- [x] 2.2 （B2）六文件已生成：types 643 / backend 322 / file_backend 77 / store 5,173 / tests 5,313 / mod 12 行；mod.rs 重导出由构造保证覆盖原 pub 面 + SessionMap 私有 use
- [ ] 2.3 可见性接线迭代：17 项跨模块私有项已翻转 pub(super)、tests 已注入定向 use、mod.rs 门控已修；**剩余 ~123 编译错误待消**（交接快照附录 A 含逐类处置提示）
- [ ] 2.4 校验链 a–h：--list 差分预言机 / --test-threads=1 / verify-literals 结构守恒 / warning 归因对比

## §3 对抗审核（编排器组织，实现完成后）

> Spec 审核记录：
> - 架构视角（d0439097）【有条件放行】——P0 spec 残留 dedent 指令与旧行数（已修）；P1 types.rs glob 例外取消 + SessionMap 私有 use、回滚补 git clean/checkpoint commit（已修）；P2 可见性上限规则、warning-diff 与结构守恒 gate 入 §4（已修）；P3 unique_test_session_dir 归 store（已修）。采纳率 6/6。
> - 回归风险视角（1d7a72e9）【有条件放行】——P1×5 全部采纳：①tasks dedent 措辞矛盾（实为禁令语境+旧版误读，已复核确认无危险指令残留）②B1 body-only 范围写死 6,220–11,532（防双层嵌套静默改路径，已入提示词）③9 处生产区 cfg(test) 归宿表 + 5,746–5,762 隔离块函数级保护（已入简报与提示词）④Cargo.lock 预脏字节快照协议（已执行快照）⑤编码/EOL 写端探测门（已入提示词）。处方采纳：--list 差分预言机、--test-threads=1、parity/serde_roundtrip/file_backend_roundtrip 裁样族、pub(crate) 冒充与 use super::* 遮蔽双封堵、探针垃圾清理（nul/.rcgu.o 已删）。采纳率 12/12。

- [x] 3.1 @code-reviewer-a：正确性/回归视角审 diff —— Line A 报告【有条件通过】已转交 PA-098；Line B 报告运行中
- [x] 3.2 @code-reviewer-b：边界/失败路径视角审 diff —— Line A【不通过：PA-098 版本包装缺陷】+ Line B【有条件通过：零 P0/P1，EOL/BOM 六文件纯 LF、cfg(test) 9/9、隔离屏障五段字节级一致、属性 167==167、非 test 构建无悬空名】
- [x] 3.3 汇总采纳/驳回并回改：Line B P2×3 文档错位/丢失已按 HEAD 权威原文修复（materialize 六行 doc 回正主+补第 6 行、extract_reasoning 补末行、default_snapshot 补整行）；P3-1 SessionMap 记录为设计偏差（定向方案下无需引入，避免 unused 警告）；P3-2 backend.rs 已过 rustfmt；修复后复验：cargo check 零错误 + 预言机 868↔868 差分空

## §4 测试门禁与验证（@test-engineer 口径）

- [x] 4.1 **统一终门禁（合流树）`npm run cargo:test` FULL_EXIT=0 全绿**（2026-08-23，本变更 session 六文件 + PA-098 runtime 结构化拆分收敛后联合通过）；warning 恰回基线 7 条
- [x] 4.2 合流树 `cargo check` 零错误（PA-098 施工中间态 172→11→0 收敛曲线实测）
- [x] 4.3 warning 集合归因对比基线：lib 7↔7、test profile 47↔47，仅 session.rs:1353-55 → session/store.rs 预期搬迁，零新增
- [x] 4.4 结构守恒脚本化校验：字面量多集 raw 1/1 + 普通 **1974/1974** 逐字节一致（GATE-PASS）；default_storage_path() 隔离屏障 476 字节级一致；868↔868 --list 差分预言机为空
- [x] 4.5 格式化口径：六文件 rustfmt 后 CR=0（工程师收尾执行）

## §5 收口（@orchestrator + @docs-release）

- [ ] 5.1 更新本 tasks.md 勾选与证据、PA-097 任务卡状态流转（In Progress → Review → Validation → Done）
- [ ] 5.2 openspec change 归档至 `openspec/changes/archive/<date>-core-hotfile-refactor/`
- [ ] 5.3 输出变更摘要（含基线对比、审核采纳记录、残余风险），不擅自 commit


## 附录 A：Line B 交接快照（2026-08-23 10:34，轮次预算耗尽时点）

**已完成**
- 六文件生成并通过结构双验（编排者生成 × 工程师只读勘察互证）：cfg(test) 九处全对号、default_storage_path 隔离块逐字幸存、body-only 无双层包装、零 use super::* 违规
- CRLF 污染（StringBuilder.AppendLine 引入）已由工程师统一归一化为纯 LF（10:33，六文件 CR=0）
- 编排者已修：地图缝隙 531–557（SessionTraceMutation 曾丢失）、多行 use 语句级解析（hooks 导入跨 9 行续名曾被丢）、Ordinal 字典防大小写覆盖（DEFAULT_SESSION_TITLE/default_session_title）、sqlite 路径 crate::agent 前缀、file_backend 孤儿 doc 迁移、mod.rs cfg(test) 门控

**进行中（工程师 c16532f3）→ ✅ 已完成（2026-08-23 收尾交付）**：编译迭代收敛、CRLF 归一化、rustfmt、验证链 a–h 全绿（明细见 §4 与其最终报告）；另拦截并修复移交清单 3 项未落地缺陷（mod.rs 门控、SessionMap 导入链、backend serde 属性丢失）+ 清除构建器遗留的 SessionStore 假 derive 行 + 解决 serde 派生宏模块作用域陷阱（default_session_title）。

**绿后序列**：rustfmt 六文件 → 验证链 a–h → Line B 双代码审核 → 全量门禁 → 归档收口

**资产索引**（.tmp/p1p2-scratch/）：cut-session-b1.ps1 / build-session-split.ps1 / flip-visibility.ps1 / patch-session-post.ps1 / verify-literals.ps1 / session-split-map.txt / ENGINEER-BRIEFING.md / LINE-B-PROMPT-DRAFT.md / consumer_surface.txt(38 名) / Cargo.lock.snapshot / brace-scope.ps1 / cut-runtime.ps1；工程师侧：before_list_sorted.txt(868) / before_check_full.txt(7 warning)

**在飞代理**：c16532f3（Line B 工程师，接管收尾中）；已停写声明：编排者对 session 域停写直至其最终报告。
