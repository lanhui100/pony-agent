# Wave-61 汇聚审查裁决（Lead）

## Reviewer-A（`review-join-wave61-a.json`）：verdict=PASS，7 ADVISORY，0 BLOCKING

逐条处置：

| ADVISORY | 处置 |
| :--- | :--- |
| `browser-smoke.mjs` 迁移后第 3 合取项在空数据环境恒真（死条件） | **接受并登记为遗留**：该环境无数据 ⇒ 分支不可达；不删断言（删了会失去「若出现则必须可读进度」的保护），仅在交付说明 §已知遗留 中写明 |
| R8 显式列位无机器断言（删 `col-start-4` 仍全绿） | **已消解**：R8 被 R9（缺席收列）取代，`col-start-*` 全部移除；新行为由交付截图 + `after-geometry.json` 佐证（`toolCallCount=0` 时 3 位点均匀铺满），并在契约 L-3 固化取舍 |
| E-3 未覆盖 `tokens===0`（渲染 0% 空环） | **已修**：`showContextRing` 收紧为「>0 且有限」；`vue-tsc`/wave61/e2e 全绿复跑（commit `427ee0b`） |
| L-2「无 window 退化」分支不可达 | 接受：`showContextRing` 已要求合法 window，退化分支确为防御性死代码（保留，不扩大改动面） |
| 圆环丢失 `tabindex`/`cursor-help` | **已修**：恢复 `tabindex="0"` + `cursor-help`（commit `8f0f485`） |
| HomeSidebar R6 两态验证拆成两例 | 接受：两例分别覆盖「缺席 ⇒ tooltip 缺席」与「在场 ⇒ tooltip 在场」，原意图双向可验 |
| composer 联合 diff 中 8 行为 wave-60 §C7 | 已确认无关（本波用 hunk 级定向暂存，wave-60 改动留在工作树未提交） |

## Reviewer-B（`review-join-wave61-b.json`）：verdict=FAIL，1 BLOCKING + 5 ADVISORY

| 项 | 处置 |
| :--- | :--- |
| **BLOCKING** `grid-cols-4 + col-start-N` 在位点缺席时留 127.5px 空洞（行宽 46%） | **已修（R9）**：可见位点数 N<4 时列模板收为 `repeat(N, minmax(0,1fr))`，`col-start-*` 移除。采纳其「缺席时收列」方案（放弃缺席态跨行对齐，保留无空洞节律）。证据：重采集 `after-valued-status-panel.png`（3 位点均匀铺满整行）+ `after-geometry.json`；`G-4c`（4/4 夹具下等列 + 列中心 ≤0.5px）仍绿 |
| ADVISORY 圆环无百分比文本、轨道对比偏淡、读作 spinner | **部分采纳**：轨道色与尺寸涉及新设计语义（是否常显百分比），登记为后续可选增强（交付说明 §已知遗留 3）；本轮不改，避免超出用户诉求与契约冻结面 |
| ADVISORY 键盘不可达（圆环百分比 / 缓存命中率） | **已修**：圆环位 `tabindex="0"`+`cursor-help`；缓存值 span `tabindex="0"`（commit `8f0f485`） |
| ADVISORY 速度位缺 `whitespace-nowrap` | **已修**（commit `8f0f485`） |
| ADVISORY 弧 `transition-all` 未纳入 reduced-motion | **已修**：`.status-ring-arc` 在双轨降级下 `transition: none !important`（commit `8f0f485`） |
| ADVISORY 计数位图标↔值空白反而松 3.5px、两行首列图标错位 10.5px | 接受：前者由 `gap-1.5`（6px）+ 图标内边距决定，与 token 行已同构；后者是两行字号/图标尺寸差异导致的墨迹错位（非布局错位），登记为可选微调 |

## 结论

- BLOCKING 全部闭环；修后复跑：`vue-tsc` 0 / `wave61` 19 passed / `vite build` 0 / `playwright wave61` 6 passed。
- 未闭环项均为 ADVISORY 且有明确归属（遗留登记或已修）。
