# 元框架复盘（Wave-59：桌面端 UI/UX 四项优化）

> 本轮对象：wave-59 —— 按用户四项 UI/UX 反馈（等待动画去重 / 计数下移 / 流式闪跳 / 状态准确性）做根因诊断与修复。
> 交付：`.dev-team/wave59-delivery.md`；契约：`.dev-team/contracts/wave-59-desktop-uiux.md`；ADR：`docs/decisions/0056-...md`。
> 团队：Lead + test-agent + executor（常驻 3）+ 4 次 Subagent（2 路 L3 审查、诊断复核）。
> 交付区间：`d3745a7` → 工作区未提交（见交付 §8「提交依赖外部波次」）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑选择基本正确**：test-agent 与 executor 是跨阶段有状态连续体（红相→绿相→复审），落为常驻 Teammate；
  两路 L3 对抗审查（计划可验证性 / 合入正确性）属无状态投影，落为 fresh-context Subagent。名册峰值 3 ≤ 8。
- **写域正交成立**：`src/**`（executor）‖ `tests/**`（test-agent）全程零冲突，无跨写域脏写。
- **真实浪费点（Token）**：Lead 亲自做了全部根因诊断（RC1..RC9），期间 `HomeWorkspace.vue` 读了 3 次、
  `stores/runtime.ts` 分段读了 5 次、`MarkdownRenderer.vue` 读了 2 次 —— 命中「≥5 文件 / 横跨 ≥3 模块」的
  Explorer 委派阈值却未外派，注意力与上下文被读取动作吃掉。改进：诊断阶段先外派 E2 Explorer 出「根因候选 + 证据行号」，
  Lead 只做定点复核。
- **有效实践**：test-agent 在共享工作区被并发波次污染后，改用 `git archive d3745a7` 纯净快照采集红相，
  并给出源码 md5 可复算 —— 这是本波最有复用价值的元协议动作。
- **信噪比**：teammate 回执普遍「结论 + file:line + Exit Code」，无套话；executor 主动上报自检发现的新增回归
  （`markdown-streaming-smooth` 由绿转红并自修），属正面行为。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- **真实拦截（L3 价值 > 全部机器门禁）**：合入审查抓到 **1 BLOCKING** ——
  `activeStreamKind` 在 `turn:completed/failed/cancelled/suspended` 四路终态均不复位，而末次模型调用按协议不发
  `hop_complete`，故每轮结束后残留 `"content"`；下一轮 `submitTurn` 到 `turn:started` 的 awaiting-first-signal 窗口
  被判 `responding` 且状态条隐藏，三点动画又已删除 ⇒ **该窗口零进行中指示**（违反 F1-2/F4-2）。
  该缺陷在 18 条验收用例全绿的情况下未被任何机器门禁发现：F1 夹具用新 pinia（`activeStreamKind=undefined` 走回退路径），
  **恰好掩盖了"状态残渣"这一类缺陷**。
- **另 2 条 ADVISORY 同样是本波新引入的回归**：`MarkdownRenderer.scheduleNonStreamingRender` 在空白内容提前 return
  却已置在飞标记 ⇒ 后续流式更新被 dirty 永久吞掉；`streamingRenderPathLock=false` 延续到完成态 ⇒ 末帧才出现的
  markdown 语法被原样显示。二者均由审查给出可跑反例（`/tmp/w59-review/*.spec.ts`）。
- **门禁穿透（流程）**：Lead 在 `update_goal complete` 之前未执行 Phase 5 元复盘、Phase 6 入队与 L3 审查，
  属流程偏离，本文件与两路 review JSON 为事后补做；补做后才发现上述 1 BLOCKING。
- **误杀**：0 条 BLOCKING 误判；7 条计划阶段 ADVISORY 中 3 条（F3-5② 口径未落契约、F4-5 夹具自证、F4-2 钉死文案）
  被 Lead 采纳为契约修订 D-59-7/8/9 与 A-59-3/4，其余 4 条登记为可接受或已覆盖。
- **门禁自身假绿**：`dev-meta-lint.py` 仅 glob 判 `review-plan*.json` / `review-join*.json` 是否存在 ——
  历史波次残留文件即可让它变绿。本波主动产出 wave-59 专属两份收据并点名此机制缺陷。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **测试分权成立**：test-agent 独立依契约矩阵写 18 条验收 + 迁移 6 条既有断言（仅换定位锚点，物理口径 0.5px / 单调不减未动）；
  executor 全程只读测试，两次「测试红」均改实现而非改断言。红相/绿相均有 md5 钉版溯源。
- **契约冻结有效但有一次缺口**：F3-5② 的**收窄口径只写在红相收据 §6，未同步到契约矩阵**，直到 L3 审查才暴露；
  说明「裁定落收据 ≠ 裁定落契约」。已在 §7 补登记 D-59-7。
- **共享工作区隔离失败（本波最大教训）**：工作区内另有一个**仍在活动**的波次（slash commands / TUI，写入时间到次日 09:02）
  与 wave-59 同文件混改（`stores/runtime.ts` getter 区域两波改动落在同一 hunk、`status.ts`、`HomeWorkspace.vue`）。
  后果：① 无法提交（hunk 级切分不可行，强行提交会产出不可构建中间态）；② 全量门禁数字持续漂移（1139→1142→1144）；
  ③ `wave47-ob2` 因他人改 `docs/observability.yaml` 而红，一度干扰归因。**Roster 内正交 ≠ 工作区正交**。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **共享工作区存在外部活动会话时，红相/绿相一律在 `git archive <base>` 快照或独立 worktree 采集**，并把源码 md5 写入收据。
   （本波已验证有效，建议写进 `dev-team` 的「红相采集」条款，替代「在共享工作区直接跑」。）
2. **状态机类改动必须补「终态残留」用例**：断言「turning terminal → 立即开新轮」窗口内的 UI 语义；
   夹具禁止用全新 store 实例掩盖跨轮残渣（本波 F1 夹具即栽在此）。
3. **L3 对抗审查必须在 `update_goal complete` 之前强制跑**；契约中任何「口径收窄」必须当场落契约矩阵，
   不得只落在红相收据（否则交付说明会过度声称覆盖，本波 F3-5 一度把 RC1..RC5 全清当成已证）。
4. **提交前置检查「是否存在外部活动会话」**：若有，先做无损快照（`git stash create` + 未跟踪文件打包），
   并把「待提交文件清单 + hunk 切分可行性结论」写入交付说明，再交给人裁决提交时机。
5. **诊断阶段优先外派 Explorer**：Lead 亲自读 ≥5 文件属反模式，改为 E2 Explorer 出「根因候选 + path:line + 反证」。

## 5. 审查两轮闭环（2026-11-02 追加）

- **r1 → FAIL**：1 BLOCKING（B-59-1 `activeStreamKind` 终态不复位）+ 2 ADVISORY。**三条都是本波新引入的回归**，而当时 18 条验收用例与全量机器门禁全绿 —— 说明"夹具覆盖契约断言"不等于"门禁覆盖状态机"。
- **r1 修复 → r2 PASS**：三处改动均以「守卫之后 / 早退之前」为落点，2 条 ADVISORY 由复审自建 5+6 个探针独立复验（`/tmp/w59r2/*`）。
- **r2 新发现 A-59-5**（同类第五入口：会话切换携带 hop 级瞬时态）→ 当轮修复，并新增 F1-5 覆盖；验收用例 18 → 23。
- **变异敏感性自证**：test-agent 用 6 个 mutant 反向验证新用例确能捕获对应缺陷（M1..M4/M6 真红、M5 对照 PASS 如实登记）—— 这是本波最值得固化为流程的动作：**新增门禁必须附 mutant 反证**。

---

# 元框架复盘（wave-pds-3：流式视觉层接入）

> 基线 `8e51bd5`（红相锚定）→ 交付 `f3979e4`。只审协作元框架与门禁有效性，不审业务代码细节。

## 1. 通信拓扑与信噪比

**拓扑**：Lead（契约 + 裁定 + 栅栏验收）→ Test Agent（冻结验收，独占 `tests/acceptance/pds-streaming/`）→ Executor（实施，独占 6 个 src 文件），审查下沉 Subagent 池（每轮 1–2 路，fresh context）。串行 WIP=1——渲染链路强耦合，并行 ROI 不成立。

**有效的一面**：Test Agent 拒绝迁就契约，先后提出 **11 条契约缺陷**（互斥的 `charsPerTick`、物理不可达的 16.7ms 阈值、`data-tail-len` 语义歧义、jsdom 读不到 `animation-name` 等），全部由 Lead 裁定并入契约。若它选择"把测试写成能过"，本波会在错误判据上假绿交付。

**无效的一面**：**契约在实施期修订了 10 版**。前 6 版的缺陷本可在设计期机械发现（互斥检查、阈值可达性、判据可执行性），却各花一轮往返（15–25 分钟）才发现。信噪比问题不在措辞，而在契约缺少**发布前自检清单**。另：Lead 曾用"接入"描述一次零视觉变化的等价重构，导致用户以为动效已上线——术语必须对齐用户可感知结果。

## 2. 门禁穿透与误杀率

**本波最重要的发现：冻结验收抓不到最大的缺陷。** v1 的 V1-5 用纯文本夹具（6–29 字），而实现按「最后一个 `\n`」切分把表格/标题/加粗/嵌套列表/多行段落全渲染错——V1-5 **恒绿**。同类假绿共 3 处：V12-5 源文本走纯文本快路径（压根不进 `MarkdownRenderer`）；V6 在 v7 后单行/多行两分支样本数变 0；R6 在 `data-pds-status≠null` 分支无 rise 断言（上一轮的"rise 空转"照样绿）。

**修正手段（可复用）**：V11-0 用同一套仓内渲染器**复现缺陷实现**，要求 6 个检测器逐个报警（实测 6/6）；V14-0b 校验"夹具登记的资格依据"与渲染产物形态一致，渲染器一变它先红；R6-b 把 jsdom 断不了的 computed 断言移到 Chromium。

**误杀率**：V8 帧预算判据先后出现假绿与假红——污染轮（idle 漂移 2ms）判 pass，干净轮因偶发 Long Task 判 fail。改为**同轮 A/B 差分**（A=`tail-reveal=off` 即旧行为，B=新行为）后直接测"本改动自身开销"，环境项在差分里消掉；再加 `[LOAD-CONTAMINATED]` / `[SPIKE]` 标记化重测（阈值未放宽，3 轮干净仍是硬要求）。最终 `B−A = +0.10/+0.10/+0.20ms`。

**另一类误杀**：V10-2/V10-3 会在子进程实跑整套测试，**工作树正在被写入时跑门禁**会把"文件半成品"放大成基线回归——本波实际踩到两次（Executor 写 `MarkdownRenderer.vue`、Test Agent 写 `v14` 期间）。

## 3. 分工契约与隔离有效性

**隔离成立**：Executor 全程未碰 `tests/**`、`crates/**`、`docs/observability.yaml`，未改 `DEFAULT_REVEAL_BUDGET`；写域越界仅一次（`350ms→140ms`），由 Executor **主动上报**、获书面授权后执行，最终收缩为 `1/1` 行。Test Agent 全程未碰 `src/**`。

**隔离失效一次**：两个 teammate 并发共享工作区时，Test Agent 写 `v14` 的同一时间窗内 Lead 跑全量套件 → "1 file failed" 假红；文件稳定后复跑全绿。**结论：并发写与门禁执行必须互斥**，但当前只有 advisory 的 `write_scopes`、没有锁，只能靠纪律。

**跨波次隔离**：`.dev-team/` 内同时存在**另一并发波次**的 Rust 改动，靠提交前逐文件 `git status` + 定向 `git add`（全程未用 `git add .`）保住不串味。

## 4. 元协议迭代建议

1. **契约发布前置机械自检**（最高价值）：冻结前必须跑——① 断言与既有冻结用例的**互斥扫描**；② 所有阈值的**可达性实测**；③ 所有判据的**可执行性**（环境是否支持该 API）。三项均可脚本化，成本远低于一轮往返。
2. **验收夹具的"删除即红"要求**：每组新夹具必须演示"删掉对应实现该夹具变红"，否则不算验收。可直接消灭本波 3 处假绿。这与 wave-59 已沉淀的"mutant 反证"是同一条原则，应上升为**所有新门禁的硬要求**。
3. **性能门禁必须四件套**：阈值 + 采样窗口 + 同轮对照 + 污染策略，缺一即变成骰子。
4. **门禁执行与文件写入互斥**：门禁脚本开头加"工作树 mtime 静默 N 秒"检查。
5. **术语纪律**：描述成果时必须附带"用户可见变化是什么/有没有"，避免把等价重构说成功能上线。
