# Wave-39 元复盘（TUI Composer 收边 + 底部指标行）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，而非业务变量命名。账本 `.dev-team/state.json`（wave 39）、契约 `.dev-team/contract-matrix-tui-dialog-wave39.md`（含 §5 v2 变更）、收据 `.dev-team/wave39-*.md`、评审 `review-wave39-{a,b,p}.json`、E2E `.dev-team/evidence/tui-polish/after/`。

## 1. 通信拓扑与信噪比

| 角色 | 模式 | 轮次 | 评价 |
| :-- | :-- | :-- | :-- |
| Explorer（E1） | subagent 只读 | 1 | 一次侦察给出 7 组 `path:line` 事实与「将破坏的测试清单」，Lead 未再全量读 tui.rs，上下文节省显著；抽验 `hairline()`=▁ 与 footer 行位均成立 |
| Protocol Agent | 常驻 teammate | v1 + v2 各 1 | v1 冻结 R1-R3 空桩（+50 行/0 逻辑），v2 仅 +7 行；v2 由独立 teammate 承担，避免 Lead 碰业务文件 |
| Test Agent | 常驻 teammate | 红相 + 独立验收 + delta 红相，共 3 轮 | A1-A10 620 行真渲染断言、A11 144 行；拒绝改写断言，纪律到位 |
| Executor | 常驻 teammate | 主实施 + delta 实施，共 2 轮 | 严格单文件写域；主动披露并行前端改动非其所写 |
| Reviewer A/B/P | subagent 池 | A 两轮（含 delta 复审）、B 一轮、P 一轮 | A 的非机器观察（双计竞态）直接转化为 v2 契约，是本次最高价值的审查产出 |

- **事件驱动唤醒有效**：全程以任务板 revision + 产物 mtime 为信号，`wait_agent` 只用于真阻塞点；无空转轮询。
- **噪声点**：Lead 为确认 E2E 产物，两次以 `sleep` 轮询回收 job 输出（共约 5 分钟空转），应在启动长任务时直接依赖完成通知而非 sleep。
- **信噪比**：审查 JSON 严格 schema，回报均压到 12-30 行；唯一长文是 E2E 收据（可接受，属物理凭证）。

## 2. 门禁穿透与误杀率

- **真实拦截**：L3-A 抓出 `output_end` 与 `completed` 双发 usage 的**可达竞态**（旧轮 completed 迟到 + 新轮失败 → 永久多算）。该缺陷在红相 A1-A10 全绿、L3-B/L3-P PASS 时仍存在，证明「渲染断言全绿 ≠ 状态机正确」，互补审查矩阵（A 视角=并发/状态流转）有效。
- **门禁穿透（必须修）**：`dev-stub-lint.py` 仅分派 `.py/.ts/.js`，对 `.rs` 走 `else: errs=[]` **空过 PASS**——本次靠 Protocol 自证 + 正则复核兜底，属**假门禁**，风险等级高，必须补 Rust 分派。
- **既有基线红**：`tui_render_layout` 3 passed/5 failed、`tui_acceptance` 34/1 在波次起点即失败（陈旧断言：`'─'` vs `▁`、footer 就绪、用户轮标签、caret 偏移、中文正文；`test_matching_commands_filter`）。Wave-39 用「基线逐项比对 + 不新增失败」放行，属**被迫的规范化漂移风险**，应开专项清偿。
- **E2E 4 条失败**：`/thinking` 引入后命令表 8 条与冻结期望 7 条漂移；用 `git archive ba530bd` 构建 pre-wave 二进制 1:1 复现，判定既有问题、非本波次回归，且未放宽 checks.py —— 归因流程可复用（`wave39-attribution/`）。
- **误杀/过度审查**：258 行变更挂载 2 路 L3 + 1 路 L3-P + 1 轮 delta 复审，总成本偏高但命中 1 个真实缺陷；若未命中则属过度，建议按「命中缺陷数」滚动校准审查路数。

## 3. 分工契约与隔离有效性

- **测试分权**：Test Agent 写 A1-A10/A11，Executor 只读测试；两次拒绝「改测试放宽」的诱惑（E2E 4 条失败、delta 断言），分权链条未被穿透。
- **写域隔离**：全程仅 `tui.rs` 由 Executor 写，`tests/`+`tools/` 由 Test Agent 写，`.dev-team/` 由 Lead/各自收据写；写域机械审计发现 149 个 tracked 证据文件被 E2E 重写 + 并行 `src/stores/providers.ts`，均通过**定向 `git add`** 隔离，未误提交。
- **契约冻结与演进**：v1 冻结后，L3-A 发现缺口 → Lead 裁决 §5 v2 → Protocol 出 v2 空桩 → Test Agent 先红相 → Executor 实施。演进链完整，无「波内就地改契约」。
- **共享文件多阶段提交技巧（值得沉淀）**：同一文件同时存在「已实现改动 + 新增空桩」时，用 `git diff` 抽取单个 hunk → `git apply --cached` 仅暂存空桩 → 提交 protocol，使「protocol → red → green」三段提交在单一共享文件上仍可物理分离，不必 revert 整文件。
- **信息下发**：Executor 未被告知 L3-A 竞态细节，只拿到 v2 契约与红相失败；Test Agent 未拿到实现思路。分层投影基本成立。

## 4. 元协议迭代建议

1. **[高] 修 `dev-stub-lint.py` 假门禁**：补 Rust 分派（允许 body 仅含 `unimplemented!()/todo!/panic!("NotImplemented")`，禁默认实参表达式/副作用），否则所有 Rust 项目的「空桩纯度」门禁形同虚设。
2. **[高] 新增「单一权威事件」协议条款**：凡新增消费流式事件的状态，必须先枚举该状态在每个事件上的语义（谁权威、谁冗余），并在契约中写明「唯一构造点」；本次双发缺陷即源于缺失该条款。
3. **[中] 审查后置「delta 闭环模板」**：non-machine 观察不得以 PASS 直接结案——要么转为 v2 契约 + 红相 + 复审（本次做法），要么写 `adjudications` 并注明残余窗口与基线对比；建议把该二选一写进 skill。
4. **[中] 基线红债务必须工单化**：允许「不新增失败」放行，但必须在 Phase 4 强制生成带编号的清偿工单（本次已建议 wave-40：`/thinking` 命令面 + checks.py 7 命令期望 + tui_render_layout 5 条陈旧断言）。
5. **[低] 长任务调度**：启动既有 E2E/构建 job 后，Lead 应直接等通知，禁止 `sleep` 轮询；本次浪费约 5 分钟墙钟与 4 次工具往返。
6. **[低] 审查预算与爆炸半径挂钩**：建议 skill 给出「命中真实缺陷数 / 审查调用数」指标，作为下一波审查路数的校准依据，避免小改动堆叠 4 路审查。
