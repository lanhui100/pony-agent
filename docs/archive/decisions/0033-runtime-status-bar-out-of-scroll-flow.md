# 0033 运行状态条移出滚动内容流：composer 上沿常驻覆盖层 + streaming 期隐藏

Status: implemented

## 背景

桌面端「消息流下方的状态信息」（`AgentRuntimeStatusBar`）原先作为滚动内容列 `[data-testid="workspace-content-column"]` 的最后一个块渲染，位于流式增长内容之下。真实浏览器逐帧实测（1280×860，合成 3s 流式）暴露两个用户可见问题：

1. 状态条被逐行增长的内容推挤，跟随滚动又在后续帧把它拉回 → 视口 Y 出现 **44 次方向反转、稳态 ±23.8px 反复抖动**；新消息插入时同样发生位移突变。
2. 最终回答 streaming 期间状态条仍然显示（"正在回复，用时 N 秒"），与「回答已在眼前，无需再报状态」的交互预期冲突。

## 候选方案

- **方案 A：保留在滚动流内，仅加高度/透明度过渡**。改动最小，但内容每增长一行（23.8px）仍会在同一帧把它下推，跟随补偿在后续帧回拉 → 实测的 44 次反转与 ±23.8px 抖动无法消除。落选：不解决 R2/R3。
- **方案 B：移入 composer 面板内部作为常规行**。视觉上贴近输入框，但 composer 是 `absolute bottom-0` 覆盖层，其高度会随状态行出现/消失增长；`useTimelineAutoScroll.getAnchorTargetTop()` 以 `composerShell.getBoundingClientRect().top` 为参考点，参考点上移/下移会让跟随目标跳变约 28px，等于把布局突变从消息流转移到滚动锚点。落选：引入新的突变源。
- **方案 C（采纳）：作为 composer shell 上沿外侧的绝对定位常驻行**。`WorkspaceComposer` 新增 `#status` 插槽，`HomeWorkspace` 通过它渲染 `absolute inset-x-0 bottom-full pointer-events-none` 的包装层（与 composer 同宽居中），状态槽位在 `agentRuntimeStatus != null` 期间常驻不卸载，仅以 `data-visible` 切换 `opacity/visibility`。完全脱离滚动内容列与锚点链路，composer 高度不变。
- **方案 D：修改 `useTimelineAutoScroll` 的流式跟随策略（`smooth` → `auto`）以消除「179px 跟随滞后」**。同探针 A/B 实测两版指标等价甚至略差（距底滞后峰值 48 vs 47px；无超调度量下 auto 版超调 1 帧 20px、原版 0 帧），且 `anchorToComposerDistance ≈ −179px` 经查为**锚点目标结构性不可达**（composer 覆盖视口底部），并非跟随滞后。落选：无因果收益，按「严禁夹带重构」回退（patch 存档 `.dev-team/evidence/reverted-wave11.1-auto-follow.patch`）。

## 决策

运行状态条作为 **composer 上沿外侧的常驻覆盖层**渲染，永久脱离滚动内容列；可见性由 `data-visible` 驱动——仅当 `agentRuntimeStatus.mode === "responding"`（真正在流式回复）且末个可见 turn 的 assistant 为 `pending` 且已有可见内容且无 running/planned 工具活动时隐藏（`opacity: 0` + `visibility: hidden` + `aria-hidden`），其余状态（connecting/thinking/tool_drafting/executing_tool/waiting_approval/retrying/error/会话操作）一律可见。状态条为纯展示元素，恒 `pointer-events: none`。

## 影响

- **布局不变量**：状态条视口 Y 极差 0px、槽位高度恒 26px（13 档宽度 × 5 种状态实测）；新消息插入不移动状态条。契约与断言冻结在 `.dev-team/contracts/wave-11-status-flow.md`（M1–M5）。
- **交互约束**：状态条不得恢复命中（否则会吞掉 composer 上沿 ~24px 条带内消息文本的点击/选中）；为右侧「滚动到最新」按钮预留 `pr-[7.5rem]`（展开态 110px），320px 越界场景由 e2e 防御性用例守护。
- **动效约束**：图标为纯 CSS「轨道 + 卫星点」（`runtime-status-orbit`），所有节点所有 mode 时长必须落在 1200–2400ms；`prefers-reduced-motion: reduce` 下全部 `animation-name: none`；隐藏态 `animation-play-state: paused`。轨道环对底色对比度 ≥3:1（实测 4.11:1）。
- **测试策略**：`streaming_content_single_frame_displacement_max_px=32` 因混淆「内容真实增长」与「滚动补偿质量」被 supersede，替换为 M3a（方向反转 ≤1，容差 8px）+ M3b（无超调，4 帧 + 8px）+ M3c（跟随滞后 ≤96px，4 行包络），基准登记见 `.dev-team/nfr-baseline-ui-wave11.json`。
- 交付与 NFR 达标报告：`docs/release/2026-10-15-pa-status-flow/README.md`。
