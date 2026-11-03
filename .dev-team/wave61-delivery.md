# Wave-61 交付说明（status-panel）

- 需求（用户原话）：①「桌面端对话框中的上下文占用圆环，改到右侧状态中的第一行，与轮次等同行」；
  ②「状态栏这块儿的排列间距需要优化：字段图标与值之间没有亲密性，间距设计和对齐都不美观」；
  ③「空值／零值时显示 -/0 都比较丑，应该不显示，在有值时平滑动效入场」。
- 契约：`.dev-team/contracts/wave-61-status-panel.md`（冻结后经历 R1–R8 八条 Lead 裁决）
- 分级：A+（单模块前端 UI；无外部 IO/网络/并发面；触发面 = UI 交互 + 冻结验收测试授权迁移）
- 团队：Lead（契约/裁决/门禁/浏览器验收）· executor（实现）· test-agent（唯一测试写手）· 2× Subagent L3（方案对抗 + 汇聚审查）

## 1. 改了什么

| 文件 | 变更 |
| :--- | :--- |
| `src/components/HomeStatusPanel.vue` | 圆环作为计数行**第 4 位点**（`status-context-usage`，与轮次/模型/工具同行）；两行统一 `grid-cols-4` + `col-start-1..4` 显式列位；位点结构 = 图标 + 值（`gap-1.5` 亲密性，宽度预留上移到容器 `min-w`）；空值/零值整位点不渲染 → 行空整行折叠 → 全空且无拦截横幅整段折叠；入场动效 `status-slot-enter(.--ring)` 200ms `cubic-bezier(0.3,0.7,0.4,1)`，仅 transform/opacity，`prefers-reduced-motion` + `[data-pds-reduce-motion]` 双轨降级 |
| `src/components/chat/WorkspaceComposer.vue` | 移除圆环 DOM 与专属逻辑（`contextUsagePercentage` / `contextUsageTooltip` / `contextDebugString` / `formatCompactTokenInteger` / `debugAttrsEnabled` / `useTraceProjection` 解构与 import），纯删除 94 行 |

用户可见效果：空会话时右栏只剩「状态」标题（不再是一排 `0` 与 `—`）；有数据时四列等宽对齐、图标紧贴数值；
上下文圆环从输入框底部移到状态面板第一行，与轮次计数同行、与下方速度位同列。

## 2. 物理收据（详见 `.dev-team/wave61-receipts.md`）

| 门禁 | 命令 | Exit | 结果 |
| :--- | :--- | :--- | :--- |
| G-1 | `npx vitest run --maxWorkers=2 --exclude …/v10-baseline-freeze.spec.ts` | **0** | 137 files / 1303 passed / 12 skipped |
| G-1′ | V10-2/V10-3 的两条子命令手动等价复跑 | **0** | 37 passed / 137 files passed |
| G-2 | `npx vue-tsc --noEmit` | **0** | 无输出 |
| G-3 | `npm run build` | **0** | built in 5.21s |
| G-4 | `npx playwright test tests/e2e/wave61-status-panel.spec.ts` | **0** | **6 passed**（含 console error=0、截屏 >10KB） |
| G-5 | `audit-diff.sh --base c575243` / `check-declaration.sh .` | **0** / **0** | OK / PASS |

红相→绿相：红相锚定 `b1d62f3`（18 failed/19）→ 实现 `5c0eb40` → 用例缺陷重写 `cf783b0` → 全绿。

## 3. 证据清单（截图 / 几何）

- 改动前：`docs/release/evidence/wave-61-status-panel/before-empty-status-panel.png`（一排 `0` 与 `—`）、`before-empty-composer-footer.png`（圆环在模型选择器左侧）
- 改动后：`after-empty-status-panel.png`（仅标题）、`after-valued-status-panel.png`（4 列对齐 + 圆环第 4 列）、`after-empty-composer-footer.png`（圆环消失）
- 几何：`after-geometry.json` —— 计数行与 token 行 4 列中心 x 完全一致（1164.75 / 1236.25 / 1307.75 / 1379.25），
  圆环位 x 中心 1379.25 = 速度位列中心（物理对齐的直接证据）。

## 4. 契约演进与授权迁移（审计可追溯）

- **R1** 缓存命中率尾段移出行内 → 并入值 tooltip（实测 86px > 63.5px 列宽，必换行）；图标 tooltip 仍精确为 `缓存读取`。
- **R2** `wave17-browser-gate` 夹具补 `call_tool`（使「8 位点」在 E-1 下可达）。
- **R3** `v10-baseline-freeze` 只对 §5 授权文件重算 SHA，`note` 标注 `wave-61 授权重锚`。
- **R4** 入场动效机制钉死为静态 class + `@keyframes`（`<TransitionGroup>` 对 Tooltip 的 Fragment 根不下发 enter 类，已复现）。
- **R5** 无可见行且无拦截横幅 ⇒ 外层 `border-t pt-3 mt-3` 整段折叠（不留空带）；**治理拦截横幅不被连坐**。
- **R6** `HomeSidebar` tooltip 断言按「位点 → tooltip」同生共死迁移，并用补夹具恢复无条件精确包含（D5）。
- **R7** 栅格化会让「相邻位左边缘不漂移」变恒真 ⇒ 补「位点自身宽度 Δ≤0.5px」等价判别断言（判别力补偿）。
- **R8** 显式 `col-start-N` 是「空值不渲染 × 两行对齐」同时成立的必要条件（真浏览器实测：仅自动流式布局时圆环会落到第 3 列）。
- **R9（收口阶段裁决，取代 R8）**：位点缺席时收列 `repeat(N, minmax(0,1fr))`，消除 `toolCallCount=0` 时
  第 3 列空置造成的 127.5px 视觉空洞（Reviewer-B BLOCKING 处置）；代价是两行严格列中心对齐只在可见位点数相同时成立。
- 收口阶段同时修复 4 条 Reviewer ADVISORY：速度位补 `whitespace-nowrap`（≈+≥1000 t/s 不折行）、
  圆环位恢复 `tabindex="0" + cursor-help`（百分比 tooltip 键盘可达）、缓存值 tooltip 补 `tabindex="0"`
  （命中率移入 tooltip 后的键盘可达）、`reduced-motion` 双轨同时关闭弧的 `transition-all`。
- §5/§5.1 共授权迁移 10 处既有冻结断言，逐处文件头写明「原意图 / 迁移理由 / 等价断言」；未授权项零改动。

## 5. 不可逆面登记

- **无**：本波不涉及 DB schema、索引键、序列化字段编号、Loki/Prometheus label 值域、外部系统状态。
- 兼容性：`status-metric-*` testid 与 `data-context-debug` 字面量格式**逐字保留**；圆环 testid 由新增 `status-context-usage` 承载，
  旧 `workspace-context-usage-circle` 随迁移下线（两处引用它的 e2e/smoke 已按 §5 授权迁移到新锚点）。

## 6. 已知遗留（不在本波因果面）

1. `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` 在 vitest 内再 spawn vitest，机器负载下子进程 pool 报 `write EPIPE`；
   其断言目标已用手动等价子命令 Exit 0 证明。属既有运行器嵌套脆弱点，建议后续专项（本波不动写域外文件）。
2. 隐身位点会留下空列（如工具位=0 时第 3 列空缺）——这是「空值不渲染 × 列对齐」的必然取舍，已由契约 L-3/R8 固化；
   若后续希望消失后重新居中，需改契约并接受两行错列。
3. 圆环为 14px 图标 + 无百分比文本，信息靠 tooltip 承载；若希望环旁常显百分比，需新契约（会挤压 63.5px 列宽）。
