# Wave-59 交付说明：桌面端 UI/UX 四项优化

日期：2026-11-01 · 分级：B · 基线 commit：`d3745a7` · 契约矩阵：[contracts/wave-59-desktop-uiux.md](contracts/wave-59-desktop-uiux.md)
团队：Lead + test-agent（独立验收测试）+ executor（实现），Subagent 承担同源复核。

## 1. 需求 → 结果

| # | 需求 | 结论 | 关键证据 |
| :-- | :-- | :-- | :-- |
| ① | 三点跳跃动画与底部「思考中」重复 | 已移除（类型/渲染/CSS 全族删除），等待指示唯一由底部状态条承载 | F1-1/1-2/1-3；真浏览器实测 `dots=0`、状态条 `data-visible=true` 文案「思考中」 |
| ② | 轮次等计数下移，与 token 计数同卡排列 | 计数移入指标卡内 `status-metrics-counts` 行（3 列网格、图标与数字成组、tabular-nums 保留） | F2-1..F2-4；截图 01；wave-17 C2-2 在真 Chromium 新锚点下转绿 |
| ③ | streaming 时 agent 消息闪跳 | 5 个根因（RC1..RC5）全部消除；内容承载节点跨 streaming→done 身份不变 | F3-1..F3-5；真浏览器 102 帧采样：**内容列回撤 0 次（最大 0px）、空白帧 0、Console Error 0** |
| ④ | 底部状态消息不准确 | hop 级 `activeStreamKind` 修正 RC6/RC7/RC8；`retrying` 接真实超时信号（RC9），缺分母时降级不虚构 | F4-1..F4-5；截图 02/03 |

## 2. 根因（诊断结论，非猜测）

| 编号 | 根因 | 位置 |
| :-- | :-- | :-- |
| RC1 | 未渲染尾巴渲染为**独立块级节点**，每 tick 新建一行再被吸回段落 | `MarkdownRenderer.vue` 原 332-343 |
| RC2 | 尾巴按逐字符 span + translateY 动画、且每次 render 重建 → 反复从 opacity 0 重播 | 同上 334 |
| RC3 | `executeRender` 在 await 后用**新 content 长度**标记已渲染，却用旧 content 生成 html → 文字出现-消失-再出现 | 原 152-157 |
| RC4 | 纯文本路径逐字符动画下标为绝对下标、封顶 480ms → 新字符成批延迟浮现 | `WorkspaceTurnItem.vue` 原 481-489 |
| RC5 | 流式走纯文本分支、完成态走 markdown 分支 → 完成帧替换整棵子树 | `WorkspaceTurnItem.vue` + `HomeWorkspace.vue` |
| RC6 | `isStreamingReasoning` 取"本轮是否出现过 reasoning" → 出现过即整轮 thinking | `stores/runtime.ts` + `lib/runtime/status.ts` |
| RC7 | `hasStreamingContent` 取累计 content → 工具后的新 hop 被误判"正在回复" | 同上 |
| RC8 | 可见性判据取"本轮任意 content 非空" → 工具后思考 hop 状态条被隐藏 | `HomeWorkspace.vue` |
| RC9 | `retrying` 分支无 store 信号源（不可达死代码） | `lib/runtime/status.ts` |

## 3. 改动清单（本波 7 文件）

| 文件 | 本波改动 |
| :-- | :-- |
| `src/components/chat/WorkspaceTurnItem.vue` | 删 `waiting` 事件类型与渲染/CSS；内容区统一为单容器，纯文本路径自建 `.markdown-body`；删逐字符动画 |
| `src/components/HomeWorkspace.vue` | 不再注入 waiting 事件；`shouldShowAgentArticle` 去 awaiting 分支；状态条可见性改为 hop 级三态 |
| `src/components/HomeStatusPanel.vue` | 头部只留「状态」+复制按钮；新增 `status-metrics-counts`（3 列网格）；token 行结构不变 |
| `src/components/MarkdownRenderer.vue` | 单根 + 唯一常驻 `.markdown-body`；删尾巴节点/逐字符；入口快照修 RC3；在飞/脏标记补排闭环；失败降级为转义文本 |
| `src/stores/runtime.ts` | `activeStreamKind` 三态（`undefined` 回退旧判据 / `null` hop 起点 / `reasoning`\|`content`）；`clearTimeoutRetryPlaceholder` 清零 retry 信号 |
| `src/lib/runtime/status.ts` | `retrying` 真实接线；缺 `retryMax` 时 label 降级「正在重试…」（不虚构分母） |
| `src/lib/useStreamingPresentationState.ts` | 无改动（D-59-2：释放节奏保留） |

## 4. 门禁物理收据（Lead 亲跑）

| 门禁 | 命令 | exit | 结果 |
| :-- | :-- | --: | :-- |
| 类型检查 | `npm run typecheck` | **0** | vue-tsc 无错误 |
| 全量单测 | `npx vitest run` | **1** | 119 files / **1131 passed / 1 failed / 12 skipped**；唯一失败 `wave47-ob2-enforcement > T7` 为**并发波次既有失败**（wave-slash-1 改了 `docs/observability.yaml` 声明未批准） |
| 构建 | `npm run build` | **0** | vue-tsc + vite build |
| 真浏览器 F3-5 | `npx playwright test tests/e2e/wave59-stream-stability.spec.ts` | **0** | 1 passed；102 帧 / 0 回撤 / 0 空白帧 / 0 Console Error |
| 真浏览器 C2-2 | `npx playwright test tests/e2e/wave17-browser-gate.spec.ts -g C2-2` | **0** | 1 passed（新锚点下 tabular-nums 物理口径） |
| 可观测性（变更级） | `audit-diff.sh --base d3745a7` | **0** | 无越界实现、收口点完整 |
| 可观测性（声明级） | `check-declaration.sh .` | **0** | 四维声明与实测一致（N-A 理由见契约 D-59-4） |

基线对照：`d3745a7` 全量 = 114 files / 1100 passed / 12 skipped / exit 0 → 本波 +5 files / +31 passed / 0 新增失败（新增 18 条 wave59 + 并发波次新增用例；6 条迁移用例改名后仍绿）。

## 5. 证据（截图，真 Chromium + vite preview）

| 截图 | 说明 |
| :-- | :-- |
| [01-status-panel-counts.png](evidence/wave-59/01-status-panel-counts.png) | 需求②：三计数（2 / 3 / 2）已下移至指标卡内，与 token 位同卡，图标与数字成组 |
| [02-awaiting-first-signal.png](evidence/wave-59/02-awaiting-first-signal.png) | 需求①：等待首信号时**无三点动画、无空 agent 壳**，唯一指示为「思考中，用时 4秒」状态条 |
| [03-status-tool-running.png](evidence/wave-59/03-status-tool-running.png) | 需求④：工具执行态标签「执行工具: 读取文件」准确 |
| [04-streaming-final.png](evidence/wave-59/04-streaming-final.png) | 需求③：流式终态渲染完整（代码块/标题/列表/引用） |
| [05-streaming-mid.png](evidence/wave-59/05-streaming-mid.png) | 需求③：流式中途帧，正文单块承载、无尾巴行位移 |

过程类采样数据（frames.json、红相/绿相运行日志）留在 `/tmp/dsh-wave59-*`，按「中间产物三分法」不入仓。

## 6. 裁定与偏差登记

- **D-59-5（Lead 追加）**：F2 计数行排版二次微调 —— `justify-between` + 右对齐数字导致三组指标被拉散，
  改为 `grid grid-cols-3` + 载体 `justify-center` + 数字 `min-w-[1.75rem] text-left`；wave-17 C2 物理口径与 testid 全部保留。
- **D-59-6（Lead 追加授权，test-agent 执行）**：
  ① `markdown-streaming-smooth.spec.ts` 盲等 60ms → `vi.waitFor`（删硬编码等待，断言语义不变）；
  ② `wave17-browser-gate.spec.ts` C2-2 增加夹具 token 注入（阈值零改动）→ 该用例由既有 e2e 债务转绿。
- **契约演进**：迁移 6 条既有断言（wave17 C2-1/C2-2、HomeWorkspace ×2、MarkdownRenderer ×3、HomeWorkspaceMarkdown ×1），
  仅更换定位锚点，物理口径未动；红相在 `git archive d3745a7` 纯净快照采集（见 [wave59-red-receipts.md](wave59-red-receipts.md)）。
- **未提交**：共享工作区有并发波次（wave-slash-1 斜杠命令）未提交改动，按「Hunk 归属审计」纪律本波**不做 git 提交**，
  避免夹带他波改动。需提交时按文件 hunk 定向暂存。

## 7. 遗留与风险

1. `wave47-ob2-enforcement > T7` 仍红：责任在并发波次（`docs/observability.yaml` 声明变更需 `[decl-change-approved]`），非本波引入。
2. 纯文本完成的 assistant 消息保持纯文本容器（F3-3 的必要代价）：与 markdown 段落在行距/间距上有细微差异。
3. 可观测性四维本波为 N-A（纯渲染层，无新增接口/网络 IO/后台任务/LLM 调用），已在契约 D-59-4 登记结构理由。
4. 环境：4175 预览进程已终止，无残留监听；`git worktree` 无残留；仓库内无过程类临时文件。

## 8. 遗留事项处理记录（2026-11-02 追加）

| 事项 | 状态 | 说明 |
| :-- | :-- | :-- |
| ADR | ✅ 已补 | [ADR 0056](../../docs/decisions/0056-streaming-render-stale-while-render-and-hop-status.md)（含候选方案 A–E 与落选理由）；`docs/INDEX.md` 已登记；`docs/streaming-rendering-optimization.md` 顶部加「已被取代」状态指向本 ADR（保留历史权衡，三件套函数仍在用） |
| 元复盘 | ✅ 已补 | `.dev-team/meta-retrospective.md`（wave-59；wave-55 版归档为 `meta-retrospective-wave55.md`），经 `dev-meta-lint.py` 校验 |
| 独立审查收据 | ✅ 已补 | `review-plan-wave59.json`（契约可验证性）与 `review-join-wave59.json`（合入审查），fresh-context Subagent 产出，经 `dev-review-lint.py` 校验 |
| Phase 6 入队 | ✅ 已入队 | `dev-queue.py enqueue`（见 §9） |
| /tmp 采样数据 | ✅ 已归档 | 权威采样目录的快照与 `frames.json` 摘要结论已写入本文件 §4；原始大文件仍在 `/tmp/dsh-wave59-*`（过程类，不入仓） |
| **git 提交** | ⛔ **未做（依赖外部）** | 共享工作区存在**另一仍在活动**的波次（slash commands / TUI，最近写入 2026-11-02 09:02）未提交改动；其中 `src/stores/runtime.ts`、`src/lib/runtime/status.ts`、`src/components/HomeWorkspace.vue` 三个文件与 wave-59 **同文件混改**，getter 区域（runtime.ts `@@ -538,+557`）两波改动落在同一 hunk，**hunk 级切分不可行**；强行做 wave-59-only 提交会产出不可构建/不可测的中间态。已做工作区无损快照（见 §10），等该波提交后即可定向提交 |
| 采样数据留证 | ✅ 已登记 | `/tmp/pony-untracked-20261102T090813.tar.gz`（39 个未跟踪文件）+ `git stash create` 提交对象 |

## 9. Phase 6 入队

见 `dev-queue.py status` 输出（本文件以 .dev-team 账本为准）。

## 10. 工作区无损快照（防丢失，非提交）

- `git stash create` → 提交对象 `62792c1c1c2dc0db2e211144bd537557ec3ac924`（含全部已跟踪文件的当前改动；未改动 HEAD/索引/工作区）
- 未跟踪文件打包 → `/tmp/pony-untracked-20261102T090813.tar.gz`（498747 B，39 个条目）
- 恢复方式：`git stash apply 62792c1c`（或 `git checkout 62792c1c -- <path>`）；未跟踪文件 `tar -xzf` 到仓库根

## 11. L3 独立审查两轮与修复闭环（2026-11-02）

| 轮次 | 收据 | verdict | 发现 | 处置 |
| :-- | :-- | :-- | :-- | :-- |
| 计划阶段 | `review-plan-wave59.json`（`l3-plan-wave59-9c4e1b`） | PASS | 0 BLOCKING / 7 ADVISORY | 3 条采纳为契约修订（D-59-7/8/9、A-59-3/4），其余登记为可接受 |
| 合入 r1 | `review-join-wave59.json`（`l3-join-wave59-4f2a9c`） | **FAIL** | **1 BLOCKING**（B-59-1 `activeStreamKind` 终态不复位）+ 2 ADVISORY（A-59-1 在飞标记泄漏 / A-59-2 纯文本锁延续到完成态） | 全部修复（`runtime.ts` 终态四路 + `submitTurn` 复位；`MarkdownRenderer` 在飞标记配对；`HomeWorkspace` 锁仅流式期有效） |
| 合入 r2 | `review-join-wave59-r2.json`（`l3-join-wave59-r2-6b31d0`） | **PASS** | 0 BLOCKING / 1 ADVISORY（A-59-5 会话切换携带 hop 级瞬时态） | 已修复（后台轮按 buffer 推导 + 快照/复位不含该字段），新增 F1-5 覆盖 |

**B-59-1 的价值**：18 条验收用例全绿时机器门禁无一发现 —— F1 夹具用新 pinia（`activeStreamKind=undefined` 走回退）掩盖了状态残渣。
若无 L3，该缺陷会让「每轮结束后开新轮」的数秒窗口零进行中指示（三点动画已删），直接进入主干。

**变异敏感性证据（test-agent 提供）**：M1 删 8 处复位 → F1-4 红；M2 回填在飞标记 → A-59-1 红；M3 锁延续 → A-59-2 红；
M4 删 retryAttempt 接线 → F4-5 红；M6 给 `.markdown-body` 加 key → F3-5⑤ 红；M5 为对照（PASS，已如实登记）。

**最终门禁（Lead 亲跑，修复后钉版）**

| 门禁 | exit | 结果 |
| :-- | --: | :-- |
| `npm run typecheck` | 0 | — |
| `npm run build` | 0 | vue-tsc + vite build |
| `npx vitest run tests/acceptance/wave59` | 0 | **23 passed**（F1-1..F1-5 / F2-1..4 / F3-1..4 / F4-1..6） |
| `npx vitest run`（全量） | 1 | **1138 passed / 1 failed / 12 skipped**；唯一失败 `wave47-ob2 > T7`（并发波次声明门禁） |
| `npx playwright test tests/e2e/wave59-stream-stability.spec.ts` | 0 | 127 帧；回撤 0、身份丢失 0、文本回撤 0、空白帧 0、done 帧 31、Console Error 0 |
| `audit-diff.sh --base d3745a7` | 0 | 无越界实现、收口点完整 |
| `check-declaration.sh .` | 0 | 四维声明与实测一致（本波 N-A） |
| `dev-meta-lint.py` | 0 | 元复盘四大维度 + 审查收据齐备 |
| `dev-review-lint.py`（3 份 review JSON） | 0 | schema v2 / 溯源 / 范围闭合均通过 |
