# Wave-24 元复盘：历史会话展示时序缺陷（思考/工具/正文排序）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，而非业务代码细节。
> 账本：`.dev-team/state.json`（wave 24）｜裁定书：`.dev-team/wave24-adjudication.md`｜降级记录：`.dev-team/wave24-pool-degradation.md`｜物理凭据：`.dev-team/evidence/wave24-*`

## 1. 通信拓扑与信噪比（Topology & Noise）

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 原始名册 | Lead + test-agent + executor-agent（3/8，单模块展示层改动，分类 B） | 合适 |
| 膨胀名册 | 后备成员 test-agent-24 / executor-agent-24 / executor-agent-25 / test-agent-26 陆续加入（峰值 7/8） | **不健康**：成员创建即失败，导致名册虚增但有效产能未增 |
| 通信方式 | 看板 + 零废话回执（要求「命令 + 汇总行 + Exit Code + 逐用例红绿」） | 好：回执可机械核验，无吹捧套话 |
| 最大信噪比收益 | 实施者主动报备「规格与既有冻结用例 :3138 冲突」并给出实测 `1 failed | 963 passed` | 好：避免了用假绿掩盖契约冲突，把裁决交给 Lead |
| 最大噪音 | 反复的 teammate 创建/唤醒失败 + 未知 subagent 失败通知（每次无 closing message） | 差：大量轮次耗在重试上，属平台级问题 |

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **假放行拦截（本波最大价值）**：执行者第一版只跑「定向子集」即申报全绿；Lead 以**全量 `npx vitest run`** 复核，立刻抓出 `tests/runtime-store.spec.ts` 2 处失败。若采信执行者自证，本波会以「Store 数组顺序被污染」的回归交付。
  → 协议修正：**全量套件是唯一放行门禁，定向子集仅用于开发内循环**；执行者回执必须给出全量汇总行，否则判无效。
- **假绿/空转门禁拦截**：浏览器门禁首跑「2 passed」看似完美，但 Lead 反向验证（回退 src → 重建 dist → 重跑）发现仍为 2 passed —— 根因是 playwright 的 `preview:e2e` 服务的是**旧 dist**，门禁实际测的是上一版构建产物。修正后重建基线产物，门禁转为 `2 failed`（真红相），恢复修复后 `2 passed`。
  → 协议修正：**浏览器门禁必须与构建产物同源**（`build` 与 `playwright test` 串联执行），且任何新增门禁都必须做一次「回退实现 → 必须变红」的判别力证明，否则视为假测试。
- **测试自身缺陷识别**：F5 首跑失败于「正文文本为空」，根因是验收 spec 把 `MarkdownRenderer` stub 成 `<slot/>`，而正文走 **prop `content`** → stub 吞内容。判定为用例缺陷（非产品缺陷），修复 stub 而非弱化断言。
  → 协议修正：凡 DOM 文本断言，必须先证明 stub/夹具能真实承载该内容，否则断言必然空转。
- **契约冲突的正确处理**：`tests/HomeWorkspace.spec.ts:3138` 与用户指令直接冲突。Lead 未采取「改实现迁就旧测试」或「改旧测试迁就实现」的单边做法，而是先取证（`git log -S` 定位引入 commit `80bf840`、读其 design.md 无排序决策、读 ADR-0008 证明工具时序落盘即丢），再裁定并留档（§6）。**误杀率 = 0**（无既有正确行为被误改）；**漏杀率**：由全量套件 + 反向判别力证明兜底。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **写域严格正交**：`tests/acceptance/` + `tests/HomeWorkspace.spec.ts`（测试侧）与 `src/components/HomeWorkspace.vue`（实现侧）由**不同执行体**承担；实施者明确「未运行测试、未触碰 tests」。
- **两处越界风险被机制挡住**：
  1. 执行者擅自改 `src/lib/runtime/messages.ts` 的数组契约（越出「只修展示」的意图）→ 被全量套件 + Lead 裁定打回；
  2. 执行者若为让门禁变绿而改冻结用例 → 权限设计上禁止（tests 只读），实际由 Lead 走契约演进门禁。
- **降级后的分权底线**：teammate 不可用时改用无状态 Subagent 池，但**测试作者 ≠ 实现者**始终成立（详见降级记录 §3）。
- **成本观察**：本波 src 净变更 15 行，但协调开销（契约冲突取证、门禁反向验证、5 次成员失败重试）远超代码量 —— 对「单点展示排序」这类任务，B 级全套 Agent Team 的边际收益偏低，A 级（Lead 兼测试 + 单执行）可能更经济；本波之所以仍值得，是因为暴露了 2 个门禁级问题（假放行、假绿门禁）。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **【硬门禁】全量套件为唯一放行凭据**：dev-team skill 的「机器事实调用点强契约」应显式写明——定向子集不得作为放行证据；回执缺全量汇总行即判无效。
2. **【硬门禁】新增门禁必须自带判别力证明**：任何新写的门禁（单测/组件/浏览器）在交付前必须执行一次「回退实现 → 门禁必须变红」，红相日志与恢复后的绿相日志同时入档。这是本波唯一识破「假绿门禁」的手段。
3. **【硬门禁】构建产物一致性**：浏览器级门禁必须 `build` 与 `playwright test` 串联；不得对已存在 dist 复用验证，否则测的是历史产物。
4. **【角色】增加「契约冲突上报」通道**：实施者遇到「规格与既有冻结用例冲突」时，正确动作是**报备并停手**（本波 executor-agent-25 做到了），而非二选一硬做。建议在 Executor prompt 模板中固定写入该条款与回执格式。
5. **【平台】teammate 续跑/创建失败需可观测**：本波 5 次成员失败均无 closing message，无法定位失败阶段，只能整体降级到 Subagent 池；建议平台暴露创建/续跑失败原因，并在 skill 中固化「连续 N 次成员失败即降级并留痕」的确定性规则（当前为 Lead 临场判断）。
6. **【效率】A/B 分级校准**：对「单文件 + <50 行 + 无 IO/并发」的展示层修复，建议默认 A 级（Lead 兼测试 + 单执行 + 1 路审查），把 B 级全套留给跨模块/契约冲突类任务，避免协调开销超过代码量。

## 5. 交付物理凭据索引

| 门禁 | 命令 | 结果 | 凭据 |
| :-- | :-- | :-- | :-- |
| 验收契约（F1-F5） | `npx vitest run tests/acceptance/stage-history-message-order.spec.ts` | 5 passed / Exit 0 | — |
| 全量前端套件 | `npx vitest run` | 90 files / 965 passed / 10 skipped / 0 failed / Exit 0 | `evidence/wave24-final-full-suite.log.gz`（原始字节，gunzip -c 查看） |
| 构建 + 类型 | `npm run build`（vue-tsc --noEmit && vite build） | Exit 0 | `evidence/wave24-build.log` |
| 版本同步 | `npm run version:check` | Exit 0 | `evidence/wave24-version-check.log` |
| Rust（原生替代 powershell 脚本） | `cargo check --manifest-path src-tauri/Cargo.toml` | Exit 0 | `evidence/wave24-cargo-check.log` |
| 真实浏览器门禁（绿相） | `npx playwright test tests/e2e/wave24-history-order-gate.spec.ts` | 2 passed / Exit 0 | `evidence/wave24-browser-gate.log` |
| 真实浏览器门禁（基线红相，判别力证明） | 同上（回退 src + 重建 dist） | 2 failed / Exit 1 | `evidence/wave24-browser-gate-baseline-red.log` |
| 视觉凭据 | Chromium 截图 | 2 张 PNG | `evidence/wave24/w24-e2e-*.png` |
