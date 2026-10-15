# Wave-11 冻结契约 — 桌面端消息流下方状态条（PA-STATUS-FLOW）

- 分级：**B-lite**（跨 3 个前端文件、单模块=chat 消息流呈现层；无外部 IO/无业务状态机）。
  机械分级依据：文件数=3（`AgentRuntimeStatusBar.vue` / `WorkspaceComposer.vue` / `HomeWorkspace.vue`），
  无 `net/io/os/exec` 关键字命中，无外部调用 → 不启用后端 NFR schema，改用等价 UI-NFR 基准
  （`.dev-team/nfr-baseline-ui-wave11.json`）。
- 基线 commit：`018bcdc`
- 红相/绿相时序：Test Agent 先落盘验收测试（红）→ Lead 锚定 commit → Executor 才可写 `src/**`。

## 1. 需求 → 可机械判定断言

| 需求 | 断言 ID | 判定方式（机器） |
| :-- | :-- | :-- |
| R1 图标换掉 loading，改为有趣动效 | M5 | `[data-testid="agent-runtime-status-icon"]` 带类 `runtime-status-orbit`，内含 ring/dot 两个子节点；`getComputedStyle(dot).animationName !== "none"`；`prefers-reduced-motion` 下为 `none`；类名不含 `snake-spinner`/`animate-spin` |
| R2 新消息出现时状态消息无布局突变 | M1/M4 | 浏览器实测：状态条视口 Y 在「流式增长」与「插入新消息」两种场景下的逐帧位移极差 ≤ 1px（基线：44 次方向反转、±23.8px 抖动） |
| R3 streaming 期间不显示状态消息 + 上推平滑 | M2/M3a/M3b/M3c | 流式期间 `[data-testid="workspace-runtime-status-slot"]` 的 `data-visible="false"` 且计算 `opacity === "0"`；内容运动方向反转 ≤ 1（容差 8px，见 §7.2）+ 无超调（M3b）+ 跟随滞后 ≤ 96px（M3c，4 行包络；见 §7.3） |

## 2. DOM / 行为契约（冻结）

1. **状态条移出滚动内容列**：不再作为 `[data-testid="workspace-content-column"]` 的子节点；
   改为 `WorkspaceComposer.vue` 新增 `#status` 插槽内的绝对定位行（`bottom-full`，即紧贴输入面板上沿上方），
   与 composer 同宽同居中（`max-w-[46.4rem]`）。禁止任何影响 `ScrollArea` / 内容列高度的写法。
2. **槽位常驻**：`[data-testid="workspace-runtime-status-slot"]` 在 `agentRuntimeStatus != null` 期间**始终存在于 DOM**（含流式期间），
   仅通过属性与透明度切换可见性 → 消除挂载/卸载导致的布局突变。
3. **可见性属性**：槽位必须携带 `data-visible="true" | "false"`；
   `data-visible="false"` 时：`opacity: 0`、`visibility: hidden`（过渡结束后）、`aria-hidden="true"`、`pointer-events: none`。
4. **隐藏条件（R3）**：`data-visible="false"` ⟺ 最后一个可见 turn 的 assistant 为 `pending` **且** 有可见流式内容
   （`assistantHasVisibleContent`）**且** 无 `running`/`planned` 的工具活动。
   思考中（无内容）/工具执行/等待授权/重试/错误/会话操作等状态**必须保持可见**。
5. **单行不换行**：状态条必须 `white-space: nowrap` + 溢出截断；任何状态下槽位高度恒定（同一 viewport 下极差 ≤ 1px）。
   槽位需为右侧 `workspace-scroll-to-bottom` 按钮预留空间（`padding-right` 或 `max-width: calc(100% - 3.5rem)`）。
6. **过渡**：显隐为 `opacity` + `translateY(≤4px)`，180–240ms `ease-out`；不得使用 height/max-height 动画。
7. **标签切换**：label 文本变化不得出现整行闪跳（禁止「先消失后出现」的空白帧）；允许原位替换 + 持续 shimmer 动效（wave-11.2 F3 修订：原「交叉淡入 ≤200ms」措辞作废，因 `:key` 重建 + 仅淡入反而产生空白帧）。
8. **错误态**：`mode === "error"` 保留 `AlertCircle`，不带 orbit 类，且 `data-visible` 语义不变。
9. **纯 CSS 动效**：图标动画只允许 CSS keyframes（禁止 JS 定时器 / rAF 逐帧驱动）。
10. **既有约束保持**：状态条根元素类名不含 `bg-*` / `border-*` / `animate-pulse`（既有单测已冻结）。
11. **testid 不变**：`workspace-runtime-status-slot`、`agent-runtime-status-bar`、`agent-runtime-status-label`、
    `agent-runtime-status-elapsed`、`agent-runtime-status-icon` 全部保留；新增 ring/dot 子 testid。

## 3. UI-NFR 基准（`.dev-team/nfr-baseline-ui-wave11.json`）

- 状态条视口 Y 极差 ≤ 1px（流式 / 新消息插入两场景）。
- 流式内容：方向反转 ≤ 1（容差 8px）+ 无超调（4 帧预算 + 8px）+ 跟随滞后 ≤ 96px（字段定义见 `.dev-team/nfr-baseline-ui-wave11.json`，原「单帧位移 ≤ 32px」与「64px」均已 supersede，理由见 §7.1/§7.2/§7.3）。
- 图标轨道环对比度 ≥ 3:1；320px 视口下状态条文本与展开态「滚动到最新」按钮水平不重叠。
- 浏览器 Console Error = 0；`prefers-reduced-motion: reduce` 下所有图标动画 `animation-name === "none"`。
- 前端单测 `npm run test:unit` EXIT=0；`npm run typecheck` EXIT=0；`npm run build` EXIT=0。

## 4. 写域（advisory，波内并发=1）

| 角色 | write_scopes |
| :-- | :-- |
| Test Agent | `tests/`、`.dev-team/evidence/`（只读契约与 `src/`） |
| Executor | `src/components/chat/AgentRuntimeStatusBar.vue`、`src/components/chat/WorkspaceComposer.vue`、`src/components/HomeWorkspace.vue` |
| Lead | `.dev-team/`、`playwright.config.ts`（如需浏览器可执行路径修复） |

## 5. 基线实测（Lead 侦察，物理收据 `.dev-team/evidence/baseline-status-flow.json`）

1280×860 真实 Chromium（vite preview @4175），合成 3s 流式 + 真实 browser-preview 提交：

| 场景 | 状态条 Y 方向反转 | 状态条单帧最大位移 | 流式期间是否显示状态条 |
| :-- | --: | --: | :-- |
| 真实短流（320ms） | 2 | 148.6px | 是（"正在回复，用时 0秒"） |
| 合成长流（3s） | **44** | **85.8px**（稳态 ±23.8px 抖动） | 是（"正在回复，用时 3秒"） |

→ 根因：状态条位于滚动内容列内、且位于流式增长内容之下，内容每增长一行（23.8px）就把它下推一行，
随后自动跟随滚动又在后续帧把它拉回，形成逐行 ±23.8px 反复抖动；且 streaming 期间未隐藏。

## 6. 非目标（Non-Goals）

- 不修改 `src/lib/useTimelineAutoScroll.ts`（除非 M3 断言实测不达标，且需 Lead 一票批准 + 契约演进）。
  → **已触发，见 §7。**
- 不修改 `src/lib/runtime/status.ts` 的状态解析语义与任何文案。
- 不重构 composer 结构、不调整 `COMPOSER_BUFFER_PX`。
- 不处理移动端/窄屏专属布局（≤ 640px 仅要求不破版）。

## 7. 契约演进 wave-11.1（Lead 一票批准，2026-10-15）

**触发**：M3 的绝对单帧位移阈值（≤32px）实测不达标（43/48px），按 §6 例外条款进入契约演进。

**根因（硬数据，`.dev-team/evidence/m3-lag-probe-result.json` + `m3-lag-probe.mjs`）**：

1. M3 原度量是**不良定义**：单帧位移把「内容真实增长」与「滚动补偿质量」混为一谈。合成流每 60ms 增长 23.8px，浏览器忙帧会把 2 行合并进一帧（+47.6px）；任何实现的单帧位移下界 = 该帧内容增长量 → 32px 阈值天生 flaky（TEST-DEFECT）。
2. 真正的缺陷被量化：流式全程 `anchorToComposerDistance` 稳定在 **−179 ~ −208px**，即跟随 lerp 永远追不上内容增长，内容在流式期间一直贴在输入面板上沿（220px 呼吸区被吃掉 ~180px），流末才回弹 → 即用户所述「反复跳变」的成因之一。
3. 成因：`useTimelineAutoScroll.handleLatestTurnSignatureChange` 对流式 assistant 更新使用 `behavior:"smooth"`（自定义 rAF lerp），每次新事件都 `cancelScrollLerp()` 重启缓动、只走完第一帧大步长；而 `handleMarkdownRenderComplete` 的流式路径早已用 `"auto"` → 两条流式跟随路径互相打架。

**演进内容**：

- **实现**：`src/lib/useTimelineAutoScroll.ts` 中 `isStreamingAssistantUpdate === true` 时跟随行为由 `"smooth"` 改为 `"auto"`（与既有 markdown 流式路径统一）。仅此一处语义改动。
- **度量（M3 替换为良定义三元组）**：
  - **M3a** 内容运动方向反转 ≤ 1（容差 2px）——「反复跳变」守卫（保留）。
  - **M3b** 无超调：每帧 `|ΔscrollTop_i| ≤ ΔcolScrollHeight_i + ΔcolScrollHeight_{i-1} + 4px`。
  - **M3c** 跟随滞后有界：每帧 `|(scrollTop_i − scrollTop_0) − (colScrollHeight_i − colScrollHeight_0)| ≤ 64px`（当前实现 −179~−208px → 红；演进后必须绿）。〔历史记录：阈值后经 §7.3 修订为 96px，公式经 §7.2 追认为 `|scrollTop − max(0, colScrollHeight − clientHeight)|`〕
  - 删除绝对 `maxJump ≤ 32px`（理由见根因 1）。
- **测试确定性**：e2e 中于 `goto` 前拦截 `api.github.com` 更新检查请求，消除沙箱代理 403 导致的 Console Error 偶发；断言仍要求 0 条 error（不放宽为白名单过滤）。
- **写域扩展**：Executor 增加 `src/lib/useTimelineAutoScroll.ts`；Test Agent 同步 `tests/HomeWorkspace.spec.ts` 中编码旧 smooth-for-stream 语义的用例。
- **审查要求**：本文件属滚动引擎核心，blast-radius 硬触发 → Join Barrier 需 ≥2 路互补 L3 审查 + 1 路 UI 生产就绪审查。

### 7.1 A/B 实测裁定：§7 引擎改动**无效，已回退**（Lead，2026-10-15）

对同一合成流式场景，用同一探针（`.dev-team/evidence/m3-lag-probe.mjs`）在「§7 auto」与「原 smooth」两版实现上各跑一次（其余代码完全相同），物理量对照：

| 指标 | §7 auto（改后） | 原 smooth（改前） |
| :-- | --: | --: |
| `anchorToComposerDistance` 区间 | −226.6 ~ −178.5px | −226.1 ~ −178.5px |
| 距底滞后峰值（列高口径） | 48.0px | 47.0px |
| 视口 `distanceToBottom` 峰值 | 48px | 47px |
| 单帧 \|ΔscrollTop\| > 25px 帧数 | 2 | 2 |
| M3b(K=4, slack=8) 超调帧 | 1（超 20px） | **0** |
| M3b(K=5) 超调帧 | 1（超 20px） | **0** |

结论：§7 的 `smooth→auto` 无因果收益，且在超调度量上略差 → **按「严禁夹带重构」回退**（patch 存档 `.dev-team/evidence/reverted-wave11.1-auto-follow.patch`）。

**根因更正（推翻 §7 的原始根因判断）**：`anchorToComposerDistance ≈ −179px` 不是「跟随滞后」，而是**锚点目标结构性不可达**（composer 覆盖视口底部，锚点目标超出 maxScrollTop），因此 smooth/auto 最终都停在 maxScrollTop；先前用「强制全局 scrollTo auto」得到的 29px 是对照实验设计缺陷（同时改写了补偿路径），不成立。

**M3 终版（Lead 追认，Test Agent 实施）**：

- **M3a**：内容运动方向反转 ≤ 1（容差 2px）。
- **M3b**：对每帧 i≥4，`|ΔscrollTop_i| ≤ Σ_{j=0..3} ΔcolScrollHeight_{i−j} + 8px`（4 帧 ≈ 67ms 延迟预算 + 8px 余量；实测原实现 0 超调，余量 8px）。
- **M3c**：对每帧，`|scrollTop_i − max(0, colScrollHeight_i − clientHeight_i)| ≤ 64px`（契约原式因「未填满视口阶段 scrollTop 恒为 0」固定累计 −302px 常数偏移而恒不可绿，Test Agent 修订式已追认；实测峰值 47px，余量 17px）。
- 删除绝对 `maxJump ≤ 32px`（原判定：度量把内容增长与滚动质量混为一谈，TEST-DEFECT）。
- 非空转证明：M3b/M3c 均能捕获「跟随失效/大跳变」；同场景 A/B 数据表明 K=4 窗口下 auto 版超调 20px、原版 0px，具备判别力。

### 7.2 三路对抗审核裁定与修复（Lead，2026-10-15）

L3-A（状态流转/回归）、L3-B（交互/视觉/性能）、L3-P（UI 生产就绪/证据审计）三路独立审核均判 **FAIL**，findings 落盘 `.dev-team/review-l3a-wave11.json` / `review-l3b-wave11.json` / `review-l3p-wave11.json`。Lead 逐条复核后裁定：

**必改（代码，Executor 写域）**

- **F1** §2.4 mode 一致性：`isRuntimeStatusVisible` mode-blind → `sessionOperation='initializing'` 窗口内（`runtime.ts:2430` 早于 `resetSessionRuntimeState` 清空 messages）缓存中 pending+有内容的 assistant 会让「正在加载最近对话…」被隐藏。修法：仅 `agentRuntimeStatus.mode === "responding"` 才可能隐藏；connecting/error/waiting_approval/retrying/tool_* 恒可见。
- **F2** 交互边界：可见态 `pointer-events: auto` + `w-full` 使 composer 上沿以上 ~24px 全宽条带吞掉消息点击/选中 → 恒 `pointer-events: none`。
- **F3** label 闪跳：`:key="displayLabel"` 销毁重建 + 仅淡入 → 去掉 `:key` 与 180ms 淡入，原位替换。§2.7 措辞同步修订为「不得出现整行闪跳（禁止先消失后出现）」。
- **F4** 窄屏遮挡：320×860 下展开态按钮（`w-[110px]`）与状态文字重叠 46px → 预留空间由 `pr-12`(48px) 改为 `pr-[7.5rem]`(120px)。
- **F5** 图标可辨识度：轨道环对比度实测 1.80:1（<3:1）→ 图标色 `text-stone-500 dark:text-stone-300`，环 `1.5px dashed` 且去 `opacity:.5`；新增基准 `icon_ring_contrast_min_ratio=3.0`。
- **F6** 渲染成本：去掉常驻 `will-change: transform`；隐藏态 `animation-play-state: paused`；ring 时长收紧到 1200–1600ms（全部 mode 仍须落在 1200–2400ms）。

**必改（测试，Test Agent 写域）**

- **T-a** M3a 容差 2 → **8px**：实测跟随引擎固有 ±6px 反向微抖（idx60–63）使 tol=2 时反转 = 2（flaky）；tol≥6 两次运行均为 0。8px ≈ 1/3 行高，作为「可感知位移」阈值；基线 ±23.8px/44 次反转仍被捕获。NFR 字段 `streaming_content_monotonic_tolerance_px=2.0` 已 supersede。
- **T-b** 新增回归：`sessionOperation='initializing'`/`error` 且存在 pending+有内容的 assistant 时，槽位必须 `data-visible="true"`（覆盖 L3-A 可达反例）。
- **T-c** 新增 e2e：320×860 + 展开态「滚动到最新」按钮，断言状态条文本盒与按钮盒水平不重叠 + 截图。
- **T-d** 门禁收据必须在**最终冻结产物**上取得原生 EXIT=0（L3-P 否决「仅探针达标、冻结 spec 无 EXIT=0」的放行方式）。

**追认（不判缺陷）**：L3-B 关于「环自转视觉不可辨（0.07px/帧）」为 non-machine 观感项，已通过 F5（对比度）+ F6（提速到 1200–1600ms）缓解；`:global(.dark)` 选择器退化问题在基线 commit 已存在，属范围外存量缺陷，另开单。

### 7.3 M3c 阈值裁定（Lead，2026-10-15）

- **64px → 96px（硬上界）**。依据：Test Agent 16 次独立 e2e 实测 M3c 峰值滞后分布 **39–67px**，其中 1 次 67 > 64（flaky）。物理下界 = 浏览器忙帧把多行内容合并进一帧时的内容增长量（1 行 23.8px / 2 行 47.6px / 实测最大 ≈2.8 行 67px）→ 阈值必须覆盖多行合并包络。取 4 行包络 **96px**，对实测峰值留 43% 余量。
- 判别力保留：跟随失效/停摆时滞后无界增长；M3b（无超调，K=4 帧 + 8px）与 M3a（反转 ≤1，容差 8px）仍收紧。
- NFR 字段 `streaming_follow_lag_max_px` 已登记 supersede（`.dev-team/nfr-baseline-ui-wave11.json`）。
