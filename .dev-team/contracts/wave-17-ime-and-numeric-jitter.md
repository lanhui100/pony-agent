# Wave-17 契约矩阵：IME 组合态回车守卫 + 动态数字抖动消除

- 基线 commit：`5d7d691`（工作区干净）
- 分级：**B-lite**（机械依据：跨 2 个模块域、6 个文件；含交互状态语义；无网络/线程/锁面变更）
- 来源：open-claude-ui（CDS 逆向工程）6 项借鉴建议，经 3 路独立对抗评审（A 交互物理 / B 设计令牌 / C 状态机与风险）优化后收敛为 2 类**因果关联真实缺陷**

---

## 1. 优化结论（评审后取舍）

| 原建议 | 判定 | 依据（file:line 物理证据） |
| :--- | :--- | :--- |
| S1-a CJK IME 互锁守卫 | **采纳（部分）** | 主输入框已有守卫 `HomeWorkspace.vue:1369`（被 `tests/HomeWorkspace.spec.ts:2806` 冻结）；但 ASK 三处回车入口与两处重命名输入框共 5 处裸奔 |
| S1-b 单边框原则 | 拒绝（已实现） | composer 外壳无 border（`WorkspaceComposer.vue:836`），内部 `.composer-trigger{border:none}`(:1437)、`.composer-search-input{border:none}`(:1527)、`ui/Button.vue:13` 无 border；唯一带 border 的附件胶囊(:992) 承载 error 状态色，删除即回归 |
| S2 滚动防抢占 150ms 冷却 | 拒绝（现有更强） | `useTimelineAutoScroll.ts` 为「无限期闩锁 + 版本号作废」：pause(482)、nextTick 校验(519)、rAF 校验(576)、lerp 逐帧中止(657)、意图源 wheel/touchstart/pointerdown(1026-1028)、键盘(784)、3000ms 空闲+内容+轮次三重恢复(842)、同帧补偿(532)。150ms 方案是其真子集，照搬即回退 wave-11/14 |
| S3-a Ask 三态闭环 | 拒绝（已实现） | `ask-plan.ts:6`（pending/consumed/cancelled/expired）、`stores/ask.ts:58`（loading/error/answering/cancelling）、`:77` stale-CAS 分流、`AskUserToolCallCard.vue:97` terminalState、`WorkspaceTurnItem.vue:437-440` 耗时徽标+三态图标 |
| S3-b 三阶决议动作阵列 | 拒绝（跨层新增） | 后端无工具级永久授权落点：`ask_control.rs:74-85` answer 为不透明 `Option<Value>`；`src-tauri/src/lib.rs:1208-1211` 仅 4 条 ask 命令；唯一持久 Store 是路径域 `path_permission.rs:162`。前端伪造第三颗按钮 = 静默谎报 |
| S4-a 思考折叠互斥锁 | 拒绝（缺陷不存在） | `WorkspaceTurnItem.vue:182` `shouldOpenReasoningBlock` 恒 false → `:361` `:open` 绑定值永不变化 → Vue 不 patch，用户展开态不会被重置；强制展开会覆盖 `HomeWorkspace.vue:70/1525/1601` 的 localStorage 显式偏好 |
| S4-b 暖光氛围扫光 | 拒绝（已实现且参考无 a11y） | shimmer `AgentRuntimeStatusBar.vue:278-301`、呼吸 `:263-274` 已存在并已做 `prefers-reduced-motion` 回退(:315-331)；参考 `ReasoningAmbientGlow.tsx` 该媒体查询命中 0 次，其 canvas rAF(:217) 与 box-shadow 光晕(:78) 会争抢帧预算 |
| S5-a Surface 0-3 令牌阶梯 | 拒绝（大范围重构） | 四层阶梯已事实上存在（`HomeSidebar.vue:67` bg-white/62、`HomeStatusPanel.vue:90` #f6f0e8、`AskPanel.vue:117` #fdf6e7、`WorkspaceTurnItem.vue:737` 0.5px 发丝）；迁移面 19 文件 / 96 hex 字面量；且会作废 `AgentRuntimeStatusBar.vue:117` 已冻结的 4.11:1 对比度测量 |
| S5-b Retina 0.5px 发丝 | 拒绝（已实现 + DPR=1 退化） | `ring-1` 已 14 处（`ui/DropdownMenu.vue:62`、`ProviderConfigPage.vue:1571`）；`border-[0.5px]` 在 DPR=1（Windows 100% 缩放）被取整/抗锯齿 |
| S5-c Tabular Nums | **采纳** | 全仓仅 1 处（`AgentRuntimeStatusBar.vue:140`）；已核实 8 处真实原地抖动位点 |
| S6 流式 Diff 视图 + 分栏 resizer | 拒绝（非因果新功能） | `src/` 内 `resizer\|split-pane\|ResizablePanel\|DiffView` 零命中；`markdown.ts:562` 代码块无行号/gutter；需新增 Rust 侧 diff 载荷契约 |

**净结论**：6 项建议 → 采纳 2 类缺陷修复，拒绝 9 个子项（含 4 项"已实现"、1 项跨层、2 项大范围重构/DPR 退化、1 项新功能）。

---

## 2. 冻结的验收契约

### C1 — IME 组合态回车守卫（缺陷类：中文/日文/韩文选词回车误触提交）

**根因**：Vue 的 `.enter` 修饰符只比对 `event.key`，不检查 `isComposing`；组合态确认候选词的回车会被当作提交。

**冻结断言**：
- C1-1 当 `KeyboardEvent.isComposing === true` 且 `key === 'Enter'` 时，ASK 自定义输入框（`data-testid=composer-ask-custom-input`）**不得**触发回答提交。
- C1-2 同上，ASK textarea（`data-testid=composer-ask-textarea`）不得提交。
- C1-3 同上，ASK 选项按钮（`data-testid=composer-ask-option-item-N`）不得触发 `handleOptionEnter`。
- C1-4 `keyCode === 229`（旧式 IME 标记）与 `isComposing === true` 等价生效。
- C1-5 **反向断言（防吞）**：`isComposing === false` 且 `key === 'Enter'` 时，提交**必须**照常发生（守卫不得过度拦截）。
- C1-6 会话重命名输入框（`data-testid=session-rename-input`）组合态回车不得 `submit-rename`。
- C1-7 工作区重命名输入框（`data-testid=workspace-rename-input`）组合态回车不得 `submitRename`。
- C1-8 "显示思考"菜单项（`data-testid=reasoning-visibility-toggle`）组合态回车不得切换。

**物理证据要求**：`npx vitest run` 红相 EXIT≠0 → 绿相 EXIT=0，且红相失败必须为断言失败（SUT 栈），非 setup/import 失败。

### C2 — 动态数字抖动消除（缺陷类：原地数值宽度变化推移相邻元素）

**根因**：Token/耗时/速度等在同一文本节点原地变化，且宿主为 `justify-around` / `flex-nowrap` 行，位数变化会横向推移相邻元素。

**冻结断言**：
- C2-1 `HomeStatusPanel.vue` 的 4 个 token 指标位（输入/输出/缓存/速度）与 3 个计数位（轮次/模型调用/工具调用）承载元素的计算样式 `font-variant-numeric` 必须为 `tabular-nums`。
- C2-2（**2026-10-16 Lead 裁决修订，见 §6 修订记录**）真实浏览器物理测量，非 class 字符串断言：
  - **C2-2a（红相驱动）**：位点计算样式 `font-variant-numeric === "tabular-nums"`。
  - **C2-2b（tabular-nums 的真实物理语义）**：**等位数异字形**宽度漂移 ≤ 0.5px —— `1` / `9` / `0` 之间，以及 `1111` / `9999` / `0000` 之间。
  - **C2-2c（用户可见缺陷本体）**：数值**位数变化**（`9` → `12` → `1234`）时，**相邻指标位的包围盒左边缘位移 ≤ 0.5px**（由容器 `min-w` 保证，属 task-4 交付面）。
  - **C2-2d（合理性哨兵）**：三档位数字自身宽度按 `9` ≤ `12` ≤ `1234` 单调不减（仅记录 + 单调性断言，**不**断言其相等 —— 位数增加必然加宽，这是字体物理事实）。
- C2-3 `HomeTracePanel.vue` 的 turn 级与 entry 级指标位（token/t·s⁻¹/ms）计算样式为 `tabular-nums`。
- C2-4 `AskPanel.vue` 过期倒计时位（`data-testid=ask-expired-hint` 同行的 `formatExpiry` span）计算样式为 `tabular-nums`。
- C2-5 不新增 `@theme`、不改任何颜色、不改 flex 结构、不动 `WorkspaceTurnItem.vue:437`（已用 `invisible` 占位预留宽度）与 `TurnDial.vue:101`（静态序号）。

### C3 — 浏览器真实链路门禁（Tier-1 前端）

- C3-1 真实 Chromium 加载构建产物，核心旅程可达（`app-layout-shell` + `workspace-composer-input` 可见）。
- C3-2 **Console Error = 0**（未捕获异常 / console.error 计数为 0）。
- C3-3 至少 1 张关键旅程截图落盘 `.dev-team/evidence/wave-17/`。

### C4 — L2-AT 对抗破坏（确定性）

- C4-1 **事件对象边界穿透**：`event === undefined`、`isComposing === undefined`、`keyCode === 0`、传入 `MouseEvent` 时守卫不得抛异常，且非组合态仍提交。
- C4-2 **组合序列畸形**：`compositionstart → keydown(Enter,isComposing=true) → compositionend` 交错序列下，仅非组合态回车产生提交。
- C4-3 **确定性并发**：固定迭代 200 次交替注入组合/非组合回车，提交计数必须**精确等于**非组合态次数（无 sleep、无概率等待）。

---

## 3. 写域划分（正交，路径段规范化）

| 任务 | write_scopes |
| :--- | :--- |
| task-1 protocol | `.dev-team/` |
| task-2 test (L2-T) | `tests/acceptance/wave17/`、`tests/e2e/wave17-*.spec.ts` |
| task-3 executor-a | `src/lib/ime.ts`（新增）、`src/components/chat/WorkspaceComposer.vue`、`src/components/HomeSessionSidebarRow.vue`、`src/components/HomeSessionSidebar.vue` |
| task-4 executor-b | `src/components/HomeStatusPanel.vue`、`src/components/HomeTracePanel.vue`、`src/components/AskPanel.vue` |

task-3 与 task-4 文件集**交集为空**；二者共享 `npm run typecheck` / `vitest` 读取面，写面正交 → 允许并行（k=2）。

## 4. 基线缺陷登记（不属本波，不得计入回归）

- **BASE-E2E-1**：`npm run test:e2e` 在基线即 `8 failed / 12 passed`（全新构建后复现）。根因：`tests/e2e/browser-preview.spec.ts` 断言 `session-sidebar-session-list`，而当前侧栏结构已变更，该 testid 在浏览器预览模式下不存在。属存量测试与 UI 漂移，非本波引入。本波 C3 采用**独立**的 wave-17 浏览器门禁 spec，不依赖该存量 spec 转绿。

## 5. 明确不做（防夹带）

- 不新增 `@theme` / 设计令牌层 / 颜色迁移
- 不新增 diff 视图 / 分栏 resizer / iframe bridge
- 不改 Rust 侧任何文件、不新增 IPC 命令
- 不删除孤儿组件 `src/components/AskPanel.vue`（该组件未被任何生产代码挂载，仅测试引用）——登记为独立 backlog，交由 `code-hygiene` 专项处理
- 不 bump 版本号（本波非发布）

---

## 6. 修订记录（Lead 一票裁决）

### REV-1（2026-10-16，红相锚定前）：C2-2 口径修订

**触发**：Test Agent 在红相阶段以真实浏览器实测证明原 C2-2 字面口径物理不可满足，并升级为契约缺陷。

**实测证据（Chromium 基线）**：
- 三档位数字自身宽度：`9` = 23px / `12` = 26px / `1234` = 40px（Δ = 17px）
- 注入 `tabular-nums` 后：`9` = 23px / `12` = 30px / `1234` = 44px（Δ = 21px，仍 ≠ 0）
- 等位数异字形：`1/9/0` 与 `1111/9999/0000` 基线 Δ = 4px / 3px，注入后 Δ = 0

**根因**：原措辞把两个不同的物理现象混为一谈 ——
1. **字形宽度抖动**（同位数、不同数字的 advance width 不等）→ `tabular-nums` 可完全消除；
2. **位数重排**（1 位 → 4 位必然变宽）→ 字体层面**不可**消除，只能由容器 `min-w` 吸收。

**裁决**：原 C2-2 拆为 C2-2a/b/c/d（见 §2），其中 **C2-2c（相邻指标位左边缘位移 ≤ 0.5px）才是用户可见缺陷的本体**，且其修复依赖 task-4 已授权的容器 `min-w`（C2-5 从未禁止 `min-width`，仅禁止新增 `@theme` / 颜色迁移 / flex 结构变更 / 动已缓解位点）。

**影响面**：仅 `tests/e2e/wave17-browser-gate.spec.ts` 的断言口径；C1、C2-1、C2-3、C2-4、C3、C4 全部不变。红相尚未锚定，故属**锚定前澄清**，不触发契约演进重冻结流程。

**未采纳**：坚持原字面口径 —— 该口径永不可绿，会迫使实施者伪造证据或引入超范围改动。

### REV-2（2026-10-16，绿相阶段）：L2-AT 身份断言缺陷修订

**触发**：Executor A 在绿相阶段发现 `tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts` 两处身份断言恒假，并给出物理探针证据。

**现象**：`:582` `expect(call[0]).toBe(ask)` 与 `:609` `answerSpy.mock.calls.every(call => call[0] === ask)` 失败，形态 `Received: serializes to the same string`。

**根因（探针实测，非推断）**：`ask` 经 `WorkspaceComposer.vue:563` 的 `activeAsk` computed 从 Pinia state 读取，返回 **Vue reactive Proxy**；spec 持有 `createPendingAsk()` 的**原始对象**，`Object.is(proxy, raw) === false`。探针报告：`{"storeSlotIsRaw":false,"activeAskIsRaw":false,"activeAskIsProxy":true,"toRawActiveAskIsRaw":true,"deepEqual":true}`。

**红相有效性论证（关键）**：该 spec 在红相时先于 `toHaveBeenCalledTimes(100)` 失败（`expected 100 times, but got 200 times`），身份断言**从未被执行**，故 C4-3 的红相证据不因此缺陷失效。Lead 在 Join Barrier 以 `git worktree` 从 tag `wave-17-red-anchor` 复跑修订后 spec 做物理复核。

**裁决**：
- **采纳**：修订为 `toRaw` 身份等价或深等价（二选一），且必须保留证伪能力（提交了错误 ask 时断言须失败）。
- **拒绝**：在 `submitAskAnswer` 内加 `toRaw(ask)` —— 属与 C1 因果无关的行为变更，违反「严禁夹带重构」。

**熔断计数**：按 skill 规定，**用例自身缺陷重写豁免熔断计数**。

**影响面**：仅 2 处断言；C1/C2/C3 与契约语义均不变。

### REV-3（2026-10-16，Join Barrier 阶段）：C3-3 物理截图凭据不成立

**触发**：Lead 在 Join Barrier 目视复核 `.dev-team/evidence/wave-17/wave17-browser-gate-home.png`，发现画面为**启动闪屏**（PONY AGENT + 加载进度条），无工作区外壳、无侧栏、无输入框。

**根因（Lead 定位）**：
- `index.html:17` 注入 `#app-loading-screen` 全屏闪屏遮罩（`position:fixed` 覆盖层）。
- `src/main.ts:91-96`：应用挂载后 `classList.add("fade-out")` → 过渡结束才 `splash.remove()`。
- Playwright `toBeVisible()` 对「被覆盖但未隐藏」的元素仍判可见 → C3-1/C3-2 断言成立，但 `openHome()` 返回时闪屏仍覆盖在最上层，故截图拍到闪屏。
- 原 C3-3 断言仅校验「文件存在且 >10KB」，**无法证伪「拍到闪屏」** → 弱证据，不满足「物理凭证先决」。

**裁决**：修订 C3-3，在截图前显式等待 `#app-loading-screen` 从 DOM 移除，并补充具备**证伪能力**的断言（防止再次拍到闪屏）。属**测试自身缺陷重写，豁免熔断计数**。

**影响面**：仅 C3-3 用例；C3-1/C3-2/C1/C2 断言不变。



