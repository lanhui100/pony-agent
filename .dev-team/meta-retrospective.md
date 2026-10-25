# 元框架复盘报告（Meta-Retrospective）

> 本轮对象：Wave-3（Token/速度双模计算 + 上下文用量诊断 + 调试埋点）。复盘重心为**协作元框架与组织协议本身**，非业务代码细节。

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead 统一调度；Protocol Agent 定契约/NFR；Test Agent 独占验收测试；Executor 独占 src/crates 实现；L3-P 审查以 Subagent 两轮挂载（首轮 FAIL、复核轮 PASS），不占常驻名册。
- **信噪比**：全程结构化指令 + 任务板状态流转；审查产出零废话 JSON 并经 `dev-review-lint.py` 机械校验（Exit 0）。
- **重复命令率**：偏高项 = `npx vitest run` 全量复跑 6 次（每次 ≈35s）。其中 2 次是为了独立核验 Executor 声称的绿相，属必要成本；另 1 次为排查一次性 `Errors 1 error`（复跑未复现，判定为构建/测试并发下的瞬态未处理拒绝）。**改进项**：Lead 应改为一次性并行采集（vitest + cargo + build 各自后台 job），避免串行重复。

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
- **真实拦截（有效）**：
  1. Lead 全量回归 **捕获 2 处 IMPL-DEFECT 回归**（`context-display-tokens.spec.ts:208`、`HomeSidebar.spec.ts:511`），Executor 自测声称"全绿"未覆盖到 → 说明"仅跑新增用例"的自测不可信，全量门禁不可省。
  2. Lead 逐行复核识破**"假绿"**：终态轮在 `useTraceProjection` 仍走 legacy 单次速度，而新单测直连函数 → 两组测试各走各路径，"用户诉求在 UI 上未修复"却全绿。这是本轮最关键的穿透案例。
  3. Lead 识破 **E2E 弱断言**：终态种子无 `providerCallRecords`，legacy 回退也能得到 250 t/s → 无判别力。改为注入"单次 375 ≠ 整轮 250"的判别式种子后，才具备证伪能力。
  4. L3-P 首轮 **BLOCKING**：调试属性无条件写入生产 DOM + `dur/ttft` 伪造占位 → 阻断合并；修复后复核转 PASS。
  5. Lead 独立执行**变异对照**：接线缺失时 `wave2_turn_flow_calculates_and_records_speed` 真实 FAILED（证据 `.dev-team/evidence/wave-2-metrics/rust-mutation-control.txt`）→ 证明该用例非空洞。
- **误判/超纲**：0 起 NFR-DEGRADED；L3-P 首轮 7 条 ADVISORY 中 3 条经复核确认有效并已闭环（OTel 死 API 接线、契约措辞收敛、delay-duration 夹带还原）。
- **协作事故（1 起，已闭环）**：Test Agent 做变异对照时用 `git checkout` 还原，**误清空 Executor 未提交的 `turn_flow.rs` 接线**。根因 = 共享工作区 + 未提交实现 + `git checkout` 粒度过粗。已由 Executor 重做恢复，并纳入协议改进（见维度四）。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **有效**：Test Agent 独占 `tests/`；Executor 全程未改前端测试；Lead 只做契约/验收/证据落盘，未写业务代码。
- **失效点**：Executor 在 task-15 越界写入 `crates/pony-agent-core/tests/otel_trace_migration_test.rs`（+150 行），违反测试分权。处置：不删除（用例有价值），改由 **Test Agent 接管所有权 + Lead 变异对照验证非空洞**。
- **滚动契约账本**：新增 `.dev-team/contracts/wave-2-metrics.md` 登记 Wave-15 §S3 的 **breaking（有界收窄）** 与 5 条兼容性证据，弥补了"契约演进无登记"的协议缺口。

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **禁止 `git checkout` 做变异对照（P0）**：任何"临时改→跑→还原"的实验，必须 `cp file /tmp/backup` 后用 `cp` 还原，严禁 `git checkout -- <file>`（会连带清除他人未提交改动）。建议在 skill 的「共享工具链互斥清单」中显式列出。
2. **共享工作区快照纪律（P0）**：波次内任何破坏性实验前，Lead 应强制 `git stash create` 或先做一次检查点提交，确保"未提交实现"不会成为他人实验的牺牲品。
3. **自测可信度分层（P1）**：Executor 报"全绿"时，必须附**全量**套件退出码；只跑新增用例的绿相一律不计。
4. **新增"判别力"门禁（P1）**：验收用例必须证明"错误实现会红"。凡断言值在不同实现路径下可能相同，即视为弱断言，强制引入判别式种子（本轮 E2E 375 vs 250 即范式）。
5. **契约诚实性门禁（P2）**：文档声称"可在 X 检索/可见"的属性，必须同时给出≥1 个真实调用点 file:line；无调用点即标注 `reserved`。本轮 OTel 四属性即因此收敛。
6. **Lead 证据采集并行化（P2）**：改用后台 job 并行采集 vitest/cargo/build/E2E，降低重复串行复跑。
