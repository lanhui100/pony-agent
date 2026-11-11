# Agent Note: 流式视觉层接入：允许单个 inline 尾巴节点（修订 D-59-1 与 F3-1 部分断言）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0059-streaming-visual-layer-tail-node.md` 之归档快照，仅供溯源。

## 背景

`0.1.175` 发布后用户反馈"消息区没有更平滑的 streaming"。排查确认两点（均为本仓事实，非推测）：

1. **视觉层从未接入**：`src/` 对 `.agents/skills/streaming-motion/{tokens,primitives}.css` 零 import、`data-pds-*` 零出现。`0058` 接入的只是**释放节奏的内部实现**（等价重构），用户可见行为设计上就是零变化。
2. **架构本身也出不来该效果**：`.markdown-body` 用 `v-html` 整块替换，子节点每次 render 全部重建 ⇒ 任何"新元素入场动画"每 tick 重播（这正是 `0056` 记录的 RC2）；且释放节奏是 `60ms × 4 字` = 16.7Hz 的 4 字阶梯，顿挫主要来自阶梯而非缺动画。

## 候选方案

- **方案 A：只做节奏细化 + 外围动效（思考尾巴 / 工具行 / 状态行），不动正文**。落选：能拿到约七成观感且零契约风险，但正文（用户注视时间最长的地方）仍无任何"浮现"承载点，用户明确要求完整效果。
- **方案 B：恢复逐字符 `span`，做真正的逐字模糊消解**。落选：需**同时**推翻 `D-59-2`（移除逐字符淡入）与 `0057` 反模式 §1；每 tick `O(N)` 节点重建必然闪跳与掉帧，与 `0057` 的核心红线正面冲突。
- **方案 C：不改 DOM，只提节奏频率**。落选：只减少阶梯间隔，正文依然"啪"地出现，没有质感变化。
- **方案 D（采纳）：单个 `inline` 尾巴节点 + 右缘遮罩渐变**。尾巴承载"当前未定稿块"的文本，右缘 `mask-image` 让新字符从透明区进入、随吐字左移渐实。**零字符节点、不重启任何 animation**（是静态遮罩 + 内容推进），因此与 RC2 有本质区别。

## 决策

1. **修订 `D-59-1`**：允许**恰好一个** `inline` 尾巴节点，必须打 `data-stream-tail`；仍禁止块级尾巴与多个尾巴。
2. **修订 `F3-1`**：`.mr-streaming-raw` / `.streaming-unrendered-suffix` / `.mr-streaming-char` 继续为 0；新增「除 `[data-stream-tail]` 外不得有第二个尾巴节点」。
3. **保持 `D-59-2`**：不建任何字符级节点。此为硬红线，`0057` 反模式 §1 不变。
4. **节奏**：`STREAMING_PRESENTATION_TICK_MS` `60 → 33`；`DEFAULT_REVEAL_BUDGET.charsPerTick` **保持 4**。理由：冻结基线 14 例与 parity 黄金表都以"每 tick 4 字"为真值，改它必须迁移冻结断言；而观感提升由"更高 tick 频率 + 新字符落在渐隐区"承担。契约起草时曾误写"4→2"，被 Test Agent 判为与 V10 互斥，已撤销。
5. **代码块尾巴必须内嵌在 `<pre><code>` 内**：否则栅栏闭合那一帧块级盒子合并会导致 `scrollHeight` 回撤（实测 16–17px × 15 次），违反 V4。
6. **降级双轨 + mount-only 开关**：`prefers-reduced-motion` 与 `[data-pds-reduce-motion="true"]` 都必须 `animation: none` 且尾巴 `mask-image: none`；localStorage 开关 mount-only 读取，系统偏好每帧求值。
7. **输入前提**：`HomeWorkspace` 新增 `cleanStreamingHopText`——流式正文只剥前导换行、保留尾随换行，否则渲染层永远看不到"块已闭合"，尾巴无法归零。非流式与 reasoning 路径不变。

## 影响

- **用户可见变化**：正文以 30Hz 推进、新字符从右缘渐显；思考尾巴 140ms 位移淡入；工具行/面板 blur 消融 220ms；状态行 rise/fall 原位交接。
- **新增回退能力**：`localStorage["pony-agent.stream-render.tail-reveal"]="off"` 一行关掉尾巴，行为回到 `0056` 形态（已在真 Chromium 验证 tail 数为 0 且正文完整）。
- **性能**：真 Chromium 三次实测 Δp95 = +3.4 / +2.9 / +2.9 ms，Long Task(>50ms) 恒为 0，未触发契约的回调口子（tick 保持 33，未退 50）。
- **测试策略演进**：新增 `tests/acceptance/pds-streaming/`（jsdom 35 例 + Chromium 8 例，冻结于 `8e51bd5`）。契约在评审中改了三版：撤销 `charsPerTick` 改动、作废 16.7ms 绝对帧阈值（60Hz 空闲 p95 即 16.9ms）、固化占位流参数（delta 120ms / ≤50 字符 / 900 帧）并改为「Δp95 ≤6ms + Long Task=0 + 连续 3 次」。`tests/acceptance/wave59/**` 与 `tests/useStreamingPresentationState.spec.ts` **一字未改**。
- **跨波次耦合（已知）**：`V10-3` 会跑既有全量单测，当前被**另一并发波次**未提交的 `docs/observability.yaml` 触发的 `ob2 [D1]` 门禁卡红。已独立复核（`ponygo observe audit --level ob2` 复现），剔除该外部用例后全量 1191 绿。解除条件：该波次带 `[decl-change-approved]` 提交后重跑。
- **可观测性**：纯渲染层，四维归属 N-A（无新增接口 / 网络 IO / 后台任务 / LLM 调用）。

## 实施后的追加记录（契约 v2→v11 演进与最终形态）

本波在实施期经历 **10 版契约修订**，每一版都由独立测试/审查发现具体缺陷后裁定，此处只记结论（细节见 `.dev-team/contracts/wave-pds-3-streaming-visual.md` 的修订记录）：

1. **撤销 `charsPerTick 4→2`**（v2）：与冻结基线 14 例互斥。节奏只改 tick `60→33`。
2. **作废 16.7ms 绝对帧阈值**（v2）：60Hz 空闲 p95 本身即 16.9ms，绝对阈值会把"不做事"判红。
3. **固化占位流参数**（v3）：delta 120ms / ≤50 字符 / 900 帧，否则 V8 不可复现。
4. **取消"尾巴 ≤1 行"硬约束**（v5）：该约束逼实现劈块，导致表格/标题/加粗/嵌套列表/多行段落渲染错乱。改为"多行尾不加遮罩"。
5. **V8 改同轮 A/B 差分**（v6）：相对基线法校正不了"测量期间被外部负载污染"（实测污染轮假绿、干净轮假红）。A=`tail-reveal=off`（等价旧行为）、B=新行为，直接测本改动自身开销。
6. **收缩尾巴作用域**（v7）：只要允许在任意未闭合块上切分，就等于要求实现增量 markdown 解析器。改为"非纯段落不发尾巴"，并删除 `composeCodeTailHtml`。
7. **补纯文本快路径**（v8）：`WorkspaceTurnItem` 的 `isSimpleTextContent` 快路径不经过 `MarkdownRenderer`，导致无 markdown 标记的普通回复**完全没有尾巴**——这曾使"观感收益基本保留"这句话不成立。
8. **资格门改语义判定**（v9）：v7 的 8 条行正则不是"纯段落"的充要条件（实测漏 tab 缩进 → 尾巴里出现 `<pre>`、`#` → `<h1>`、`***` → `<hr>`、单 `-` → `<h2>`）。改为 `renderPartialMarkdown(tailSource)` 后检查结果里有无块级标签，**判据与渲染器同源**，并删除全部行黑名单。
9. **`$$` 裁定**（v10）：以渲染结果为准，`$$` 渲染为普通段落即允许发尾巴；拒绝为它重新引入黑名单。该原则可自校正——将来接入数学扩展后 `$$` 变块级，判据自动发回 stable。
10. **V8 瞬时尖峰容许**（v11）：仅 `B−A>4ms` 且两段 Long Task=0 的轮次标 `[SPIKE]`、不计入 3 轮干净轮也不直接 FAIL，自动补测（上限 8 轮）。阈值未放宽。

### 最终形态要点
- **资格门**：渲染结果无块级标签（`h1–h6/ul/ol/li/pre/table/blockquote/hr/div/section/figure/dl/dt/dd`）且顶层 `<p>` ≤ 1；同一渲染结果直接复用为尾巴 HTML（实测每 tick 仅 1 次 `renderPartialMarkdown`，无重复渲染）。
- **非资格路径**：回到 `autoCloseBoundaries` 增量提交语义，且"未闭围栏且末行无换行时只补 `\n`、不补闭合围栏"——既消除屏上字面 ```，又不触发 md-unwrap 吃掉 `#`。
- **纯文本快路径**：同样发尾巴，切分取最后一个空行之后；`stable + tail` 为同一字符串连续切片，**拼接恒等于原文**（12 例 + 12 步前缀全等断言）。
- **性能**：V8 三次干净轮 `B−A = +0.10 / +0.10 / +0.20 ms`，两段 Long Task 全 0（v9 的 render-then-verify 使 B−A 由负转正，仍远低于 4ms 预算）。

### 已知限制（如实登记，不在本波修）
1. **loose list 无法整体落在未闭合块**：`splitMarkdownBlocks` 遇空行必切块，故流式期可能出现两个 `<ul>`、done 帧合并为一个。与 HEAD 同构，非本波引入。
2. **非资格路径每 tick 渲染两次**（资格探针 1 次 + `renderNonEligibleTail` 1 次）：代码块/列表/表格流式期每帧多一次 marked + DOMPurify。V8 预算内可接受，后续可按需复用探针 HTML。
3. **纯文本资格门实际不生效**：该路径是文本插值、不含 HTML，不存在块级嵌套风险；当前实现为"乐观放行 + 异步缓存"，属冗余但无害。
4. **`data-pds-reduce-motion` 无 UI 写入者**：属性轨为"已支持、未接线"能力，媒体轨（`prefers-reduced-motion`）承担实际降级。
5. **V12 的"无行丢失"真值来自 `renderMarkdown(全文)`**：该函数漂移则 V12 跟漂。
6. **门禁必须在工作树稳定时跑**：V10-2/V10-3 会子进程实跑套件，写入过程中跑会把"文件半成品"放大成基线回归（实施期实际踩到过两次）。

