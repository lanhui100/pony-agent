# Wave-52 契约矩阵 — 设置页「软件自动更新」卡片（A 级 / 单组件 UI 文案与布局）

需求（用户原话）→ 契约映射：
- R1 标题「软件更新」→「软件自动更新」→ C1
- R2 开关 Switch 放在同行最右端 → C2
- R3 上次检查日期放在「当前版本」之后 → C3

基线 commit：`e43f8c3`（chore(release): bump version to 0.1.163）
分级依据：单组件文案/布局 + 2 个文件；无外部网络新增、无并发原语、diff < 100 行 ⇒ A 级。
写域：Executor = `src/components/config/ConfigGeneralSection.vue`；Lead/Test = `tests/`、`tests/e2e/`、`.dev-team/`。

## 冻结契约（红相锚定后只读）

| 编号 | 断言（机器可判） | 判定层 |
| :--- | :--- | :--- |
| C1 | 分区标题元素 `data-testid="config-update-section-title"` 文本严格等于 `软件自动更新` | jsdom + 真实浏览器 |
| C2 | 自动检查行 `data-testid="config-update-autocheck-row"` 内，Switch `config-update-autocheck-switch` 为最右端元素：真实浏览器 `|switch.right − row.right| ≤ 2px`，`label.left − row.left` ≤ 2px，且 `switch.left ≥ label.right` | jsdom（DOM 序）+ 真实浏览器（几何） |
| C3 | `config-update-last-checked` 位于版本行 `config-update-version-row` 内、DOM 序在版本 chip `config-update-current-version` 之后；几何 `lastChecked.left ≥ chip.right`，两者垂直中心差 ≤ 4px（同一行） | jsdom + 真实浏览器 |
| C4 | 行为不变量：切换开关仍写 `pony-agent.update-prefs.v1 = {autoCheck:false}` 并同步 store；既有 testid 全保留；不新增网络/IO/定时器 | jsdom（存量用例） |
| C5 | 真实浏览器内点击 Switch 后 `aria-checked` 由 false→true，且 `pony-agent.update-prefs.v1` 写为 `{autoCheck:true}` | Playwright |
| C6 | 真实浏览器：Console Error = 0、未捕获异常 = 0；截图落 `.dev-team/evidence/wave-52/`（> 10KB） | Playwright |
| C7 | 视觉基线：变更前截图 `tests/visual-baseline/wave-52-config-update-section-before.png`；变更后截图 `.dev-team/evidence/wave-52/after.png`。**像素级自动 diff 不适用**（本需求刻意改变卡片高度，下游内容必然位移；自动 diff 只会产生必然假红），以 C2/C3 的真实浏览器几何断言作为机器门禁，两张截图作为人工对眼证据。理由入档 `.dev-team/wave52-visual-gate-deviation.md` | 人工证据 + 几何断言 |

## 非目标（Non-Goals）
- 不改更新检测业务语义（store / `lib/update-check.ts` / Tauri updater 一律不动）。
- 不改卡片其余 UI（检查中/新版本/最新/无发布/错误五态、签名升级入口、隐私披露文案）。
- 不新增设置项、不改持久化 key、不动 Rust 侧。

## 允许的实现自由度（实现方只读）
- 行容器 class 可自行选择（Tailwind `flex items-center justify-between` 等），只要满足 C2/C3 的几何与 DOM 序断言。
- 「上次检查」展示条件沿用现值（`lastCheckedLabel !== null` 才渲染），文案前缀 `上次检查：` 保持不变。
