# Wave-59 红相收据（F1/F2/F3/F4 冻结验收 + 既有断言迁移）

作者：test-agent（L2-T，独立测试者）
契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md`
收据生成时间：2026-11-01 22:5x（Asia/Shanghai）

---

## 0. 结论先行

| 项 | 结果 |
| :-- | :-- |
| 新增验收用例 | **18 条**（F1×3 / F2×4 / F3×6 / F4×5；其中 F3-2、F3-3 各拆 2 条、F4-4 为并行守卫） |
| 迁移既有断言 | 7 条（wave17 C2-1 ×1、wave17 C2-2 ×1、HomeWorkspace ×2、MarkdownRenderer ×3、HomeWorkspaceMarkdown ×1） |
| 红相（当前实现 = 契约基线 `d3745a7`） | **vitest：21 failed / 92 passed / 7 skipped，exit 1**；**playwright：2 failed / 3 passed，exit 1** |
| 覆盖 | F1-1/F1-3、F2-1..F2-4、F3-1/2a/3a/3b/4/5、F4-1/F4-2/F4-5 全部真实失败；F1-2/F3-2b/F4-3/F4-4 在基线即通过（已在 §5 逐条说明原因，未伪造成失败） |
| 契约歧义 | 2 处，**已由 Lead 裁定**（F3-5 ② 采纳收窄读法 / F3-2a 保持严格读法），见 §6 |

**红相事实来源（关键）**：本收据的失败**不是**在共享工作区跑出来的 —— 采集期间共享工作区
`src/**` 被另一条并行工作流持续修改（见 §1），因此红相在**契约基线 `d3745a7` 的纯净快照**上采集。

---

## 1. 红相环境与源码溯源（可复验）

### 1.1 为什么用快照

契约矩阵声明基线为 `commit d3745a7`（工作区 clean）。但本 agent 采集期间（22:36–22:47）共享工作区
`git status` 从 clean 变为：

```
 M src/components/HomeSidebar.vue
 M src/components/HomeWorkspace.vue          ← 22:47 出现
 M src/components/chat/AgentRuntimeStatusBar.vue  ← 22:47 出现
 M src/lib/runtime/status.ts                 ← 22:47 出现
 M src/lib/runtime/types.ts
 M src/stores/runtime.ts
?? src/lib/chat/
?? src/stores/workspace-panels.ts
```

其中 `src/stores/runtime.ts` 的改动为 `sessionNotice` / `mode: "info"`（对应未跟踪的
`.dev-team/contract-matrix-slash-commands.md`，C-9 斜杠命令工作流），与 Wave-59 无因果关系。
在这类并发写域上跑红相会得到「被部分修复过」的不可信结论，故：

```
git archive d3745a7 | tar -x -C /tmp/w59-baseline     # 纯净基线（含 configs / package.json / tests）
ln -s /home/dm/pony-agent/node_modules /tmp/w59-baseline/node_modules
# 仅把本次新增/迁移的测试文件拷入快照，其余一律为 d3745a7 原件
```

**快照源码指纹（与契约基线一致，可复算）**：

| 文件 | md5（`/tmp/w59-baseline`） |
| :-- | :-- |
| `src/components/MarkdownRenderer.vue` | `0c76076fd5210a287e6e610e2cfde9db` |
| `src/components/chat/WorkspaceTurnItem.vue` | `c41db93d872aa4daa4ded8231a2b4147` |
| `src/components/HomeWorkspace.vue` | `4298f51ccfb25763a7cb72457b74e196` |
| `src/components/HomeStatusPanel.vue` | `f48908a3c1e3c84a89bf42cbde153188` |
| `src/stores/runtime.ts` | `ff397dc86503bb4e2dd44c4f1cb03747` |
| `src/lib/runtime/status.ts` | `4c3088761d96c841f209d4373731df58` |

### 1.2 夹具说明（避免把环境差异误判为缺陷）

- `HomeStatusPanel` 单测一律包 `reka-ui TooltipProvider`（与 wave-17 C2 同构），否则 `TooltipRoot` setup 抛错；
- jsdom 无布局：F3 的**像素级**指标只在 F3-5（真 Chromium）测量，单测只做 DOM 结构/节点身份；
- `motion-v` 指令在单测环境未注册，统一注入 no-op `motion` 指令桩，仅消除解析噪声；
- F3/F4 夹具把 `pony-agent.stream-render.release-chars` 置为 10000：释放节奏（D-59-2 保留）不属于本波
  契约对象，放大只为让「内容已到达展示层」确定成立，不参与任何断言。

### 1.3 工具链

- node `v22.21.0` / vitest `4.1.9` / jsdom
- @playwright/test `1.60.0`（chromium，`vite preview` @ `127.0.0.1:4175`，`dist` 由快照源码 `npx vite build` 现构建）
- 复现命令（两段即全部红相证据）：

```bash
cd /tmp/w59-baseline
npx vitest run tests/acceptance/wave59 tests/MarkdownRenderer.spec.ts \
  tests/HomeWorkspaceMarkdown.spec.ts tests/HomeWorkspace.spec.ts \
  tests/acceptance/wave17/tabular-nums-contract.spec.ts \
  tests/workspace-runtime-status-streaming.spec.ts --reporter=verbose
# → Test Files 8 failed | 1 passed (9) / Tests 21 failed | 92 passed | 7 skipped (120) / VITEST_EXIT=1

npx vite build && npx playwright test tests/e2e/wave59-stream-stability.spec.ts \
  tests/e2e/wave17-browser-gate.spec.ts --reporter=list
# → 2 failed / 3 passed / PLAYWRIGHT_EXIT=1
```

---

## 2. 新增验收用例：逐用例结果（红相）

`tests/acceptance/wave59/`
（`wave59-harness.ts` 为共享夹具，非 spec）

| 用例 | F 编号 | 基线结果 | 关键输出（原始摘录） |
| :-- | :-- | :-- | :-- |
| `wave59-f1-waiting-animation.spec.ts` F1-1 | F1-1 | **FAIL** | `F1-1：awaiting-first-signal 状态下不得渲染任何重复等待动画节点（实际：{"dot":3,"panel":1,"testid":1}）: expected { dot: 3, panel: 1, testid: 1 } to deeply equal { dot: +0, panel: +0, testid: +0 }` |
| 同上 F1-2 | F1-2 | PASS（§5.1） | 状态条 `data-visible="true"` + 标签 "思考中..." 已在基线成立 |
| 同上 F1-3 | F1-3 | **FAIL** | `F1-3：等待首个信号时 shouldShowAgentArticle 必须为 false —— 回合内只允许用户消息 article: expected [ …(2) ] to have a length of 1 but got 2` |
| `wave59-f2-status-metrics-layout.spec.ts` F2-1 | F2-1 | **FAIL** | `F2-1：头部行内不得再出现计数承载 span.inline-flex（契约要求计数下移，头部仅保留标题+复制按钮）: expected [ '9', '12', '345' ] to have a length of +0 but got 3` |
| 同上 F2-2 | F2-2 | **FAIL** | `F2-2：必须新增 [data-testid="status-metrics-counts"] 计数行（用于承接下移的三个计数位）: expected false to be true` |
| 同上 F2-3 | F2-3 | **FAIL** | `F2-3：status-metrics-counts 计数行必须存在（wave-17 C2 口径的定位锚点）: expected false to be true` |
| 同上 F2-4 | F2-4 | **FAIL** | `F2-4：status-metrics-counts 计数行必须存在: expected false to be true`（token 行 4 位本体未回归，失败点仅在计数行缺失） |
| `wave59-f3-streaming-dom-stability.spec.ts` F3-1 | F3-1 | **FAIL** | `F3-1：streaming 期间不得以独立节点呈现未渲染尾巴/逐字符；违规采样：[{"step":0,"raw":1,"suffix":0,"char":9},{"step":1,"raw":1,"suffix":0,"char":28},{"step":2,"raw":1,…"char":7},{"step":3,…"char":8},{"step":4,…"char":30},{"step":5,…"char":25}]` |
| 同上 F3-2a | F3-2a | **FAIL** | `F3-2a：streaming 首帧必须已有 .markdown-body 承载节点…: expected false to be true` |
| 同上 F3-2b | F3-2b | PASS（§5.2） | 基线在「首个渲染周期之后」承载节点身份本就稳定（缺陷不在该窗口） |
| 同上 F3-3a | F3-3a | **FAIL** | `F3-3a：streaming→完成态切换不得替换 .markdown-body 子树（DOM 引用必须一致）: expected <div data-v-0b0b9f63 …(1)>…(5)</div> to be <div data-v-0b0b9f63 …(1)>…(5)</div>` |
| 同上 F3-3b | F3-3b | **FAIL** | `F3-3b：流式期间正文必须由 .markdown-body 承载；否则完成态只能新建承载节点 ⇒ 子树被替换（RC5 闪跳）: expected false to be true` |
| 同上 F3-4 | F3-4 | **FAIL** | `F3-4：WorkspaceTurnItem 内容区不得出现 .assistant-streaming-char 逐字符节点（各采样点实际数量：[11,14,22]）: expected [ 11, 14, 22 ] to deeply equal [ +0, +0, +0 ]` |
| `wave59-f4-runtime-status-accuracy.spec.ts` F4-1 | F4-1 | **FAIL** | `F4-1：正文 delta 到达后必须切到 responding（实际：{"mode":"thinking","label":"思考中...","startTime":1793544574830}）: expected 'thinking' to be 'responding'`（RC6） |
| 同上 F4-2 | F4-2 | **FAIL** | `F4-2：工具后的新 hop（本 hop 尚无 reasoning/text delta）必须为 thinking 且状态条可见，实际：{"mode":"responding","label":"正在回复...","statusSlotVisible":"false"}`（RC7+RC8） |
| 同上 F4-3 | F4-3 | PASS（§5.3） | 正文流式时状态条隐藏（wave-14/优化3）在基线已成立 |
| 同上 F4-4 | F4-4 | PASS（§5.4） | 与既有 spec 三例同构的并行守卫，基线即绿 |
| 同上 F4-5 | F4-5 | **FAIL** | `F4-5：resolver 声明 retrying 能力（retryAttempt=2 → mode=retrying），但 store 侧无任何可触达信号源 ⇒ 不可达死代码。候选探针结果：[{"source":"messages.pending(TIMEOUT_RETRY_PENDING_MESSAGE)","mode":"responding"}]；retry 语义 state 键：[]`（RC9） |

### 2.1 F3-5（真浏览器 `tests/e2e/wave59-stream-stability.spec.ts`）

**FAIL**，exit 1（`PLAYWRIGHT_EXIT=1`）。四次独立运行的物理数据（同批采样，全部回撤 > 1px）：

| 运行 | 帧数 | 内容列回撤点数 | 最大回撤 | 正文容器回撤 | Console Error |
| :-- | --: | --: | --: | --: | --: |
| run#1 | 91 | 22 | **-47px** | 1（-3px） | 0 |
| run#2（retry） | 99 | 28 | **-56px** | 1（-3px） | 0 |
| run#3 | 72 | 19 | **-56px** | 1（-3px） | 0 |
| run#4 | 68 | 16 | **-40px** | 0 | 0 |

原始失败摘录（run#2，retry）：

```
Error: F3-5 ①：workspace-content-column scrollHeight 逐帧不得回撤 > 1px；回撤点：
[{"index":5,"from":408,"to":404,"delta":-4},{"index":6,"from":404,"to":402,"delta":-2},
{"index":9,"from":425,"to":402,"delta":-23},{"index":12,"from":473,"to":433,"delta":-40},
{"index":16,"from":481,"to":439,"delta":-42},{"index":46,"from":622,"to":575,"delta":-47},
{"index":83,"from":842,"to":786,"delta":-56}, …]（原始帧数据：/tmp/dsh-wave59-2026-11-01T14-48-35-446Z/frames.json）
Expected length: 0  Received length: 28
```

- ①（内容列 scrollHeight 单调）**红**；①'（agent 正文容器 scrollHeight 单调）**红**（3/4 次出现 -3px）。
- ②③④ 在本轮基线运行中**未判到**（①先失败）：离线复核 `frames.json` 得
  「首个非空正文帧后无空白帧（0 帧）」「Console Error = 0」，故 **②③ 在基线即为绿**，
  修复只需保证不回归；④ 截图已真实落盘
  （`/tmp/dsh-wave59-2026-11-01T14-48-35-446Z/{before-stream-mid.png,after-stream-final.png,frames.json}`）。
- 补充观察（非契约断言，供 executor 定位 RC1/RC2）：首个正文帧的可见文本长度序列为
  `4 → 1 → 1 → 1 → 4 → 4 …`，即**文本先出现再回退**，与「尾巴独立成块 + 逐字符 span 重建」一致。

---

## 3. 迁移既有断言的逐用例结果

| 文件 / 用例 | 迁移内容（仅定位锚点） | 基线结果 | 关键输出 |
| :-- | :-- | :-- | :-- |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` C2-1（计数位） | 头部 `status-panel-toggle span.inline-flex` → `[data-testid="status-metrics-counts"] span.inline-flex`；断言口径（3 位点 / ≤2 层祖先 tabular-nums / 数值 9·12·345）不变 | **FAIL**（预期） | `C2-1 锚点（wave-59 F2-2）：计数位已下移至 [data-testid="status-metrics-counts"] 行，该行必须存在: expected false to be true` |
| `tests/e2e/wave17-browser-gate.spec.ts` C2-2（真实 Chromium 物理宽度） | counts 行锚点同上迁移；C2-2a/b/c/d 阈值（0.5px / 单调不减）与口径全部不变 | **FAIL**（预期） | `Error: HomeStatusPanel 必须渲染 7 个指标位，实际 4 个；测得：["输入 token 位","输出 token 位","缓存 token 位","生成速度位"]` |
| `tests/HomeWorkspace.spec.ts` 「首个信号前」用例 | 「三点动画 3 个 + waiting 面板存在」→ F1-1（无 dot/panel/testid）+ F1-2（状态条可见 + 标签非空）；原状态迁移链（无 assistant → 空 pending → 工具到达）保留 | **FAIL**（预期） | `AssertionError: F1-1：等待首个信号期间不得再渲染 assistant-awaiting-first-signal 等待面板（重复指示）: expected true to be false` |
| `tests/HomeWorkspace.spec.ts` 「空 pending assistant」用例 | 「三点动画 3 个」→ F1-1（无 dot/panel/testid）；「无 reasoning 文本占位」原语义保留 | **FAIL**（预期） | `AssertionError: F1-1：空 pending assistant 不得渲染 assistant-awaiting-first-signal 等待面板: expected true to be false` |
| `tests/MarkdownRenderer.spec.ts` ×3（`keeps rendering…` / `renders markdown incrementally…` / `renders the streaming suffix…`） | 「`.mr-streaming-raw` / `.streaming-unrendered-suffix` / `.mr-streaming-char` 必须存在/逐字符内容」→ F3-1（三类节点数量必须 0）；「最终 `<strong>` / 全文可见」原语义保留 | **FAIL**（预期） | `expected { raw: 1, suffix: +0, char: 10 } to deeply equal { raw: +0, suffix: +0, char: +0 }`；`{ raw: 1, suffix: 0, char: 13 }`；`{ raw: 1, suffix: 0, char: 4 }` |
| `tests/HomeWorkspaceMarkdown.spec.ts`（逐字符释放） | 「`.assistant-streaming-char` 恰 4 个」→ F3-4（逐字符节点必须 0）；「正文按释放节奏逐步可见（展示层为完整内容前缀）+ 无 v-motion 包裹 + 无 fade-batch」原语义保留 | **FAIL**（预期） | `expected [ DOMWrapper{ …(3) }, …(3) ] to have a length of +0 but got 4` |

**迁移纪律核对**：`git diff` 只包含定位锚点与失败信息文案；wave17 C2 的 tabular-nums 物理口径、
C2-3/C2-4、HomeWorkspaceMarkdown 前两例、MarkdownRenderer 其余 8 例在基线**保持绿**
（本次红相 run：wave17 C2-1 token 位 / C2-3 ×2 / C2-4 ×2 全 PASS）。

---

## 4. F4-4 的一等证据（非本文件用例）

`tests/workspace-runtime-status-streaming.spec.ts` 三例（契约 F4-4 指定必须保持绿）在红相 run 中：

```
✓ does not show runtime status bar when assistant is streaming content
✓ shows runtime status bar when executing tool before streaming content
✓ shows runtime status bar when tool is drafting even if previous assistant message has pending status
```

---

## 5. 基线即通过（未伪造失败）——逐条原因

> 依据任务要求：若某条在当前实现下意外通过，必须在收据中说明原因并显式标注。

**5.1 F1-2（状态条可见 + 标签非空）**：基线 `isRuntimeStatusVisible` 在「无工具活动、最后回合无
assistant 正文」时返回 `true`，且 `resolveAgentRuntimeStatus` 在 `calling_model` + 无流式内容时已给出
`thinking/"思考中..."`。故 F1-2 在基线即成立，它的作用变为**回归守卫**：executor 删掉三点动画后，
必须保证「唯一指示」仍由状态条承载（若把等待指示一起删掉，F1-2 会转红）。

**5.2 F3-2b（增长过程承载节点身份不变）**：基线只在「内容变短」时才清空 `renderedHtml`（
`MarkdownRenderer.vue:245-250`），纯增长路径下 `.markdown-body` 元素**在首个渲染周期之后**
确实不被替换。因此该条绿；真正的身份破坏发生在**首帧缺失**（F3-2a）与**流式→完成切换**
（F3-3a/F3-3b）。保留 F3-2b 作为防回归守卫（修复方案若改用 `v-if` 重建承载节点，会在此转红）。

**5.3 F4-3（正文流式 → `data-visible="false"`）**：`isRuntimeStatusVisible` 的 wave-14/优化3 判据
（末回合 assistant `pending` 且有可见正文 → 隐藏）在基线即成立。该条为**回归守卫**：
F4-1/F4-2 修好后仍必须保持该隐藏行为。自检锚点用契约级语义（回合 section 文本含正文），
不绑定内部类名；为使「正文已可见」前提确定成立，夹具把
`pony-agent.stream-render.release-chars` 放大到 10000（释放节奏不属本波范围，D-59-2 保留）。

**5.4 F4-4（wave-14 三例并行守卫）**：与既有 spec 同构，基线即绿（§4 一等证据）。

**5.5 F3-5 ②③**：见 §2.1（①先失败，②③ 离线复核为绿）。

---

## 6. 契约歧义与 Lead 裁定（已裁定，测试口径冻结）

> 裁定时间：2026-11-01（Lead 一票裁决）。

### 6.1 F3-5 ② 首帧空白口径 —— **Lead 裁定：采纳「收窄读法」**（原字面口径否决）

争议：契约 F3-5 ②「任一帧在 content 长度 > 0 时文本不得为空」的字面读法，与 D-59-1
「stale-while-render：文字允许滞后 ≤ 1 个 render 周期」冲突 —— 内容 patch 后的第一帧（16ms 窗口）
在正确实现下也可能还没有可呈现的 HTML；若要字面口径，实现必须保留文本回退节点，与 F3-1 直接冲突。

**Lead 裁定（已生效，测试按此冻结）**：
- 从「正文首次非空帧」起不得再回到空白（无「先清空再重绘」）；
- 「首个内容帧 → 首个非空正文帧」延迟 ≤ 4 帧（`FIRST_TEXT_MAX_FRAME_LAG`，≈66ms，覆盖 16ms render + 解析）；
- 实现**不需要**为 F3-5 ② 保留文本回退节点（F3-1 优先）。

### 6.2 F3-2a 「streaming 首帧起承载节点必须已存在」—— **Lead 裁定：保持严格读法，不授权调整**

契约原文只说「身份不变（不得重建）」。本测试按「整段 streaming 期间身份不变 ⇒ 首帧即须存在」的
严格读法断言：若首帧无承载节点，完成/渲染时必然新建节点，身份不可能全程不变；
与 D-59-1 兼容（节点常驻、内容可为空）。

**Lead 裁定（已生效）**：测试口径正确、**不授权我调整**；由 executor 把承载容器改为
**无条件常驻**（`v-html` 内容可为空），这是契约本该有的形态。executor 只读测试。

### 6.3 F4-5 双路线判定

契约允许「接入真实重试信号」**或**「删除分支并登记 N-A」。冻结测试实现为**双分支可执行契约**：
- 路线 A（resolver 仍对 `retryAttempt>0` 返回 `retrying`）⇒ 必须存在 store 侧真实可触达信号源
  （`$state` 中 `/retry/i` 语义键被置位后 `agentRuntimeStatus.mode === "retrying"`，**或**
  真实超时重试路径 `TIMEOUT_RETRY_PENDING_MESSAGE` 的 pending assistant），且 `turn:started` 回
  `calling_model` 后清零；
- 路线 B（分支已删）⇒ 任何 `retryAttempt` 均不得再返回 `retrying`。
基线失败信息显示当前 store 两条候选信号源均不可触达（RC9 死代码）。

---

## 7. 交付文件清单

新增（test-agent 独有写权）：

- `tests/acceptance/wave59/wave59-harness.ts`（共享夹具，非 spec）
- `tests/acceptance/wave59/wave59-f1-waiting-animation.spec.ts`（F1-1/1-2/1-3）
- `tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts`（F2-1..F2-4）
- `tests/acceptance/wave59/wave59-f3-streaming-dom-stability.spec.ts`（F3-1/3-2a/3-2b/3-3a/3-3b/3-4）
- `tests/acceptance/wave59/wave59-f4-runtime-status-accuracy.spec.ts`（F4-1..F4-5）
- `tests/e2e/wave59-stream-stability.spec.ts`（F3-5，真浏览器逐帧采样 + 前后截图 + 帧数据落盘）

迁移（仅定位锚点）：

- `tests/acceptance/wave17/tabular-nums-contract.spec.ts`（C2-1 计数位锚点 → `status-metrics-counts`）
- `tests/e2e/wave17-browser-gate.spec.ts`（C2-2 counts 行锚点同上）
- `tests/HomeWorkspace.spec.ts`（等待三点 → F1-1/F1-2，两处）
- `tests/MarkdownRenderer.spec.ts`（尾巴/逐字符节点 → F3-1，三处）
- `tests/HomeWorkspaceMarkdown.spec.ts`（逐字符节点 → F3-4，一处）

未修改：`src/**`（零改动）、`.dev-team/contracts/wave-59-desktop-uiux.md`（零改动）。

中间产物留删：本收据只登记**证据**（上述文件 + `frames.json` 路径引用）；
过程类日志（`/tmp/w59-red-vitest-final.log`、`/tmp/w59-red-e2e-final.log`）与
`/tmp/w59-baseline` 快照、`/tmp/dsh-wave59-*` 截图目录均在仓库外，不入仓。
