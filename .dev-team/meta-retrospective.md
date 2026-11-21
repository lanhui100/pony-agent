# 元复盘 · B264 波次 `tui-resume-archive`（TUI /resume 列表重排 + 会话归档）

> 范围：2026-11-21 单会话交付（B 级，k=1 共享工作区串行，无 worktree）。
> 审查收据：`.dev-team/review-plan-tui-resume-archive.json`（方案逆向对抗，FAIL→R1~R10 裁决）、
> `.dev-team/review-l3-tui-resume-archive.json`（L3 代码对抗）、`.dev-team/review-l3p-tui-resume-archive.json`（L3-P 六问）。
> 上手经验见 `history` 段：与 `wave-acp-delegate` 同一套拓扑，但本轮名册峰值 4、审查 3 路，墙钟大幅下降。

## 通信拓扑与信噪比

- 名册 4 人（Lead + protocol + test + executor），审查全部走 Subagent 计算池，未挤占槽位；无持久参谋。
- 三次真实阻塞都不是沟通问题，而是**共享构建锁**：test lane 的 `tui_dialog_wave39` / `tui_render_layout`
  单二进制各跑 340~400s（`ask_lifecycle_settle_tests` 内 sqlite 用例 >60s/条），把 protocol 的 `cargo check`
  压在队列里 55 分钟。两名 Agent 主动上报等待证据并要求裁决（而不是静默重试或绕锁裸跑），是本轮最有效的降噪行为。
- 冗余观察：既有回归基线在**改动前**跑了一遍（6 target / exit 0），Executor 又跑了一遍（exit 0）。
  这两遍在「同一分支、同一 target 目录」下语义重叠，时间成本 ≈25 分钟。下个波次应显式约定：
  基线只在基线 commit 上跑一次，绿相验证复用同一份收据 + 只增量跑本次触及的用例集。

## 门禁穿透与误杀率

- **真正拦住缺陷的是 Plan 期的 L3 方案审查（6 BLOCKING）**：宽度绑定错面、`list_row` 二次截断吃 `…`、
  视图索引与原始索引混用、`/archive` 漏 Ask 收口、`d` 对当前行的未定义语义。这些若留到实现后才发现，
  返工面是整个渲染分支。**结论：B 级任务把契约冻结后先审设计，ROI 明显高于事后审 diff。**
- **两路 Review 各自独立命中同一致命项**（HEAD 过不了 `rust:lint` 的 bool 入参棘轮），说明棘轮是
  真实防线；但 lint 门在实现提交后才跑，导致「实现绿 → 门红 → 再改一轮」。改进：把 `npm run rust:lint`
  前移到 Executor 完成回执前（已在本轮回执模板里体现）。
- 误杀/噪声：`dev-review-lint` 首轮把我的单条 `--scopes "a,b,c"` 当成一个范围判未闭合（工具期望多参数），
  以及 v1 收据缺溯源字段被判无效——都是**接口约定问题**，非审查质量；已在流程中固定为「先包 schema v2 再 lint」。
- 假绿风险（已记录、未修）：冻结测试 T4 只用 `open_session_selector(Vec::new())` 断言 0/0；
  `format_push_string`/`unnested_or_patterns` 类问题靠棘轮而非测试发现。测试分权挡住了「Executor 自证」，
  但挡不住「测试自己覆盖不全」——下一轮 test-agent prompt 要补验收模板（要求含「全可见/部分隐藏/仅归档」三态）。

## 分工契约与隔离有效性

- 写域切分有效：protocol 只落契约 + `todo!()` 空桩，test 只写 tests，executor 只写 src；
  唯一越界是 executor 在修复轮新增 `src/bin/tui/tests/resume_row_layout_tests.rs`（A4 可选单测），
  已按「衍生写域」登记而非打回——**教训**：给 Executor 的「可选」项必须同时给写域，否则它只能越界。
- 红相锚定（`2ab07d00` protocol → `d0bef489` test freeze → `381802f3` impl）让「测试只读」从口头变成
  commit 结构：Executor 想改断言必须先改历史。这一条在本轮零争议。
- 契约演进门禁生效：Plan 审查后由 Lead 原地改写 C1/C3/C5/C6 + 追加 R1~R10 流水，再交 Executor；
  Executor 报 4 条偏差（width 预留、日志字段避撞、阻塞回执通道、失败不轮换），全部可追溯。

## 元协议迭代建议

1. **把 lint 棘轮加入 Executor 完成定义**（现在只在 Join Barrier 跑）：`check --bin` 绿 ≠ 交付合格。
2. **测试分权的补位**：test-agent 的 prompt 增加「必须覆盖可见面三态（全可见/部分隐藏/仅归档）与接线断言
   （源序护栏 `include_str!`，参考 `tests/tui_ctrl_o_global_shortcut.rs`）」，否则纯函数测试会给出假绿。
3. **共享构建锁的调度策略**：波次开始前由 Lead 统一声明「谁在何时持有 lane」，避免 protocol 的 3 秒 check
   排在 7 分钟回归后面；建议 lane 增加优先级（重活 heavy 与轻 check 分队列）。
4. **既有慢测隔离**：`tui_dialog_wave39` / `tui_render_layout` 的 sqlite 子模块用例应拆出独立 target，
   否则每次回归都要为它付 6~7 分钟。
5. **L3-P 的判定基准要与真实门对齐**：本轮 L3-P 的 BLOCKING 完全等价于 `rust:lint` 一条门，
   六问里 5 问是 N-A/PASS——说明对「纯本地 UI 变更」这类任务，L3-P 的边际价值主要在**契约兼容性**一问，
   建议按任务类型裁剪六问（保留契约兼容 + 日志/凭据，其余记 N-A 并给结构性理由），把省下的审查预算
   转投 Plan 期对抗（本轮 ROI 最高的一环）。
