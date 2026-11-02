# 0058 流式释放与视口阈值接入 PDS-SM reducer（等价重构 + 常量单源化）

Status: implemented

## 背景

[0057](0057-streaming-motion-framework-agnostic-spec.md) 把流式动效抽象成五层规范，但当时明确"不接入运行时代码"——规范因此处于三条悬空状态：

1. **同一算法有两份实现**：`useStreamingPresentationState.ts` 内联写着"每 tick 4 字符 + 超过 120 字符积压按 1/8 加速 + 34 tick 停滞全放"，与 `logic/budget.ts` 的 `stepReveal` 逐字同构。改一处必然忘另一处。
2. **同一阈值有三处定义**：`useTimelineAutoScroll.ts` 的 `AUTO_SCROLL_THRESHOLD_PX = 260`、`USER_SCROLL_IDLE_MS = 3000`、`DEFAULT_SCROLL_INTENT` 的同名字段、`tokens.css` 的 `--pds-pin-tolerance`。三处任何一处漂移都不会报错，只会让行为悄悄变。
3. **44 例 reducer 单测保护不了任何生产行为**：它们测的是没人调用的纯函数；生产路径只有 hook 层的集成测试（14 例），覆盖不到积压加速、停滞收敛的边界。

## 候选方案

- **方案 A：不接入，规范仅作文档**。落选：双写必然漂移；且等于承认 0057 的 44 例单测是"测试一个孤岛"——这比不写测试更糟，因为它制造了虚假的安全感。
- **方案 B：把 reducer 复制进 `src/lib/streaming-motion/`**。落选：立刻产生两份真值，反向加剧问题 1；且 skill 需要为"复制到新项目"保留可运行文件，复制进 src 后两份会分叉。
- **方案 C：把 `useTimelineAutoScroll` 整体改写为 `intent` 状态机**。落选（本波推迟）：现实现含 `programmaticScrollActive` / `userScrollOverrideVersion` / `scrollQueued` 三套互锁、rAF 看门狗、导航与补偿双 lerp、同帧补偿（≤96px）。`reduceScrollIntent` 是去掉了这些机制的**简化模型**，两者不同构；强行替换等于重写 wave-11 / wave-14 调好的逻辑，而唯一的验证手段是真浏览器逐帧 e2e（F3-5），成本与风险都不属于"接入"这个动作。故本波只做常量单源化，控制流保持原样。
- **方案 D（采纳）：直接 import skill 内的 reducer 作为唯一实现**。`useStreamingPresentationState` 的释放推进整体交给 `stepReveal`；`useTimelineAutoScroll` 只把两个常量投影自 `DEFAULT_SCROLL_INTENT`，控制流一行不动。

## 决策

1. **释放节奏归 reducer**：`useStreamingPresentationState` 不再持有任何释放算法，只保留"按消息存游标 + 读用户可配置速率 + 投影成可见前缀"三项职责。`charsPerTick` 仍可由 `localStorage` 覆盖，其余参数取 reducer 默认值。
2. **降级改走 reducer 参数**：`prefers-reduced-motion` 由原来的"把速率设成 `MAX_SAFE_INTEGER`"改为传入 `reduceMotion: true`。语义更直白，输出字符数不变。
3. **视口常量单源化**：`useTimelineAutoScroll` 的 `AUTO_SCROLL_THRESHOLD_PX` 与 `USER_SCROLL_IDLE_MS` 改为 `DEFAULT_SCROLL_INTENT.pinTolerancePx` / `.idleResumeMs` 的投影，源码中不再出现 `260` / `3000` 字面量。
4. **`src → .agents` 是编译期依赖，不是文档依赖**：`.agents/skills/streaming-motion/logic/*.ts` 从此参与 `vue-tsc` 与 `vite build`，因此**不得**被 `.gitignore`、打包排除或清理脚本删除。
5. **等价重构不进红相**：本波不改变 hook 的可观测输出，故 A1/A4/A4c/A5/A6 走"基线绿 → 重构后仍绿"，只有源码守卫（A2/A3）与两张黄金表（A4b/A4d）先红后绿。红绿形态与逐条验收记入 `.dev-team/contracts/pds-integration.md`。
6. **两处必须显式写的语义**（都因"reducer 是共享规范，宿主有额外职责"）：
   - **写入时机必须与旧实现一致**：只有 `visible < incomingLength`（尚未追平）时才推进并回写。`stepReveal` 会按新长度夹取 `visible`（这对 reducer 是正确的），但宿主若把这个夹取结果回写，就会改变旧行为——旧实现只在**增长分支**回写游标，收缩时仅本地夹取用于显示。生产可达路径是超时重连把 pending content 置 `""`（`src/stores/runtime.ts`）。把写入门槛设在"尚未追平"同时满足两件事：截断语义不变，且已追平时零响应式写入。
   - **速率 < 1 夹到 1**：旧实现 `Math.floor(0.5) = 0` 会让文字停滞约 2s 直到停滞兜底，这是缺陷；新实现夹到 `1`。这是本波唯一**有意**改变可观测输出的地方，已在契约中单列为"有意偏离"。

## 影响

- **用户可见行为**：除下面一条外无变化。既有 14 例基线断言一字未改且全绿；新增 15 例（源码守卫 + 守卫自测 / 对拍 / 三张黄金表（默认速率、自定义速率、截断）/ 响应式失效计数 / 速率夹取）。
  - **唯一有意偏离**：`localStorage` 释放速率 < 1（如 `"0.5"`）时，旧实现算出 0、文字停滞约 2s；新实现夹到 1 逐字推进。属缺陷修复，不做等价保留。
- **保护面变化**：生产释放路径从此由 reducer 的 44 例（含畸形输入对抗）共同覆盖；这是本波的主要收益，而非某个具体缺陷被修复。
- **新增编译期耦合**：`src/` 依赖 `.agents/`，构建产物与类型检查都包含这两个 reducer 文件。任何把它们移出仓库或改名 `SKILL.md` 目录结构的操作都会破坏构建。
- **验收**：`.dev-team/contracts/pds-integration.md`（A1–A7）；`tests/pds-integration-parity.spec.ts`。
- **推迟项（wave-pds-2）**：`useTimelineAutoScroll` 的控制流未状态机化。若后续要做，前提是先有稳定的真浏览器逐帧基线（F3-5），且必须作为独立 ADR 提出——本篇不为其背书。
- **可观测性**：纯前端渲染层改动，四维归属 N-A（无新增接口 / 网络 IO / 后台任务 / LLM 调用）；`audit-diff.sh` 与 `check-declaration.sh` 均 exit 0。
