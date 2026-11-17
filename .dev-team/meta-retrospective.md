# 元复盘：B232 审批续轮上下文（A 级，wave approval-resume-context）

> 交付：分支 `fix/approval-resume-ctx`（红相 985d0e02 / b51eaea7 / 2cfc6973，实现 533bce0c）。
> 收据：`.dev-team/review-plan-b232.json`、`.dev-team/review-join-b232.json`、`/tmp/b232-final-gates.log`。

## 1. 通信拓扑与信噪比

- 角色：Lead（契约/测试/ADR/裁决）+ exec-tui（唯一实现者）+ 2 个 fresh subagent（方案审查、实现审查）。Roster=2，符合 A 级容量。
- 有效信号：两次审查各命中 3 条 BLOCKING（方案 3 条、实现 1 条新增链式缺陷），全部在合并前修复；无「先吹后改」套话。
- 噪音与浪费：①第一次全量 bin 回归在 16 核上把 load 推到 37（超 1.5× 预算），被及时 kill 并改为三条子集——教训是「全量回归也要先评估既有慢用例（ask_lifecycle 单例 >60s）」；②冻结测试经历 985d0e02→b51eaea7→2cfc6973 三次重锚，其中 v2/v3 各由一次审查直接驱动，属必要成本，但首版测试设计（从 UI 气泡取数）本可在写测试前用一次 30 秒 grep 证伪。
- 重复执行：构建队列争用导致同一测试命令排队 5–10 分钟，未重跑同一命令（遵守反重试纪律），改用后台作业 + 等待，未产生重复构建浪费。

## 2. 门禁穿透与误杀率

- 穿透修复（真实拦截）：Review-1 的 B1（落库取 `display_message`）击穿了「组合正文随续轮持久化」这一承诺——若放行则 F-12 只修一轮；B2 击穿「捕获来源正确性」（会张冠李戴旧轮指令）；B3 击穿「测试有效性」（纯函数自证、接线零断言）。Review-2 又抓出链式挂起（Ask→审批、审批→审批），属同一缺陷的第二现场。
- 误杀：0 条 BLOCKING 被证伪；1 条 ADVISORY（T6 断言别模块常量）属装饰性断言，已改为比较两条组合结果；1 条报告「8 例」与 v2 契约不一致，已修文档。
- 机械门禁：`cargo check` exit 0、approval 21 / running_status_visibility 23 / composer_history_recall 3 全绿、12 例冻结验收全绿；`audit-diff(--decl-approved)` 与 `check-declaration` exit 0；ADR `verify-note` 与 backlog `lint+doctor` PASS。
- 盲区（如实登记）：bin 全量套件未跑（预算+慢用例）；接线点（事件循环调用 capture/seam）仍无自动化断言，靠「唯一接缝 + 代码审查」兜底；审批请求不持久化 ⇒ 重启/多实例下仍会退回旧行为（A1，ADR 已登记为边界）。

## 3. 分工契约与隔离有效性

- 写域隔离：实现者仅写 `bin/tui.rs`，Lead 仅写测试/契约/ADR/状态账本/docs；两次审查均为只读 subagent，零越界。`git status` 与 write_scopes 机械比对通过。
- 测试分权：冻结测试由 Lead 编写并在实现前锚定（红相 22 个编译错误为证），实现者全程只读测试文件；三次重锚均由审查结论驱动并留流水，不是「为了让测试变绿而改测试」。
- 共享工作区风险：主工作树有并行会话 18 项未提交改动（含 `tui.rs`），本卡改用独立 worktree（`/home/dm/pony-agent-worktrees/approval-resume-ctx`），零冲突；但 backlog 账本是跨 worktree 单例（写入主工作树），B232 的 record/start/resolve 落在主树未提交文件中，交付时需由主树使用者随其提交带入。
- 契约演化：v1→v2→v3 均按「审查→Lead 一票裁决→原地改写契约+流水」推进，未出现口头继承。

## 4. 元协议迭代建议

1. 冻结测试前增加**最小证伪步骤**：凡「数据来源」类设计（如从 UI 气泡取数），要求先用一条 grep/read 证明该来源在生产时序下确实存在对应记录，再写断言——本轮 v1 的 T4/T7 就是缺这一步而返工。
2. A 级也建议强制「实现 diff 的第二路审查」（本轮正是第二路抓出链式缺陷）；成本约 1 次 subagent，收益是阻断同病第二现场。
3. 回归范围在契约里必须写**具体坐标**（本次三条 filter），而不是「bin 全量」——慢用例会让全量回归与构建预算直接冲突，最后只能豁免，等于没写门禁。
4. 建议在 dev-team skill 中补一条「跨 worktree 单例账本（backlog）的交付归属」处理约定：明确由哪个工作树提交、如何避免与并行会话的 in_progress 行混入同一提交。
