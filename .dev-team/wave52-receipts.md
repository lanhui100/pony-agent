# Wave-52 交付收据 — 设置页「软件自动更新」卡片

- 任务来源（用户原话）：桌面端设置中「软件更新」改为「软件自动更新」；开关 Switch 放在同行最右端；上次检查日期放在当前版本之后。
- 分级：A 级（单组件文案/布局；diff +20/−18，1 个文件；无并发/鉴权/网络新增）
- 基线 commit：`e43f8c3` → 红相锚定 `4e770e8`
- 契约矩阵：`.dev-team/contract-matrix-wave52.md`（C1~C7）

## 1. 红相（改动前，机器收据）
| 门禁 | 命令 | 结果 |
| :--- | :--- | :--- |
| 组件契约（jsdom，冻结） | `npx vitest run tests/ConfigGeneralSectionUpdateLayout.spec.ts` | **4 failed / 4**（exit 1）：C1 缺 `config-update-section-title`、C2 缺 `config-update-autocheck-row`、C3 缺 `config-update-version-row`、C3b 同因 |
| 浏览器几何（真实 Chromium，冻结） | `npx playwright test tests/e2e/wave52-update-card-layout.spec.ts --retries=0 --grep "C1\|C2\|C3"` | **3 failed**（exit 1）：`element(s) not found`（C1/C2/C3 锚点均不存在） |

存量回归基线：`.dev-team/regression-baseline-wave52.json`（107 文件 / 1056 passed / 12 skipped / exit 0）。
原始日志：`.dev-team/wave52-red-receipts.md`。
视觉基线（改动前真实截图，随红相提交）：`tests/visual-baseline/wave-52-config-page-before.png`、`tests/visual-baseline/wave-52-config-update-section-before.png`。

## 2. 绿相（改动后，机器收据）
| 门禁 | 命令 | 结果 |
| :--- | :--- | :--- |
| 构建 + 类型 | `npm run build`（含 `vue-tsc --noEmit`） | exit 0，`✓ built in 3.37s` |
| Executor 单测 | `npx vitest run tests/ConfigGeneralSectionUpdateLayout.spec.ts tests/ConfigGeneralSectionUpdate.spec.ts` | **15 passed / 15**，exit 0 |
| 全量单测 | `npx vitest run` | **108 文件 / 1060 passed / 12 skipped**，exit 0（基线 1056 + 新增 4） |
| 真实浏览器几何 | `npx playwright test tests/e2e/wave52-update-card-layout.spec.ts --retries=0` | **5 passed / 5**（26.1s）：C1 文本、C2 `\|switch.right−row.right\|=0px`、C3 同排且居 chip 右、C5 `aria-checked` 翻转 + 偏好落盘、C6 Console Error = 0 / 未捕获异常 = 0 |
| 可观测性变更级门禁 | `audit-diff.sh --base e43f8c3` | **[OK] 无越界实现、收口点完整**，exit 0（7 变更文件 / 505 行新增） |
| 可观测性声明级门禁 | `check-declaration.sh /home/dm/pony-agent` | **结论 PASS**，exit 0（metrics 结构性 N-A 理由成立；tracing/logs/issues 一致） |

证据截图（证据类，入仓）：`.dev-team/evidence/wave-52/config-update-section-after.png`（14707B，真实 Chromium 元素截图）。

## 3. 独立审查（Subagent 计算池，不占名册）
- 视角 A（契约/几何/回退）：`.dev-team/review-wave52-a.json` — `verdict: PASS`，`findings: []`，`dev-review-lint.py --scopes …` exit 0。
  反例攻击：`<label for>` 指向 role=switch 在鼠标与键盘下均可切换并持久化；六档宽度（1280/900/780/620/420/360）几何探针 `|row.right−switch.right| = 0px`、Switch 后尾随元素 0 个；`ariaSnapshot` 可及名保留「自动检查更新」。
- 视角 B（可访问性/视觉风险/越界）：`.dev-team/review-wave52-b.json` — `verdict: PASS`，1 条 **ADVISORY**，`dev-review-lint.py --scopes …` exit 0。
  - 可访问性实测：`label for=` → reka-ui 渲染的 `BUTTON[role=switch]`（button 属 labelable），点文案 false→true 且写 localStorage、无双触发；reka 自动生成 `aria-label="自动检查更新"`；Tab 焦点序 检查更新→自动检查→轨迹上报，Space/Enter 均可切换，focus-visible 环存在。未覆盖 WebKit/WKWebView（本机无 webkit/firefox 二进制）。
  - 视觉实测：1280 / 780 视口下与变更前**同高**（section 145px、card 119px，零像素位移）；chip 与「上次检查」文字底差 0.25px。
  - ADVISORY（`ConfigGeneralSection.vue:258`）：版本行左簇缺 `whitespace-nowrap`/`truncate`，视口 360/400 时「当前版本」逐字换行、日期折 3/2 行。判 ADVISORY 的依据（非免死金牌）：`src-tauri/tauri.conf.json` `app.windows[0].minWidth = 780` ⇒ 产品内卡片宽度 ≥675px，该路径不可达；780 下几何与变更前完全一致。
    **Lead 裁决：接受为已知局限，不在本波修**——加 `whitespace-nowrap` 属超出用户需求的新样式变更，且可能把「折行」换成「溢出裁剪」，须独立评估；已登记为后续候选（见 §5）。

## 5. 后续候选（不阻塞本波交付）
- `ConfigGeneralSection.vue` 版本行左簇窄视口可读性（`whitespace-nowrap` / `truncate` 取舍）——仅当未来下调 `minWidth` 或引入窄屏布局时才成为真实缺陷。

## 4. 偏差与裁决
- 视觉像素 diff 不适用 → 已被真实浏览器几何断言替代，裁决与理由见 `.dev-team/wave52-visual-gate-deviation.md`（Lead 一票，偏差仅限本条）。
