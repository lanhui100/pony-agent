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
| C2 | 标题行 `data-testid="config-update-header-row"` 内，标题文案在左、Switch `config-update-autocheck-switch` 为该行最右端元素：真实浏览器 `|switch.right − headerRow.right| ≤ 2px`、`|switch.right − 卡片右边缘| ≤ 2px`、标题与 Switch 垂直中心差 ≤ 2px | jsdom（DOM 序）+ 真实浏览器（几何） |
| C2b | 卡片方框内不得再出现「自动检查更新」文案，`config-update-autocheck-row` 不得存在；整个 section 内 Switch 数量恰为 1（即标题行那个） | jsdom + 真实浏览器 |
| C3 | `config-update-last-checked` 位于版本行 `config-update-version-row` 内、DOM 序在版本 chip `config-update-current-version` 之后；几何 `lastChecked.left ≥ chip.right`，两者垂直中心差 ≤ 4px（同一行） | jsdom + 真实浏览器 |
| C4 | 行为不变量：切换开关仍写 `pony-agent.update-prefs.v1 = {autoCheck:false}` 并同步 store；既有 testid 全保留；不新增网络/IO/定时器 | jsdom（存量用例） |
| C5 | Switch 可及名为「软件自动更新」（`aria-label`，因卡片内已无可视文案作标签）；真实浏览器内点击后 `aria-checked` 由 false→true，且 `pony-agent.update-prefs.v1` 写为 `{autoCheck:true}` | Playwright |
| C6 | 真实浏览器：Console Error = 0、未捕获异常 = 0；截图落 `.dev-team/evidence/wave-52/`（> 10KB） | Playwright |
| C7 | 视觉基线：变更前截图 `tests/visual-baseline/wave-52-config-update-section-before.png`；变更后截图 `.dev-team/evidence/wave-52/after.png`。**像素级自动 diff 不适用**（本需求刻意改变卡片高度，下游内容必然位移；自动 diff 只会产生必然假红），以 C2/C3 的真实浏览器几何断言作为机器门禁，两张截图作为人工对眼证据。理由入档 `.dev-team/wave52-visual-gate-deviation.md` | 人工证据 + 几何断言 |

## 非目标（Non-Goals）
- 不改更新检测业务语义（store / `lib/update-check.ts` / Tauri updater 一律不动）。
- 不改卡片其余 UI（检查中/新版本/最新/无发布/错误五态、签名升级入口、隐私披露文案）。
- 不新增设置项、不改持久化 key、不动 Rust 侧。

## 允许的实现自由度（实现方只读）
- 行容器 class 可自行选择（Tailwind `flex items-center justify-between` 等），只要满足 C2/C3 的几何与 DOM 序断言。
- 「上次检查」展示条件沿用现值（`lastCheckedLabel !== null` 才渲染），文案前缀 `上次检查：` 保持不变。

## 契约演进 v2（2026-10-27，Lead 一票裁决，用户反馈驱动）

用户反馈原话：「开关应该是 section『软件自动更新』同行最右端，方框内不应该再有『自动检查更新』」。

- **C2 变更**：v1 的「卡片内自动检查行最右端」作废 → v2 为「**section 标题行**最右端」；行锚点 `config-update-header-row`。
- **C2b 新增**：卡片方框内删除「自动检查更新」文案行（`config-update-autocheck-row` 必须不存在），section 内 Switch 恰为 1 个。
- **C5 变更**：因卡片内已无可视标签，Switch 必须自持 `aria-label="软件自动更新"` 保可及名。
- **C1/C3/C4 不变**；`config-update-autocheck-switch` 等既有 testid 继续保留（存量用例 `tests/ConfigGeneralSectionUpdate.spec.ts:186` 依赖其点击行为）。
- 演进纪律：测试由 Lead 迁移并**重新锚定红相**，Executor 只在红相之后实施。

## 契约演进 v3（2026-10-27，Lead 一票裁决，用户反馈驱动）

用户反馈原话：「检查更新按钮点击后发现新版本，则转换为『立即升级』按钮，下方的立即升级按钮去除」。

| 编号 | 断言（机器可判） | 判定层 |
| :--- | :--- | :--- |
| C8 | **主按钮模式机**（唯一元素 `config-update-check-button`，位于版本行右端）：`signedInstalling` → 「正在升级…」+ spinner + disabled；`signedUpdateAvailable` → 「立即升级」+ `ArrowUpCircle`，点击走 `installSignedUpdate()`；`updateChecking` → 「检查中…」+ spinner + disabled；其余 → 「检查更新」+ `RefreshCw`，点击走 `checkForUpdateNow()` | jsdom（四态） |
| C9 | 方框内**不得再有**独立安装按钮 `config-update-install-button`；版本行只允许 1 个 `button`（即主按钮） | jsdom + 真实浏览器 |
| C10 | 安装进度条 `config-update-progress` 保留（`signedInstalling && signedProgress !== null` 时渲染，宽度随进度变化） | jsdom |

- C4 行为不变量不变：非安装态主按钮仍是「检查更新」并触发检查（存量 `tests/ConfigGeneralSectionUpdate.spec.ts` 全绿）。
- 契约演进纪律同 v2：测试由 Lead 迁移并**重新锚定红相**（本次红相：`ConfigGeneralSectionSignedUpdate.spec.ts` 5 例红），Executor 只在红相之后实施。
- 交互语义：点击「检查更新」后若检测到**可安装的签名更新**（`signedStatus === 'available'` 且候选非空），同一按钮原位变「立即升级」，不再另起一行按钮。
