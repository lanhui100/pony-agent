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
| R3 streaming 期间不显示状态消息 + 上推平滑 | M2/M3 | 流式期间 `[data-testid="workspace-runtime-status-slot"]` 的 `data-visible="false"` 且计算 `opacity === "0"`；消息内容逐帧单调上移（反转 ≤ 1 次，单帧位移 ≤ 32px） |

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
7. **标签切换**：label 文本变化需交叉淡入（≤ 200ms），不得出现整行闪跳。
8. **错误态**：`mode === "error"` 保留 `AlertCircle`，不带 orbit 类，且 `data-visible` 语义不变。
9. **纯 CSS 动效**：图标动画只允许 CSS keyframes（禁止 JS 定时器 / rAF 逐帧驱动）。
10. **既有约束保持**：状态条根元素类名不含 `bg-*` / `border-*` / `animate-pulse`（既有单测已冻结）。
11. **testid 不变**：`workspace-runtime-status-slot`、`agent-runtime-status-bar`、`agent-runtime-status-label`、
    `agent-runtime-status-elapsed`、`agent-runtime-status-icon` 全部保留；新增 ring/dot 子 testid。

## 3. UI-NFR 基准（`.dev-team/nfr-baseline-ui-wave11.json`）

- 状态条视口 Y 极差 ≤ 1px（流式 / 新消息插入两场景）。
- 流式内容位移反转 ≤ 1 次；单帧位移 ≤ 32px。
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
- 不修改 `src/lib/runtime/status.ts` 的状态解析语义与任何文案。
- 不重构 composer 结构、不调整 `COMPOSER_BUFFER_PX`。
- 不处理移动端/窄屏专属布局（≤ 640px 仅要求不破版）。
