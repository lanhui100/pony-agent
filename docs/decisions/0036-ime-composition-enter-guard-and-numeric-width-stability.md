# 0036 IME 组合态回车守卫与动态数字位宽稳定

Status: implemented

## 背景

前端存在两类用户可见缺陷（wave-17 修复，来源为对 `open-claude-ui`（Claude Design System 逆向工程示例库）6 项借鉴建议的对抗评审收敛）：

1. **IME 组合态回车误触提交**：Vue 的 `.enter` 修饰符只比对 `event.key`，**不检查 `isComposing`**。中文/日文/韩文用户在拼音或假名选词时按回车确认候选词，会被当作提交。主输入框路径已有守卫（`HomeWorkspace.vue:1369`，被 `tests/HomeWorkspace.spec.ts:2806` 冻结），但另有 **6 处回车入口完全裸奔**：ASK 自定义输入框、ASK textarea、ASK 选项按钮、「显示思考」菜单项、会话重命名输入框、工作区分组重命名输入框。
2. **动态数字横向抖动**：状态栏与 trace 面板的 token 计数、耗时、生成速度在同一文本节点原地变化，宿主为 `justify-around` / `flex-nowrap` 行，位数变化（`9` → `12` → `345`）会横向推移相邻位点。全仓此前仅 1 处使用 `tabular-nums`（`AgentRuntimeStatusBar.vue:140`），本项是该已修复缺陷类别的一致性外推。

同时，评审**否决**了 9 个被提建议子项（4 项已实现、1 项跨层新增、2 项大范围重构/DPR 退化、1 项非因果新功能），理由与 file:line 证据见 `.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §1。

## 决策

1. **新增唯一接缝 `src/lib/ime.ts`**，导出纯谓词 `isImeComposingEvent(event: Event | null | undefined): boolean`：仅当 `event instanceof KeyboardEvent` 且（`isComposing === true` 或 `keyCode === 229`）时返回 `true`；`null` / `undefined` / 非键盘事件 / 无 DOM 全局一律返回 `false` 且**不得抛异常**。
2. **6 处回车入口全部接入守卫**：新增一行包装函数（谓词早退 + 委托原函数），**不改动既有 `submitAskAnswer` / `submitRename` / `toggleReasoningVisibility` 的签名与 `@click` 引用**。`handleOptionEnter` 增加可选 `event?: KeyboardEvent` 参数，守卫置于**任何状态写入之前**（避免「提交被吞但选中态已污染」的半修复）。
3. **鼠标点击路径不加守卫**：守卫语义是区分「Enter 作为 IME 候选确认」与「显式提交」；鼠标点击无候选确认语义，`@click` 保持直通。
4. **动态数字位点统一 `tabular-nums`**：`HomeStatusPanel.vue`（7 位点）、`HomeTracePanel.vue`（turn 级 + entry 级指标行）、`AskPanel.vue`（过期倒计时）。
5. **位数重排由容器位宽吸收，而非依赖 `tabular-nums`**：`tabular-nums` 只能统一**等位数**不同字形的 advance width，无法让 1 位与 4 位等宽（实测：仅加 `tabular-nums` 反而使相邻位位移由 17px 升至 21px；加 `min-w` 后才归零）。因此数字承载元素同时获得固定最小宽度 + 右对齐。
6. **不引入设计令牌层、不迁移颜色、不新增 diff 视图/分栏、不改 Rust、不 bump 版本**。

## 候选方案与取舍
- **A. 在 `submitAskAnswer` 内加 `toRaw(ask)` 修复测试身份断言**（L2-AT 用例 `toBe(rawObject)` 因 Pinia reactive Proxy 恒假）：属与 C1 因果无关的行为变更，违反「严禁夹带重构」—— **否决**，改为修订测试断言为 `toRaw` 身份等价。
- **B. 抽共享 composable 承载 6 处守卫**：6 处入口分属 3 个组件目录，抽 composable 会扩大写域与回归面；而纯谓词 `src/lib/ime.ts` 已是零副作用最小单元，组件侧仅保留一行包装 —— **否决 B，采纳单函数接缝**。
- **C. 不新增接缝，在 6 处各写一份内联 `if (e.isComposing || e.keyCode === 229)`**：与 `HomeWorkspace.vue:1369` 现状一致，但会产生 7 份重复判定且无单一可测契约 —— **否决**。代价是 `ime.ts` 的判定比既有内联更严格（多一道 `instanceof` 闸门 + `=== true`），差异仅在跨 realm 事件对象与真值非布尔 `isComposing` 两档；审查者实测本仓 `src/` 内 iframe/webview/`createEvent` 零命中，**当前无可达放行路径**，登记为 PA-136。
- **D. 照搬 `open-claude-ui` 的 `border-[0.5px]` 发丝边框**：`border-width` 在 DPR=1（Tauri Windows 100% 缩放，主流配置）被取整/抗锯齿，边界视觉退化；且仓内 `ring-1` 已 14 处达成同等收益 —— **否决**。
- **E. 引入 `@theme` + Surface 0-3 四层令牌阶梯**：四层阶梯在本项目已事实上存在（`bg-white/62..97` 叠加 `bg-stone-50..200` 与 `#faf6ef/#f6f0e8/#fdf6e7` 米色卡片），缺的是命名而非分层；迁移面 19 文件 / 96 个 hex 字面量，且会作废 `AgentRuntimeStatusBar.vue:117` 已冻结的 4.11:1 对比度测量 —— **否决**。
- **F. 生成期间强制展开思考折叠面板并禁用折叠（互斥锁）**：`WorkspaceTurnItem.vue:182` 的 `shouldOpenReasoningBlock` 恒 `false`，`:361` 的 `:open` 绑定值永不变化，Vue 不会 patch，**所要修复的状态失步不存在**；强制展开反而会覆盖 `HomeWorkspace.vue:70/1525/1601` 的 localStorage 显式用户偏好 —— **否决**。
- **G. 三阶决议动作阵列（仅本次通过 / 永久白名单 / 拒绝）**：后端无工具级持久授权落点（`ask_control.rs:74-85` 的 `answer` 为不透明 `Option<Value>`；`src-tauri/src/lib.rs:1208-1211` 仅 4 条 ask 命令；唯一持久 Store 是路径域 `path_permission.rs:162`）。前端伪造第三颗按钮会把 `always_allow` 当工具答案回灌模型，形成「UI 承诺了未发生的授权」的静默谎报 —— **否决**，转 backlog PA-131。

## 影响

- **用户可见**：中文/日文/韩文用户选词回车不再误提交 ASK 回答、误提交重命名、误切「显示思考」；状态栏 token 计数/速度与 trace 面板指标在流式更新时不再横向推移相邻内容。
- **契约面**：新增模块级纯函数 `src/lib/ime.ts`（3 个组件 + 1 个测试目录消费）；Vue 模板事件绑定形态由裸处理器改为守卫包装，组件对外 props/emits 零变更。
- **测试面**：新增 3 个冻结验收文件（`tests/acceptance/wave17/` 2 个 + `tests/acceptance/wave17-adversarial/` 1 个，31 例）与 1 个真实浏览器门禁（`tests/e2e/wave17-browser-gate.spec.ts`，4 例）；红相锚定 `f060c8d`，绿相检查点 `49fd118`。
- **已知限制与债务**：`HomeWorkspace.vue:1369` 的守卫粒度过宽（组合态无条件早退，会吞掉 Ctrl/Cmd+Z 撤销）属**存量缺陷**，本波未触及（PA-135）；`tests/e2e/browser-preview.spec.ts` 存在存量基线失败（PA-134）。
