# 契约：PDS-SM 接入生产路径（wave-pds-1）

> 冻结方：Lead。实施者只读。判定等级 **A 级 / L2**：2 个模块、无新增对外接口、无网络 IO、无并发原语、改动行数 < 100。
> 依据：`docs/decisions/0057` 影响节声明的"后续动作"。

## 范围

| 项 | 文件 | 性质 |
|---|---|---|
| S1 | `src/lib/useStreamingPresentationState.ts` | 用 `stepReveal` 替换内联释放逻辑 |
| S2 | `src/lib/useTimelineAutoScroll.ts` | 视口阈值 / 空闲恢复常量改为从 `DEFAULT_SCROLL_INTENT` 取 |
| S3 | `docs/decisions/0058-*.md` | 记录本次接入与**未接入部分** |

## 硬约束

- **C1 零行为变化**：`tests/useStreamingPresentationState.spec.ts`（既有 14 例，只读，**一字不得改**）必须保持全绿。
  该文件即本次的等价性基线；任何断言调整都视为行为变化，须回到本契约重议。
- **C2 既有冻结验收只读**：`tests/acceptance/**`、`tests/e2e/**` 一律不得修改。
- **C3 公开面不变**：`useStreamingPresentationState` 的返回对象字段、`useTimelineAutoScroll` 的入参与返回保持完全一致。
- **C4 单源化**：`useTimelineAutoScroll.ts` 不得再出现 `260` / `3000` 这两个魔法数（必须来自 `DEFAULT_SCROLL_INTENT`）。
- **C5 生产路径可追溯**：`useStreamingPresentationState.ts` 必须 import `stepReveal`，不得复制其算法。
- **C6 降级语义不变**：`prefers-reduced-motion` 下仍一次性释放全部字符。

## 允许的内部变化

### 等价（不改变可观测输出）

1. `readReleaseRate()` 的返回值不再喂给"加法"，而是作为 `RevealBudgetConfig.charsPerTick`；输出字符数不变。
2. `prefersReducedMotion()` 由"速率设为 MAX_SAFE_INTEGER"改为传入 `reduceMotion: true`；输出字符数不变。
3. 停滞计数与 `lastIncomingLength` 的更新时机**与旧实现逐字一致**：只在"尚未追平"时推进并回写；已追平（含内容被截短）时整段跳过。
   第三轮审查曾指出"已追平时回写计数"会在「长期停滞→截断→单 tick 跳回原长」上产生差异，该差异已通过改为"仅 `visible < incomingLength` 时写入"消除——见下条 #5 与验证记录。
4. 非法长度（NaN/负数，本 hook 结构上不可达）不再回退游标——防御性，仅为与 reducer 契约对齐。
5. **截断时游标不下调**：写入条件是 `previousReveal.visible < incomingLength`——这正是旧实现回写游标的时机。已追平或被截短时**整段跳过**，因此 reducer 的夹取结果不会被回写到宿主游标，"截断→再增长"仍瞬间追平收缩前进度（生产可达：`src/stores/runtime.ts` 超时重连把 pending content 置 `""`）。副带收益：已追平时零响应式写入。

### 有意偏离（改变退化输入下的表现）

6. `localStorage` 速率 < 1（如 `"0.5"`）：旧实现 `Math.floor(0.5)=0` ⇒ 游标恒 0、文字停滞约 2s 直到停滞兜底；新实现夹到 `1`。这是缺陷修复，理由见 ADR 0058。

## 验收（本契约自带的门禁）

| ID | 断言 | 方式 |
|---|---|---|
| A1 | 既有 14 例全绿，断言一字未改 | `git diff --stat tests/useStreamingPresentationState.spec.ts` 为空 + vitest |
| A2 | 生产文件 import 并**真正调用** reducer（剥注释后判定）；无内联加速/停滞算式 | 源码守卫 + **守卫自测**（已知绕过形态必须被拦下、合法写法不得误伤）|
| A3 | 两个常量**正向投影**自 `DEFAULT_SCROLL_INTENT` 对应字段；无 260/3000 字面量 | 源码守卫 ×3 |
| A4 | hook 与 `stepReveal` 在默认配置、自定义速率下逐 tick 输出一致 | 对拍（仅证接线，非同源即恒真） |
| A4b | **黄金表**：固定序列的逐 tick 释放长度按重构前算法手算，独立于新实现 | 特征化断言（唯一能抓算法漂移） |
| A4c | 已追平且无变化时，再次 sync 不产生多余响应式失效 | computed 评估计数 |
| A4d | 截断置空后再增长，显示瞬间追平收缩前进度 | 黄金表（游标不下调语义） |
| A5 | 降级路径一次性全量释放 | 既有 reduced-motion 用例 + 对拍 |
| A6 | 速率 `0.5` 时按 1 推进（不卡死） | 行为断言 |
| A7 | 全量单测 + 类型检查 + 构建 Exit 0 | `npm run test:unit`、`npm run build` |

**红相说明**：本次为**等价重构**，hook 的可观测输出不应变化，故"先红后绿"只适用于源码守卫（A2/A3）与两条黄金表（A4b/A4d，已实际验证：去掉修复后分别返回 `[4,8,12,0,4,8]` 而非 `[4,8,12,0,11,16]`）；A1/A4/A4c/A5/A6 走"基线绿 → 重构后仍绿"。这是等价性重构的正确门禁形态，不用假红灯凑数。

**等价性验证记录**：
- 实现方本地复算：HEAD 版旧算法 vs 当前逻辑，2005 条序列（5 定向 + 2000 随机，速率 1/2/3/4/7，含 12% 截断为空、16% 截断到更短）→ 逐 tick **0 差异**。
- 第四轮独立复核：以 `git show HEAD:` 导出的**真旧实现**与当前 hook 并排对拍，300 条随机序列 / 419,526 tick，另 3000 条 / 4,285,253 tick → **diffTicks = 0**；定向族（长期停滞后截断再跳回原长、截断为空再增长、截断到更短再增长、`reduceMotion × 截断`）全部 0 差异。
- 唯一**内部**差异（不可观测）：停滞兜底收敛那一 tick，旧实现写 `stallTicks = 34`，reducer 写 `0`；此后任一次增长都会把计数清零，4.28M tick 内 0 处显现。

**守卫能力的边界（如实登记，第三轮审查实测）**：A2/A3 的源码守卫是**冒烟级，不是证明级**——任何正则都无法穷举"重新实现同一算法"的所有写法。已知仍可绕过（不修，因为无法穷举，且改判据只会把绕过面推到别处）：
  - **改名绕过**：把 `backlog` 改叫 `gap` 后 `Math.floor(gap / 8)` 不被拦；`LIMIT = 0x22`、`delta / DIVISOR` 同漏。
  - **使用点绕过**：`AUTO_SCROLL_THRESHOLD_PX` 声明合规，但在比较处写 `<= 130 * 2`。
  - **整段抄回旧算法**：黄金表 encode 的正是旧算法，抄回旧实现时 A4b/A4d 不会变红 —— 此时 A2 是唯一防线，而它可被改名绕过。
**结论**：守卫负责"早发现"，不负责"证明不存在"。真正的防线是 review + 基线 14 例 + 黄金表 3 张，三者都不足以单独成立，必须同时存在。

## 明确不做（转 wave-pds-2）

`useTimelineAutoScroll` 的**控制流**不改为状态机：现有实现含 `programmaticScrollActive` / `userScrollOverrideVersion` /
`scrollQueued` / 看门狗 / 双 lerp / 同帧补偿，与 `reduceScrollIntent` 的简化模型不同构；强行替换等于重写
wave-11 / wave-14 调好的逻辑，且只能靠真浏览器逐帧 e2e（F3-5）验证。本波只做常量单源化。
