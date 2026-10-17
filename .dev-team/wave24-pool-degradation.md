# Wave-24 物理账本补充：Agent 池降级记录（Lead 记账）

## 1. 现象（可观测事实）
wave-24 期间，`spawn_teammate` 与 `send_message` 唤醒的**常驻 teammate** 多次在无 closing message 的情况下失败（inactive / "failed before it finished"）：

| 成员 | 动作 | 结果 |
| :-- | :-- | :-- |
| test-agent | 完成 task-1 后被要求执行 task-4 | 未被唤醒（任务板无 claim，文件无改动） |
| executor-agent | 完成 task-2 后被要求执行 task-5 | 未被唤醒（任务板无 claim，工作树无改动） |
| test-agent-24 | 新建 → 完成 task-4；再被要求执行 task-6 | 第二次续跑失败（文件 mtime 未变） |
| executor-agent-24 | 新建 | 创建即失败（无任何写入） |
| executor-agent-25 | 新建 | 成功完成 task-5 |
| test-agent-26 | 新建 | 完成 task-6 的 F5 部分后失败（HomeWorkspace:3138 未改） |

## 2. 降级动作（Lead 裁定）
按 dev-team skill 的降级路径，改用**无状态 Subagent 池**承担剩余测试修订与业务实现：

| 工作 | 执行者 | 独立性依据 |
| :-- | :-- | :-- |
| 修订 `tests/HomeWorkspace.spec.ts:3138` 断言与标题 | subagent 3029cc12 | 非 src 实施者，仅写 tests |
| 移除 `fallbackReasoningShown` 条件化（src） | subagent 1db43ca3 | 与测试修订者为**不同** Subagent，未触碰 tests |
| 修复验收 spec 的 MarkdownRenderer stub（测试基建缺陷） | subagent fc697924 | 仅改测试 stub，未触碰 src |
| 新增真实浏览器门禁 spec `tests/e2e/wave24-history-order-gate.spec.ts` | subagent 45ef0917 | 仅新建 e2e spec |
| 终审物理门禁（全量套件 / build / Rust check / 浏览器门禁） | Lead | Lead 不放行未取得机器证据的交付 |

## 3. 独立性声明（BASE-W24-1）
虽然执行者由 teammate 降级为 Subagent，但**测试作者与实现者始终不是同一个执行体**：
- F5（新增验收用例）与 :3138 断言修订由测试侧 Subagent 完成；
- toolOrder 无条件化由另一个 Subagent 在 `src/components/HomeWorkspace.vue` 内完成，且其明确未运行测试（避免与并发测试文件编辑互相干扰），最终放行门禁由 Lead 执行。
- 因此「Executor 不得自写自测」的分权底线在降级后仍成立。

## 4. 遗留风险
- 常驻 teammate 续跑失败原因不可观测（无 closing message），本波无法定位到具体失败阶段；建议后续在平台侧排查（本记录仅作事实留痕）。
- 受影响任务 task-6/task-7 的 owner 已按 Lead 记账收敛为 lead，待门禁通过后 complete。
