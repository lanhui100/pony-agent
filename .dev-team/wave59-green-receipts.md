# Wave-59 绿相独立验证收据（L2-T 独立测试者，只读 src / 不改测试 / 不改实现）

契约源（只读）：`.dev-team/contracts/wave-59-desktop-uiux.md`
红相收据：`.dev-team/wave59-red-receipts.md`
验证时间：2026-11-01 22:58 – 23:13（Asia/Shanghai）
验证人：test-agent（与被测实现作者分权，未修改任何 `src/**` 与测试文件）

---

## 0. 结论先行

| # | 命令 | exit | 结果 |
| :-- | :-- | --: | :-- |
| 1 | `npm run typecheck` | **0** | vue-tsc 无错 |
| 2 | `npx vitest run`（全量） | **1** | 119 files：118 passed / 1 failed；Tests **1131 passed / 1 failed / 12 skipped（1144）** |
| 3 | `npm run build` | **0** | vue-tsc + vite build 成功（e2e 用 dist 现构建） |
| 4 | `npx playwright test tests/e2e/wave59-stream-stability.spec.ts`（F3-5） | **0** | 1 passed（8.2s） |
| 附 | `tests/acceptance/wave59` 单独跑 | **0** | **18 passed（18）** |

**判定**：Wave-59 四条契约（F1/F2/F3/F4）在冻结测试下**全部转绿**；F3-5 真浏览器逐帧采样 **0 回撤 / 0 空白帧 / 0 Console Error**。
全量 vitest 唯一失败为 `wave47-ob2-enforcement.spec.ts > T7-PONYGO-OB2-GATE`，根因是**并发波次（wave-slash-1）改动了 `docs/observability.yaml` 声明而变更门禁未获 `--decl-approved`**（工作区脏树状态门禁，非本波代码缺陷；同一用例在干净基线上绿，证据见 §6.1）。

---

## 1. 环境与源码溯源（钉版：md5 门禁前后一致）

- `HEAD = d3745a7`（工作区含本波 + 并发波次未提交改动）
- 工具链：node `v22.21.0` / vitest `4.1.9` / @playwright/test `1.60.0`（chromium，`vite preview` @127.0.0.1:4175）

**钉版 gate 链（§0 的 1–4 为同一次连续执行，开始/结束 md5 完全一致 → 该证据对应同一份源码）**：

| 文件 | md5（gate 链开始 = 结束） |
| :-- | :-- |
| `src/components/MarkdownRenderer.vue` | `72e6186dc46f23c286d836605b21d364` |
| `src/components/HomeWorkspace.vue` | `57424eba6a3926507be09f3a90548700` |
| `src/components/chat/WorkspaceTurnItem.vue` | `a5888850ee6db3e47b5b8fcb45230875` |
| `src/components/chat/AgentRuntimeStatusBar.vue` | `e35022d82bd2dff9dddf1a7a9a2d4dbe` |
| `src/components/HomeStatusPanel.vue` | `cf4bb5e16643aae57ea6b87ece4b94aa` |
| `src/stores/runtime.ts` | `34fcf9aa300dd83fc8024ed4df6edc5f` |
| `src/lib/runtime/status.ts` | `c6887039acac443a7da56b76fd0924ed` |
| `src/lib/runtime/types.ts` | `b3788cefc94096bf77efbecd8ef71643` |

`diff pin-start pin-end` → 空（PIN UNCHANGED）。

**⚠ 采集期间源码仍在变动（诚实披露）**：`MarkdownRenderer.vue` 于 **22:59:13**、`HomeWorkspace.vue` 于 **23:08:07**
被 executor/并发波次继续修改（红相采集期 22:48–22:50 亦有一次）。因此：

- **22:58–23:08 期间的 3 次全量 vitest 与 3 次 e2e 运行标记为「非钉版（同源但跨版本）」**，仅作趋势参考；
- **§0 的 4 条 gate 与 §5 的 F3-5 权威数据取自 23:10:30–23:12:30 的钉版链**（期间 md5 无变化）。

---

## 2. 逐 F 编号 PASS 表（钉版全量 vitest 输出）

| F 编号 | 结果 | spec 文件 : 用例名 |
| :-- | :-- | :-- |
| F1-1 | **PASS** | `tests/acceptance/wave59/wave59-f1-waiting-animation.spec.ts` : *F1-1: awaiting-first-signal 下不得存在 .assistant-waiting-dot / .assistant-waiting-panel / assistant-awaiting-first-signal 节点* |
| F1-2 | **PASS** | 同上 : *F1-2: 同一状态下底部状态条必须可见且标签非空（承载「思考中/准备就绪中」唯一指示）* |
| F1-3 | **PASS** | 同上 : *F1-3: awaiting-first-signal 时不得残留空 agent 壳（无 agent 文章容器、无 workspace-agent-actions）* |
| F2-1 | **PASS** | `wave59-f2-status-metrics-layout.spec.ts` : *F2-1: 头部行不得再承载计数位，且必须保留「状态」标题与复制 Session ID 按钮* |
| F2-2 | **PASS** | 同上 : *F2-2: 新增 status-metrics-counts 行，位于 token 卡内、token 行之上，顺序为 轮次→模型调用→工具调用* |
| F2-3 | **PASS** | 同上 : *F2-3: 三个计数位自身或 ≤2 层祖先必须含 tabular-nums（wave-17 C2 口径的新锚点）* |
| F2-4 | **PASS** | 同上 : *F2-4: token 行结构不回归（4 个直接 span.inline-flex），且计数行不得使用 justify-around* |
| F3-1 | **PASS** | `wave59-f3-streaming-dom-stability.spec.ts` : *F3-1: streaming 期间不得出现 .mr-streaming-raw / .streaming-unrendered-suffix / .mr-streaming-char 尾巴节点* |
| F3-2a | **PASS** | 同上 : *F3-2a: streaming 首个渲染帧起，承载节点 .markdown-body 必须已存在（stale-while-render 前提）* |
| F3-2b | **PASS** | 同上 : *F3-2b: 内容增长过程中 .markdown-body 承载节点身份不变（不得重建）* |
| F3-3a | **PASS** | 同上 : *F3-3a: MarkdownRenderer streaming→完成态切换，.markdown-body 身份不变且完成帧含完整内容* |
| F3-3b | **PASS** | 同上 : *F3-3b: 生产路径（HomeWorkspace）streaming→完成态切换，正文承载节点 .markdown-body 身份不变且完成帧含完整内容* |
| F3-4 | **PASS** | 同上 : *F3-4: WorkspaceTurnItem 内容区不得出现 .assistant-streaming-char 逐字符节点，流式与完成态共用同一容器* |
| F3-5 | **PASS** | `tests/e2e/wave59-stream-stability.spec.ts` : *合成 markdown 长流下：滚动高度单调不减、无空白帧、Console Error = 0*（数据见 §5） |
| F4-1 | **PASS** | `wave59-f4-runtime-status-accuracy.spec.ts` : *F4-1: hop 内 reasoning delta 后 mode=thinking，紧随 text delta 后必须切到 mode=responding* |
| F4-2 | **PASS** | 同上 : *F4-2: 工具事件后的新 hop（尚无 reasoning/text delta）→ mode=thinking 且状态条 data-visible=true* |
| F4-3 | **PASS** | 同上 : *F4-3: 正文流式（当前 hop 有 text delta 且正文可见）→ 状态条 data-visible=false（沿用优化3）* |
| F4-4 | **PASS** | 同上 : *F4-4: 既有 wave-14 三例可见性语义保持（正文流式隐藏 / 工具执行可见 / 工具草稿可见）*；另 `tests/workspace-runtime-status-streaming.spec.ts` 三例同轮全绿 |
| F4-5 | **PASS** | 同上 : *F4-5: retrying 不得是冒充能力的不可达分支（要么 store 真实可触达，要么已显式删除）* |

**迁移用例（绿相，钉版全量输出）**

| 文件 : 用例 | 结果 |
| :-- | :-- |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` : *C2-1 … 三个计数位承载元素必须含 tabular-nums*（锚点已迁至 `status-metrics-counts`） | **PASS** |
| 同文件 : *C2-1 … 四个 token 指标位* / *C2-3 ×2* / *C2-4 ×2* | **PASS**（口径未动，无回归） |
| `tests/HomeWorkspace.spec.ts` : *shows the status-bar waiting indication (not jumping dots) before the first assistant signal arrives* | **PASS** |
| `tests/HomeWorkspace.spec.ts` : *shows no first-signal waiting animation for an empty pending assistant (and no reasoning placeholder)* | **PASS** |
| `tests/MarkdownRenderer.spec.ts` : *keeps rendering new markdown content while forceMarkdownStreaming is active* / *renders markdown incrementally while forceMarkdownStreaming is active* / *renders the streaming suffix without any separate unrendered-suffix node (F3-1)* | **PASS** |
| `tests/HomeWorkspaceMarkdown.spec.ts` : *reveals streaming assistant content with continuous release (no per-character nodes, F3-4)* | **PASS** |
| `tests/e2e/wave17-browser-gate.spec.ts` : *C2-2（真实 Chromium 物理宽度，锚点已迁）* | **FAIL（基线原版同红，非本波引入）** —— 见 §6.2 |

---

## 3. 命令原文与 exit code（钉版链，可复算）

```bash
cd /home/dm/pony-agent
npm run typecheck                                   # TYPECHECK_EXIT=0
npm run build                                       # BUILD_EXIT=0
npx vitest run                                      # VITEST_EXIT=1
#  → Test Files  1 failed | 118 passed (119)
#  → Tests  1 failed | 1131 passed | 12 skipped (1144)
npx playwright test tests/e2e/wave59-stream-stability.spec.ts
#  → 1 passed (8.2s)                              # PW_EXIT=0
npx vitest run tests/acceptance/wave59              # 18 passed (18), exit 0
```

非钉版参考运行（22:58–23:08，源码在同窗口内被改动，故不作为判据）：

| 轮次 | 时间 | files | Tests | exit | 失败项 |
| :-- | :-- | :-- | :-- | --: | :-- |
| 全量#1 | 22:59–23:02 | 2 failed/117 | 2 failed / 1125 passed / 12 skipped (1139) | 1 | ob2 T7 + `markdown-streaming-smooth`（flaky，见 §6.3） |
| 全量#2 | 23:03–23:05 | 1 failed/118 | 1 failed / 1126 passed / 12 skipped (1139) | 1 | ob2 T7 |
| 全量#3 | 23:06–23:08 | 1 failed/118 | 1 failed / 1129 passed / 12 skipped (1142) | 1 | ob2 T7 |
| e2e ×3 | 23:04:20 / 23:04:51 / 23:05:37 | — | 各 1 passed | 0 | — |

---

## 4. 全量 vitest 与基线差异表

基线口径：`.dev-team/regression-baseline-wave59.json`（`command: npx vitest run`，114 files / 1100 passed / 12 skipped，exit 0）。
为得到**逐用例名字级差异**，本 agent 在 `git worktree add /tmp/w59-base-wt d3745a7` 的**干净基线**上重跑了全量：

```
base:    Test Files 114 passed (114)   Tests 1100 passed | 12 skipped (1112)   BASE_VITEST_EXIT=0   ← 与基线 json 完全一致
current: Test Files 1 failed | 118 passed (119)   Tests 1131 passed | 1 failed | 12 skipped (1144)
```

| 差异类别 | 数量 | 明细 |
| :-- | --: | :-- |
| 新增测试文件 | +5 | 4 个 `tests/acceptance/wave59/*.spec.ts` + 1 个 `tests/slash-commands.spec.ts`（**并发波次 wave-slash-1**）；`tests/e2e/wave59-stream-stability.spec.ts` 不入 vitest 收集 |
| 新增用例 | +32 | wave59 **+18**；`slash-commands.spec.ts` **+14**（并发波次，其中 3 条 W-1..W-3 与 2 条 T7 用例在本次验证窗口内被追加，故全量总数 1139→1142→1144 漂移，非本波引起） |
| 用例改名（迁移，非新增覆盖） | 4 | HomeWorkspace ×2、HomeWorkspaceMarkdown ×1、MarkdownRenderer ×1（另 2 条 MarkdownRenderer 用例名未变、仅断言迁移，故不出现在名字 diff 中） |
| skipped | 0（不变） | 12 → 12，无新增 skip、无 `it.skip` 新增 |
| **新增失败** | **1（确定性）+ 1（并发负载 flake，1/4 次）** | ① `wave47-ob2-enforcement > T7-PONYGO-OB2-GATE`（§6.1）；② `markdown-streaming-smooth > renders streaming partial markdown smoothly…`（§6.3） |
| 新增通过 | +32 | 上述 +18 / +14 全部通过；4 条迁移改名用例全部通过 |

**名字级 diff 结论**：除上述 4 条改名与 32 条新增外，**没有**任何既有用例从 PASS 变 SKIP，也没有其他 PASS→FAIL。

---

## 5. F3-5 采样数据摘要（权威 = 钉版运行）

截图/帧数据落盘目录（**绝对路径**）：`/tmp/dsh-wave59-2026-11-01T15-11-35-074Z/`
（`before-stream-mid.png` 97 477 B、`after-stream-final.png` 127 717 B、`frames.json` 19 556 B）

| 指标 | 钉版值 | 契约阈值 | 结论 |
| :-- | --: | :-- | :-- |
| 帧数 / 采样跨度 / 中位帧间隔 | 102 / 1935.8ms / 18.3ms | — | 采样密度充足（≈55fps） |
| `workspace-content-column` scrollHeight 最大回撤 | **0px**（>1px 回撤点 0 个） | ≤1px | **PASS** |
| agent 正文容器 scrollHeight 最大回撤 | **0px**（0 个） | ≤1px | **PASS** |
| 空白帧（首个非空正文帧之后、content>0 且文本为空） | **0** | 0 | **PASS** |
| 首个内容帧 → 首个非空正文帧 滞后 | **0 帧** | ≤4 帧 | **PASS** |
| Console Error / pageerror 条数 | **0** | 0 | **PASS** |
| 终态内容长度 / 终态可见文本长度 | 410 / 284 字符 | 应含完整内容 | **PASS** |
| 列 scrollHeight 区间 | 351 → 885px 单调 | — | 单调不减 |

非钉版补充运行（同实现族，源码在窗口内被微调）：`…15-04-20-287Z` 74 帧（中位 28ms）、`…15-04-51-050Z` 26 帧（rAF 被节流，中位 129ms）、`…15-05-37-890Z` 41 帧（中位 49.8ms）—— **均为 0 回撤 / 0 空白帧 / 0 Console Error**。

---

## 6. 失败/异常条目明细（含并发波次归因）

### 6.1 【确定性】`tests/acceptance/wave47-ob2-enforcement.spec.ts > T7-PONYGO-OB2-GATE`（判为**并发波次 + 脏树状态门禁**，非 Wave-59 缺陷）

原始声明：

```
FAIL tests/acceptance/wave47-ob2-enforcement.spec.ts > ob2 变更级门禁与横切收口点验收测试 (L2-T & L2-AT)
     > T7-PONYGO-OB2-GATE: ponygo observe audit --level ob2 物理门禁为绿相 (Exit 0)
AssertionError: expected 1 to be +0 // Object.is equality
```

手工复跑 `ponygo observe audit --level ob2`（exit=1）的 stderr 原文：

```
[PASS] ob0 归属声明
[PASS] ob1 声明与现实一致
[FAIL] ob2 收口与变更门禁
[FAIL] [D1] 声明在本次变更中被修改，需要显式批准：请用 --decl-approved 或在 commit message 加
       [decl-change-approved]（防同一次提交削弱规则集自证清白）
[info] 扫描 58 个变更文件 / 5230 行新增
当前级: ob1  目标级: ob2   结论: FAIL（exit=1）
```

**归因证据**：
1. 触发项是「声明文件被改动且未显式批准」，不是代码违规；`git diff docs/observability.yaml` 的改动全部来自
   **wave-slash-1（C-8/C-9 斜杠命令）** 的归属登记段落（`【wave-slash-1 归属登记（2026-11-01）｜斜杠命令 /compact…】`
   与 `reviewed_at: 2026-10-27 → 2026-11-01`），不涉及本波 F1–F4 的任何 DOM/状态改动；
2. 同一用例在**干净基线 worktree（d3745a7）**上的全量运行**通过**（`114 files / 1100 passed / 12 skipped，exit 0`）——
   即该失败由「未提交的声明变更」引起，提交时带 `[decl-change-approved]` 或审核通过后即恢复绿；
3. 与本波因果无关（本波 D-59-4 只要求 `docs/observability.yaml` 登记 N-A 结构性理由，不新增接口/网络 IO）；
4. **交叉印证**：executor 在**开工前**（2026-11-01T22:48:06）录制的全量快照
   `.dev-team/wave59-preexisting-failures.txt` 已把同一用例 `T7-PONYGO-OB2-GATE` 列为「并发波次既有问题/红相锚定，
   记录不修」，即本项在 Wave-59 实现开始之前即为红。

→ **处置建议（Lead 决定）**：属工作区状态项，不作为 Wave-59 绿相阻塞项；若要求全量 exit 0，需在提交信息加
`[decl-change-approved]` 或对 wave-slash-1 的声明变更走显式批准。

### 6.2 【确定性，但为既有债务】`tests/e2e/wave17-browser-gate.spec.ts > C2-2`（迁移后仍红；**基线原版同红**）

迁移后（当前树）原始声明：

```
Error: 至少 6 个指标位必须直接渲染数字文本（防探针降级掩盖定位失效）；实际 3 个：
["轮次计数位=real-text","模型调用计数位=real-text","工具调用计数位=real-text",
 "输入 token 位=probe-node","输出 token 位=probe-node","缓存 token 位=probe-node","生成速度位=probe-node"]
```

**基线对照**：在干净 worktree（`d3745a7`，未迁移的**原版** C2-2）上执行同一命令：

```
npx vite build && npx playwright test tests/e2e/wave17-browser-gate.spec.ts -g "C2-2"
→ 1 failed / 2 次（含 retry #1）  BASE_C22_EXIT=1
→ Error: 至少 6 个指标位必须直接渲染数字文本…；实际 3 个：[…同一条消息…]
```

**结论**：C2-2 的红是**基线既有债务**（fresh page 无会话/无 token 数据 → 4 个 token 位渲染 `—` 走
`[RED DEGRADED: PROBE-NODE]`，而 Assert 2 要求 ≥6 个位点直出数字，环境上不可满足），
**与本次锚点迁移及 Wave-59 实现无关**（迁移前后失败消息逐字相同）。
另注：`tests/e2e/wave17-browser-gate.spec.ts` 文件头自述「基线 `npm run test:e2e` 即 8 failed / 12 passed」，
本项属该既有清单。**不自行改测试**，如需闭环请 Lead 授权（例如 e2e 夹具先注入 token 数据）。

### 6.3 【非确定性，1/4 次】`tests/acceptance/markdown-streaming-smooth.spec.ts > renders streaming partial markdown smoothly without raw unrendered-suffix jitter`（判为**并发负载下的时序 flake**）

原始声明（全量#1 唯一一次）：

```
FAIL tests/acceptance/markdown-streaming-smooth.spec.ts > Markdown Bold & Streaming Smooth Acceptance
     > 3. Streaming Markdown Smooth Rendering without Jitter / Raw Exposure
     > renders streaming partial markdown smoothly without raw unrendered-suffix jitter
AssertionError: expected '<!--\n    Wave-59 F3-2/F3-3：单一稳定根 + 唯…' to contain
                '<strong>“建筑工地的砌砖工”</strong>'
```

**复现谱系（同一实现）**：

| 场景 | 结果 |
| :-- | :-- |
| 单文件单跑 ×5 | 6/6 passed（5 次全绿） |
| 单文件 4 进程并发 ×1 轮 | 4/4 全绿 |
| 全量 #1 / #2 / #3 / 钉版 | **1 失败** / 绿 / 绿 / 绿 |

**机理核查（只读源码 + 独立复现，未改测试）**：该用例在 `setProps` 后只等待 60ms 就断言
`wrapper.html()` 含 `<strong>`。独立复现（单独 spec，用后已删）显示：60ms 时
`.markdown-body` 内 HTML **已含** `<strong>“建筑工地的砌砖工”</strong>`，`wrapper.html()` 长度 448 且包含该串 ——
说明渲染本体正确，失败只发生在全量并发（多 worker 抢 CPU）时 16ms 排程 + 异步解析合计超过 60ms 等待窗口。
与 Wave-59 的 stale-while-render（首次渲染在渲染周期内落地）**可能**使裕度变窄，但**不是确定性回归**。
→ **处置建议（Lead）**：该文件不在本 agent 写权范围内，未改动；若要消除 flake，需测试所有者把 60ms 盲等
改为显式条件轮询（`vi.waitFor`）——按纪律须经 Lead 授权，不得由我或 executor 私自改。

---

## 7. 环境清理声明

| 项 | 状态 |
| :-- | :-- |
| 4175 预览服务（playwright webServer `vite preview`） | **已退出**：`ss -ltnp \| grep :4175` 无监听；`curl 127.0.0.1:4175` → 000；`pgrep -af "[v]ite preview"` 无进程 |
| 基线 worktree `/tmp/w59-base-wt` | **已移除**（`git worktree remove --force`）；`git worktree list` 仅剩 `/home/dm/pony-agent d3745a7 [main]` |
| 红相快照 `/tmp/w59-baseline`、`/tmp/w59-base-full` | 仓库外临时目录，未入仓（仅本收据与红相收据引用其路径） |
| 诊断用临时 spec（`tests/acceptance/wave59/_tmp-debug.spec.ts`） | **已删除**（用后即删，`ls tests/acceptance/wave59/` 仅剩 4 个冻结 spec + harness） |
| 截图/帧数据 | 证据类，落 `${TMPDIR}`（`/tmp/dsh-wave59-*`），路径已在 §5 登记；仓库内无新增截图 |
| 仓库写入范围 | 仅 `.dev-team/wave59-green-receipts.md`（本文件）+ 红相收据 §6 裁定登记；**`src/**` 零改动、契约矩阵零改动、测试文件零改动** |

---

## 8. 纪律声明

- 本 agent 全程**只读 `src/**`**，未执行任何写操作；`src` 的改动均来自 executor / 并发波次（§1 已给 md5 钉版与变动时间）。
- **未修改任何测试文件**（冻结测试自红相起保持一致；本轮唯一改动是红相收据 §6 的 Lead 裁定登记）。
- 失败项均**原文照录**，无不吞异常、无 skip、无放宽阈值；两处未修复项（§6.1 / §6.2）已给出归因与基线对照，交 Lead 裁定。


---

## 9. §追加裁定（Lead 追加授权 A/B 落地，2026-11-01 23:13–23:18）

> 授权来源：Lead 追加裁定 A（`markdown-streaming-smooth.spec.ts` 盲等 → 条件等待）、
> B（`wave17-browser-gate.spec.ts` C2-2 夹具数据注入）。
> 纪律：**只改等待方式 / 只加夹具数据**，断言语义与阈值零改动（`git diff` 对断言行与阈值行的匹配结果为 0 处）。
> 本段改动文件 md5：`tests/acceptance/markdown-streaming-smooth.spec.ts` = `2d1f1e1ecfaca236e4ee8a035d9810d5`、
> `tests/e2e/wave17-browser-gate.spec.ts` = `4f605796dcfc1c68fcfc0b448c8dc678`；`src/**` 未改动（MarkdownRenderer `72e6186d…`、HomeWorkspace `57424eba…` 与 §1 钉版一致）。

### 9.1 A：`markdown-streaming-smooth.spec.ts` 盲等 60ms → 条件等待

- 变更：删除 `await new Promise(r => setTimeout(r, 60))`，改为
  `await vi.waitFor(() => 等待 MarkdownRenderer 以 contentLength=<本次内容长度> 且 streaming=true 发出 render-complete, { timeout: 5000, interval: 20 })`，
  超时消息给出已收到的事件列表。等待条件取自**组件公开事件契约**，不依赖断言的期望字符串；三条断言
  （`not.toContain("**“工地的总监理”**")` / 两条 `toContain("<strong>…</strong>")`）逐字未动。

| 阶段 | 命令 | exit | 结果 |
| :-- | :-- | --: | :-- |
| **before** | `npx vitest run tests/acceptance/markdown-streaming-smooth.spec.ts` ×5 | **0** | 6/6 passed（但为盲等 60ms） |
| **before** | `npx vitest run`（全量 #1 / #2 / #3 / 钉版） | **1 / 1 / 1 / 1** | #1 该用例红（flake），#2/#3/钉版仅 ob2 红 |
| **after** | `npx vitest run tests/acceptance/markdown-streaming-smooth.spec.ts` ×3（连跑） | **0 / 0 / 0** | 每次 6/6 passed |
| **after** | `npx vitest run`（全量 1 次） | **1** | Tests **1131 passed / 1 failed / 12 skipped（1144）**，唯一失败仍为 `wave47-ob2-enforcement > T7`（并发波次既有项，§6.1）；本用例绿 |

### 9.2 B：C2-2 夹具数据注入

- 变更：新增 `seedTokenMetricFixture(page)`（patch pinia runtime store 的一条 completed turn：`traceTimeline` 的
  `call_model` 条目 + `providerCallRecords[0]`（input 1234 / output 567 / cacheHit 89 / ttft 321 / dur 4200）
  + 轮级 `inputTokens/outputTokens/cacheHitInputTokens`），并在 `openHome` 之后、测量之前调用；
  另加 `page.waitForFunction` 条件等待「四个 token 位确实渲染出数字」。
  **未改** Assert 1（7 个位点）/ Assert 2（≥6 个 real-text）/ C2-2a `fontVariantNumeric` / C2-2b 等位数 ≤0.5px /
  C2-2c 相邻位左边缘 ≤0.5px / C2-2d 单调不减 —— 全部原样。

| 阶段 | 命令 | exit | 结果 |
| :-- | :-- | --: | :-- |
| **before** | `npx playwright test tests/e2e/wave17-browser-gate.spec.ts -g "C2-2"`（迁移后，无注入） | **1** | 1 failed/2 attempts（含 retry）；`至少 6 个指标位必须直接渲染数字文本…实际 3 个` |
| **before（基线原版）** | 干净 worktree `d3745a7` 上同一命令（未迁移版式） | **1** | 同一条断言、同一条消息 ⇒ 既有 e2e 债务 |
| **after** | 同一命令 ×2（连跑） | **0 / 0** | 每次 `1 passed`（1.5s / 1.3s）；7 个位点全部 real-text，C2-2a/b/c/d 在**新锚点 `status-metrics-counts`** 下全部通过 |

**结论**：注入后**环境可达**，C2-2 由既有 e2e 债务转为**绿**，F2-3 的物理口径（位数变化不推移相邻位点、等位数宽度稳定）
已在真实 Chromium + 新锚点下取得物理证据；原「环境不可达」判定**已被本夹具推翻**（记录在案，避免后续误登记为债务）。

### 9.3 环境确认（只读）

| 项 | 结果 |
| :-- | :-- |
| `git worktree list` | 仅 `/home/dm/pony-agent d3745a7 [main]` —— **无残留 worktree** |
| `/tmp/dsh-wave59-*` 截图目录 | **9 个目录仍在**（红相 4 个 14:48–14:49、绿相 4 个 15:04–15:11、另有 15:14:41 一个由并发的 Lead 侧 e2e 运行产生，数据同样 0 回撤/0 空白帧/0 Console Error）；权威目录：`/tmp/dsh-wave59-2026-11-01T15-11-35-074Z/` |
| 4175 端口 | **本 agent 两次 B 运行结束后均为无监听**（`ss -ltnp \| grep 4175` 空、无 `vite preview` 进程）；**收尾复核时（23:17:42）另有一处并发 e2e 运行遗留的 `vite preview --port 4175`（pid 373298/373299，started 23:17:19，无对应 playwright 进程）** —— 非本 agent 启动，未擅自终止（避免打断并发运行），已在此登记 |
| 采集期插曲（诚实披露） | 23:14:50 B 首次执行报 `http://127.0.0.1:4175 is already used` —— 端口被**并发的另一次 playwright 运行**占用的 `vite preview`（pid 347519/347535/347536，started 23:15:01）残留占用；确认无活跃 playwright 后清理该孤儿进程并重跑，之后两次执行与非钉版/钉版权威运行均正常收尾 |


---

## 10. §审查裁定补测（L3 两路独立审查 → 6 处补/改，2026-11-02）

> 来源：`.dev-team/review-join-wave59.json`（1 BLOCKING + 2 ADVISORY）、`.dev-team/review-plan-wave59.json`（7 ADVISORY）、
> 契约 §7 裁定（B-59-1 / A-59-1 / A-59-2 / D-59-7 / D-59-8 / D-59-9 / A-59-3 / A-59-4）。
> 纪律：**只改 `tests/**`**，`src/**` 零改动（本段钉版 md5：`MarkdownRenderer.vue` `bbe956fb…`、
> `HomeWorkspace.vue` `5132c8af…`、`stores/runtime.ts` `a8fa9c32…`）。

### 10.1 新增 / 修改用例清单

| 裁定 | 类型 | 文件 | 用例 / 变更 |
| :-- | :-- | :-- | :-- |
| **B-59-1（BLOCKING）** | 新增 F1-4 | `tests/acceptance/wave59/wave59-f1-waiting-animation.spec.ts` | *F1-4: 上一轮 content 流式结束后立即 submitTurn（尚未收到 turn:started）窗口内，状态条必须可见且 mode∈{connecting,thinking}* —— 同一 store **真实事件**跑完一整轮 content（`turn:started→turn:delta→turn:completed`）后调**真实 `submitTurn`**，不派发 `turn:started`；断言 `mode ∈ {connecting,thinking}` 且 `data-visible="true"` |
| **D-59-8** | 扩 F3-5 | `tests/e2e/wave59-stream-stability.spec.ts` | 合成流终态把 assistant 置 `status:"done"`；逐帧新增 `bodyCarrierTagged / afterDoneSwitch / assistantStatus`，新增断言 ⑤：⑤-1 切换后帧数 ≥5、⑤-2 承载节点身份不变（身份标记不丢）、⑤-3 文本不回撤（≥切换前 250 字符）、⑤-4 切换后无空白帧、⑤-5 必须真采到 `status="done"` 帧 |
| **D-59-7** | 口径落契约 | 同上（文件头注释） | 文件头②改为 D-59-7 正式口径：「首次非空帧后不得再空」+「首个内容帧后 ≤4 帧出现首个非空帧」+「首个 render 周期内空白不计白屏闪」，并明示**原「content>0 任一帧不得为空」已作废** |
| **D-59-9** | 重写 F4-5 | `tests/acceptance/wave59/wave59-f4-runtime-status-accuracy.spec.ts` | *F4-5: retrying 由生产路径（submitTurn 遇可重试错误）真实触达，且新一轮 start 后清零* —— 上一轮 assistant `errorDetail` 命中 `isRetryableError`（超时）→ 真实 `submitTurn` → 断言 `mode="retrying"` + 状态条可见 + `turn:started` 后清零；**不再**由夹具写 `TIMEOUT_RETRY_PENDING_MESSAGE` 自证。附带修正：turn:started 须用 `store.activeTurnId`（submitTurn 用自身 requestId 作 turnId）派发 |
| **A-59-3** | 放宽 F4-2 | 同上 | 去掉 `label: "思考中..."` 精确钉死，只断言 `mode` + `data-visible`（另加「文案非空」弱断言） |
| **A-59-4** | 新增 F4-6a/b/c | 同上 | F4-6a：`error` / `sessionError` 强制可见（每例均带「有正文的 pending assistant」作为 F4-3 隐藏压力）；F4-6b：`sessionOperation` = switching / initializing / deleting 强制 `connecting` + 可见；F4-6c：**3 个连续 hop** 均按 `reasoning→thinking / text→responding` 切换 |
| **A-59-1** | 新增回归 | `tests/MarkdownRenderer.spec.ts` | *A-59-1: 非流式空白内容之后，同一实例的流式内容仍必须被渲染（在飞标记不得泄漏）* —— 用 `vi.waitFor` 条件等待替代盲等 |
| **A-59-2** | 新增回归 | `tests/HomeWorkspaceMarkdown.spec.ts` | *A-59-2: 纯文本流式（锁 false）在最后才出现 markdown 语法时，完成态必须渲染 `<strong>` 而非原样 `**`* |

用例总数：`tests/acceptance/wave59` 由 **18 → 22** 条（F1×4 / F2×4 / F3×6 / F4×8）。

### 10.2 零假测试验证：变异（mutation）敏感性 —— 每条新用例都有「去掉修复即转红」的物理证据

> 做法：将当前工作树复制到 `/tmp/w59-mut`（**不改共享工作区**），逐条打回原缺陷（mutant），跑对应用例；
> 每条 mutant 跑完即恢复。mutant 目录已删除，日志在 `/tmp/w59-mut-M*.log`（仓库外，不入仓）。

| Mutant | 打回的缺陷 | 用例 | 结果 | exit | 原始断言摘录 |
| :-- | :-- | :-- | :-- | --: | :-- |
| **M1** | 删除 `runtime.ts` 全部 8 处 `this.activeStreamKind = null`（B-59-1 原缺陷） | F1-4 | **FAIL** | 1 | `F1-4（B-59-1）：awaiting-first-signal 窗口不得被判为 responding；实际：{"mode":"responding","label":"正在回复...","statusSlotVisible":"false","waitingDots":0}` |
| **M2** | `scheduleNonStreamingRender` 空白早退时回填 `renderInFlightVersion`（A-59-1 原缺陷） | A-59-1 | **FAIL** | 1 | `A-59-1：在飞标记泄漏会切断补排闭环 —— 正文停在旧快照（新内容永不渲染）: expected '<div ... class="markdo…' to contain '新内容'` |
| **M3** | 完成态 `streamingRenderPathLock===false` 直接 return false（A-59-2 原缺陷） | A-59-2 | **FAIL** | 1 | `A-59-2：完成态必须渲染 <strong>（锁 false 不得延续到完成态）；实际正文："…纯文本第一段 **加粗收尾**…"` |
| **M4** | 移除 `retryAttempt: retryingAfterTimeout ? 1 : undefined`（D-59-9 接线） | F4-5 | **FAIL** | 1 | `F4-5（D-59-9）：submitTurn 遇可重试错误后必须由生产路径触达 retrying；实际：{"mode":"thinking",…}` |
| **M5** | （对照，非缺陷）完成态锁 mutant | F3-5 e2e | PASS | 0 | 说明：合成流带 markdown 语法 → 锁为 true，M5 分支不进入；**不能**据此证明 D-59-8 灵敏，故作对照实验登记 |
| **M6** | `.markdown-body` 增加 `:key="streaming ? … : …"`（完成态重建承载节点 = RC5 原缺陷） | F3-5 e2e | **FAIL** | 1 | `F3-5 ⑤（D-59-8）：完成态切换后 .markdown-body 节点身份必须不变（不得替换子树）；下列帧丢失了切换前的身份标记或承载节点：` |

结论：F1-4 / A-59-1 / A-59-2 / F4-5 / F3-5⑤ 均在打回原缺陷后**真实转红**；共享工作区源码在验证后仍为修复态（md5 见 §10 头）。

### 10.3 绿相复跑（钉版，命令原文 + exit code）

```bash
cd /home/dm/pony-agent
npm run typecheck                                            # exit 0
npx vitest run tests/acceptance/wave59                       # 4 files / 22 passed, exit 0
npx vitest run                                               # 1 failed | 1137 passed | 12 skipped (1150), exit 1
npx playwright test tests/e2e/wave59-stream-stability.spec.ts # 1 passed, exit 0
```

- 全量唯一失败仍为 `tests/acceptance/wave47-ob2-enforcement.spec.ts > T7-PONYGO-OB2-GATE`（§6.1：并发波次
  wave-slash-1 的 `docs/observability.yaml` 声明变更未获 `--decl-approved`），**非本波**，无新增失败。
- 相对上一轮（1131 passed / 1144）新增 6 pass + 6 全量总数：新增 F1-4、F4-6a/b/c、A-59-1、A-59-2（F4-5 为改写）。

### 10.4 F3-5（含 D-59-8）权威采样摘要

截图/帧数据（**绝对路径**）：`/tmp/dsh-wave59-2026-11-02T01-26-04-062Z/`
（`before-stream-mid.png` 97 708 B、`after-stream-final.png` 144 549 B、`frames.json` 37 299 B）

| 指标 | 值 | 判据 |
| :-- | --: | :-- |
| 帧数 / 采样跨度 / 中位帧间隔 | 127 / 2175.6ms / 16.8ms | ≈60fps |
| 完成态切换帧号 / 切换后帧数 | 96 / 31 | ≥5 |
| 内容列 / 正文容器 scrollHeight 回撤点 | 0 / 0 | ≤1px → **PASS** |
| 完成切换后承载节点身份丢失帧 | **0** | 必须 0 → **PASS**（D-59-8 ⑤-2） |
| 完成切换后文本回撤帧（基线 250 字符） | **0**（终态 365 字符） | 必须 0 → **PASS**（⑤-3） |
| 完成切换后空白帧 | **0** | 必须 0 → **PASS**（⑤-4） |
| 切换后 `assistantStatus="done"` 帧 | 31 | >0 → **PASS**（⑤-5） |
| Console Error / pageerror | **0** | 0 → **PASS** |
| 首内容帧→首非空正文帧滞后 | 6→6 = **0 帧** | ≤4 → **PASS**（D-59-7） |


---

## 11. §复审 r2 补测（A-59-5：会话切换不得携带 hop 级流式类型，2026-11-02）

> 来源：L3 复审 r2 判定 PASS + 1 条同类缺口（Lead 登记契约 **A-59-5**），反例参考 `/tmp/w59r2/leak.spec.ts`（只读，未入仓）。
> 纪律：**只改 `tests/**`**，`src/**` 零改动（本段钉版 md5：`stores/runtime.ts` `1cf33118…`、
> `HomeWorkspace.vue` `5132c8af…`、`MarkdownRenderer.vue` `bbe956fb…`，gate 链前后一致 = PIN UNCHANGED）。

### 11.1 新增用例

| 裁定 | 文件 | 用例 | 断言 |
| :-- | :-- | :-- | :-- |
| **A-59-5** | `tests/acceptance/wave59/wave59-f1-waiting-animation.spec.ts` | *F1-5: 会话切换不得携带上一会话的 hop 级流式类型（含「后台轮有正文缓冲 ⇒ responding」子断言）* | 场景 1（契约主体）：同一 pinia 上会话 A 用**真实事件**驱动到 `activeStreamKind="content"`，再 `switchSession` 到会话 C（C 有后台运行轮、`textBuffer`/`reasoningBuffer` 均空、`phase=calling_model`），**不派发任何 turn:started/delta** → 断言 `data-visible="true"`、`mode ∈ {connecting, thinking}`、label 非空（弱断言，不钉文案）；场景 2（子断言）：后台轮 `textBuffer` 非空时 `mode === "responding"`（按缓冲推导，而非一律置空） |

关键夹具纪律：**不使用全新 pinia 掩盖残渣** —— A→C 全程同一 store，A 的 `content` 态由真实 `turn:started`/`turn:delta` 事件产生（复审已点名 F1-4 早期版本的此类错误，本条已规避）。

用例总数：`tests/acceptance/wave59` 由 22 → **23** 条（F1×5 / F2×4 / F3×6 / F4×8）。

### 11.2 红 → 绿证据（同一代码路径，executor 修复前后）

| 阶段 | 结果 | 原始断言摘录 |
| :-- | :-- | :-- |
| **红（修复前，09:36 采集）** | F1-5 **FAIL** | `F1-5（A-59-5）：切到「本 hop 尚无正文」的后台轮后不得继承上一会话的 content 流式类型；实际：{"sessionId":"w59-f1-5-session-c","activeStreamKind":"content","mode":"responding","label":"正在回复...","statusSlotVisible":"false"}` |
| **绿（修复后，09:39 采集）** | F1-5 **PASS** | 5/5 passed（`stores/runtime.ts` 后台轮恢复路径改为按 `textBuffer`/`reasoningBuffer` 推导，无后台轮时置 `undefined`） |

红相即灵敏度证据：该失败由**真实跨会话序列**（同 store、真实事件）触发，非夹具自证。

### 11.3 绿相复跑（钉版，命令原文 + exit code）

```bash
cd /home/dm/pony-agent
npm run typecheck                          # TYPECHECK_EXIT=0
npx vitest run tests/acceptance/wave59     # Test Files 4 passed (4) / Tests 23 passed (23) / WAVE59_EXIT=0
```
