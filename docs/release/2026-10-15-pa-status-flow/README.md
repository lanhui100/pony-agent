# PA-STATUS-FLOW 交付与 UI-NFR 达标报告（2026-10-15）

> 需求：桌面端「消息流下方的状态信息」三项优化 —— ① 状态图标去 loading 化、改为有趣动效小图标；② 新消息出现时状态消息不再布局突变；③ 最终 agent 消息 streaming 期间不显示状态消息、上推平滑无反复跳变。
> 契约与门禁真源：[`.dev-team/contracts/wave-11-status-flow.md`](../../../.dev-team/contracts/wave-11-status-flow.md)；UI-NFR 基准：[`.dev-team/nfr-baseline-ui-wave11.json`](../../../.dev-team/nfr-baseline-ui-wave11.json)。

## 1. 需求 → 断言 → 物理收据映射

| 需求 | 断言 | 物理收据（原生 EXIT / 实测物理量） |
| :-- | :-- | :-- |
| R1 图标非 loading + 有趣动效 | M5（e2e 真实浏览器）+ 组件契约 | `status-flow-stability.spec.ts` EXIT=0；图标类 `runtime-status-orbit` + ring/dot 子节点；`animationName !== none`；`prefers-reduced-motion` 全 `none`；无 `snake-spinner`/`animate-spin`；ring 对比度 **4.11:1**（背板实测像素 `rgb(242,237,230)`，修复前 1.43:1） |
| R2 新消息出现时状态消息零布局突变 | M1 / M4（逐帧 `getBoundingClientRect().top`） | 状态条视口 Y 极差 **0px**（≤1px 阈值）；基线实测 **44 次方向反转 / ±23.8px 反复抖动** → 0 |
| R3 streaming 期间隐藏 + 上推平滑 | M2 / M3a / M3b / M3c | `data-visible="false"` + 计算 `opacity==="0"`（过渡 ≤400ms）；M3a 反转 **0**（≤1，容差 8px）；M3b 超调帧 **0**；M3c 峰值滞后 **47–55px**（≤96px，4 行包络） |

## 2. 门禁收据（HEAD = `46a0e7d` 冻结产物，`.dev-team/evidence/final-gates-wave11.log`）

| 门禁 | 命令 | 原生 EXIT | 结果 |
| :-- | :-- | --: | :-- |
| 版本同步 | `npm run version:check` | 0 | — |
| 类型检查 | `npm run typecheck` | 0 | — |
| 单元/组件测试 | `npm run test:unit` | 0 | 76 files / **813 passed** / 10 skipped（基线 75/804） |
| 构建 | `npm run build` | 0 | built in 2.49s |
| 冻结 e2e 验收 | `npx playwright test tests/e2e/status-flow-stability.spec.ts --retries=0` | 0 | **4 passed**（M1/M2/M3a-c/M4/M5 + Console Error=0 + 截图） |
| 回归对照 | `checkpoint-branch.spec.ts` | 1 | 6 passed / 2 failed = **与基线完全一致**（pre-existing） |
| 回归对照 | `browser-preview.spec.ts` | 1 | 6 failed = **与基线完全一致**（pre-existing，侧栏默认收起） |

## 3. UI-NFR 逐项达标（基准 `.dev-team/nfr-baseline-ui-wave11.json` rev wave-11.3）

| 基准字段 | 阈值 | 实测 | 判定 |
| :-- | :-- | :-- | :-- |
| `status_row_viewport_y_range_px` | ≤1 | 0px（流式 / 新消息插入两场景） | PASS |
| `status_slot_height_range_px` | ≤1 | 0px（5 种状态 + 13 档宽度恒 26px） | PASS |
| `streaming_content_motion_reversals_max` | ≤1（容差 8px） | 0 | PASS |
| `streaming_follow_overshoot_budget_frames` / `_slack_px` | 4 帧 + 8px | 0 超调帧 | PASS |
| `streaming_follow_lag_max_px` | ≤96 | 47–55px（16 次分布 39–67px） | PASS |
| `console_errors_max` | 0 | 0（含更新检查域确定性桩） | PASS |
| `reduced_motion_animation_name` | none | 全部 none（含 `::after` 伪元素） | PASS |
| `icon_animation_duration_ms_range` | 1200–2400 | ring 1250–1600 / dot 1200–2400 | PASS |
| `icon_ring_contrast_min_ratio` | ≥3:1 | 4.11:1 | PASS |
| `narrow_viewport_no_overlap` | 320×860 不重叠 | 展开态按钮与文本盒无重叠（截图） | PASS |
| `js_driven_frame_animation_allowed` | false | 纯 CSS keyframes，无 JS 定时器/rAF | PASS |

**NFR-DEGRADED 欠账：0**（`grep DEGRADED .dev-team/evidence` 无命中）。

## 4. 视觉凭据

- `status-flow-orbit-icon.png` —— 新「轨道 + 卫星点」图标（工具执行态）
- `status-flow-streaming-hidden.png` —— 最终回答 streaming 期间状态条隐藏
- `status-flow-narrow-no-overlap.png` —— 320px 越界防御检查：状态条不遮挡「滚动到最新」按钮

## 5. 已知边界与遗留

- 桌面窗口 `minWidth=780`（`src-tauri/tauri.conf.json:19`），320px 不可达；T-c 属防御性越界检查，非验收口径。780×560 实测 label 宽 390px、无截断。
- 契约演进过程中曾尝试把流式跟随由 `smooth` 改为 `auto`（wave-11.1），经同探针 A/B 实测无因果收益（滞后峰值 48 vs 47px、超调 1 帧 20px vs 0 帧）后按「严禁夹带重构」**回退**，patch 存档 `.dev-team/evidence/reverted-wave11.1-auto-follow.patch`。
- 范围外存量缺陷（非本波引入）：`AgentRuntimeStatusBar.vue` 的 `:global(.dark)` 经 scoped 编译后退化为全局 `.dark` 选择器（基线 commit 已存在），建议另开单。
