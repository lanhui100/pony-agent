# 2026-10-18 wave-36 PA-138..PA-145 修复波（dev-team 编排）

## 范围与分级

任务板 wave-35 残留 8 卡（PA-138..145）验证并修复。B 级：跨 6 个 Rust 模块 + CI 工作流 + 测试基建；
含安全姿态变更（PA-141）与判据语义变更（PA-143/144）。用户裁定：PA-141 采用方案 (b) 宿主审批；
PA-140 允许推分支触发 CI。

## 编排

- Protocol Agent：契约矩阵 `.dev-team/contracts/wave-36-pa138-145.md`（45 条 C-id）+ NFR 增量 + state 账本。
- Test Agent（独立分权）：红相测试冻结（7062f1c），L2-T 收据 `receipts/wave36/RED_PHASE_MANIFEST.md`。
- L2-AT（subagent 池）：对抗测试 `tests/wave36_adversarial_classification.rs`（t01-t08）。
- Executor-A/B（teammate 失败后降级为无状态 subagent 池，wave-24 先例）：实施 PA-138/139/140/141/142/143/144/145。
- Lead：红/绿锚定提交、裁决（C-138-4 fail-closed、C-141-8 complete_step 同审批、c141_8 测试内部矛盾修正、
  t05/t07 对抗漏网修复、TTFT 单时钟采样）、Join Barrier、CI 收据、收口。

## 关键裁决

1. C-138-4：`context.session_id=None` 时若 PlanStore 存在任一 Draft ⇒ Deny（保守 fail-closed）。
2. C-141-8：complete_step 的 Draft→Executing 迁移与 start_execution 同审批（堵第二条自解锁路径）。
3. c141_8 测试期望 `executing`→`completed`（PlanStore「全部 step 完成⇒Completed」冻结语义）。
4. L2-AT 对抗抓到 2 处实现漏网：数字紧邻字母（429abc）、IPv6 literal host:port —— 全部采纳修复。
5. PA-145 取舍：纯 "500"/"529" 文本 L1 由 Retry→Abort（classify 码表冻结无此二码，两出口一致性优先）。

## PA-139 根因（重要）

两套独立竞态叠加：
1. MockHttpServer 队列消费无归属校验 —— 杂散连接读到完整请求体即取走响应，真实请求 1-2ms `client error (Connect)`
   或拿到确定性 503×5。修复：`start_with_predicate` 归属校验（不匹配→503 不消费队列），原 `start()` 语义不变。
2. wave15 TTFT 双时钟采样：per-call 与轮级 TTFT 各自独立 `elapsed()`，采样点漂移可反转（82ms > 75ms）。
   修复：tool_recovery.rs delta 回调合并为单次 `Instant::now()`。

## 验收（物理收据）

- 全量：`RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib` ⇒ 1064 passed / 0 failed（~34s）。
- PA-139：50/50 全量连跑全绿（21:03–21:32，`receipts/wave36/pa139_runs/run_01..50.log`）。
- PA-140：CI run 39132384596 — ubuntu core-platform-check **success**（1064 passed），macos success；
  Windows verify 红为 main 预存（run 39103198486 同 job 同步失败），本波零新增 Windows 失败。
  PR #2（draft）。收据 `.dev-team/evidence/wave36/ci-run.txt`。
- 对抗 8/8、census 0 violation/111 files、wave30b 3/3、recovery_acceptance 6/6、adversarial suite 8/8。

## 过程事故（留档）

1. 两次外部 `git stash` 卷走整个未提交工作区（悬空快照 330012aa），已完整恢复；多会话共享工作区应禁用 stash。
2. 两套 50x 并行互相干扰（端口/收据覆盖）——长时验收必须单实例独占。
3. 常驻 teammate 多次轮次失败，按 wave-24 先例降级为无状态 subagent 池 + Lead 兜底。

## 残留登记

- 桌面端审批 UI/IPC 接线（`TurnInput.plan_execution_approval` 宿主入口）—— 建议立卡。
- Windows verify 的 hook_commands/mcp_client 预存失败 —— 平台债，另立。
