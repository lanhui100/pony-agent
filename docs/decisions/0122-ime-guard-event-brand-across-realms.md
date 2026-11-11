# 0122 IME 组合态守卫改按事件品牌串判定（跨 realm 事件不再漏拦）

Status: implemented

> 本文修订 [0036](0036-ime-composition-enter-guard-and-numeric-width-stability.md) 决策 1 的**判定口径**
> （`instanceof KeyboardEvent` + `isComposing === true` → 品牌串 + 真值语义）；0036 的其余决定
> （6 处入口接入、鼠标路径不加守卫、`tabular-nums` 与容器位宽）全部仍然现行。

## 背景

`src/lib/ime.ts` 的 `isImeComposingEvent` 是 wave-17 C1 的守卫接缝，原实现以
`event instanceof KeyboardEvent` 作为唯一闸门，并以 `event.isComposing === true` 严格比较。
[0036](0036-ime-composition-enter-guard-and-numeric-width-stability.md) 的候选方案 C 已如实登记过这条差异：
「`ime.ts` 的判定比既有内联更严格（多一道 `instanceof` 闸门 + `=== true`），差异仅在跨 realm 事件对象
与真值非布尔 `isComposing` 两档；审查者实测本仓 `src/` 内 iframe/webview/`createEvent` 零命中，
**当前无可达放行路径**，登记为 PA-136」。该登记随任务系统退役迁入 `backlog:B099`。

复核结论是这条「不可达」的判定**只在单一文档上下文内成立**：

1. **跨 realm 失效已可复现**：`instanceof` 只认同当前 realm 的构造器。jsdom 实测
   （iframe `contentWindow.KeyboardEvent` 构造的事件）—— `Object.prototype.toString` 得
   `[object KeyboardEvent]`、`isComposing === true`，而 `event instanceof KeyboardEvent === false`。
   即同一份事件对象在品牌与字段上都明确是「组合态键盘事件」，守卫却返回 `false`。
2. **失效方向是 fail-open**：返回 `false` 意味着放行回车 ⇒ 中文/日文/韩文用户选词回车被当成显式提交，
   正是 wave-17 C1 要消灭的原始缺陷。守卫在跨文档场景下不是「更严格」，而是**沉默地不设防**。
3. **真值口径与组件内联判定不同集合**：`HomeWorkspace.vue` 的三处内联判定是
   `event.isComposing || event.keyCode === 229`（真值语义），`ime.ts` 是 `=== true`（严格语义）。
   同一语义两套口径，任何非布尔真值（框架注入、测试夹具、旧浏览器）在两条路径上判定相反。

Tauri 桌面壳是 WebView，第三方内容与测试环境的第二 window 都不是结构上不可能；「当前零命中」
描述的是**今天的调用面**，不是这条判定本身的安全性。

## 决策

1. **键盘事件按品牌串识别**：`Object.prototype.toString.call(event) === "[object KeyboardEvent]"`。
   品牌串读的是内部槽（`[[Class]]` / `Symbol.toStringTag`），跨 realm 稳定，不依赖构造器身份。
2. **组合态按真值语义判定**：`Boolean(event.isComposing) || event.keyCode === 229`，与
   `HomeWorkspace.vue` 的存量内联判定逐档对齐（非布尔真值同权生效）。
3. **异常 fail-closed**：品牌串读取与字段读取各自 `try/catch`，任何读取异常返回 `false`，
   保证「守卫不得抛异常」的冻结契约（wave-17 C4-1）不因新判定方式退化。
4. **不扩大写域**：`HomeWorkspace.vue` 的三处内联判定本轮不动（其守卫粒度问题归 `backlog:B098`，
   且那三处形参已是 `KeyboardEvent`，跨 realm 不可达）；统一收口作为可选后续，不在本次变更内。
5. **形参签名不变**：仍是 `isImeComposingEvent(event: Event | null | undefined): boolean`，
   6 处调用方零改动。

## 候选方案与取舍

- **A. 维持 `instanceof`，把跨 realm 继续登记为已知限制**：落选。缺陷方向是 fail-open（漏拦 =
  用户选词回车误提交），成本由用户承担；而修复只需改一处纯函数、零调用方改动。把可复现的
  fail-open 留在守卫里，等于用「今天没人这么用」替代「这条判定是对的」。
- **B. 鸭子类型（`"isComposing" in event` / `event.type` 前缀）**：落选。`in` 对 null 原型对象与
  Proxy 可抛异常（与 C4-1 不抛异常契约冲突）；`MouseEvent` 等非键盘事件也带 `type`，前缀判定
  会把鼠标事件放进组合态分支，制造反向误吞。
- **C. 逐 realm 遍历 `window` / `iframe.contentWindow` 做 `instanceof`**：落选。realm 集合不可穷举
  （动态创建的 iframe、`window.open`、WebView 内部上下文），且遍历本身引入对 DOM 全局的读取，
  与「纯函数、无外部状态读取」契约冲突。
- **D. 品牌串判定（入选）**：跨 realm 稳定、零依赖、零 DOM 全局读取。已知弱点是可被
  `Symbol.toStringTag` 伪造；但伪造品牌只是让「非键盘事件」进入字段读取，缺失字段即返回 `false`
  —— 失效方向是 fail-closed（不误吞），故不构成风险。该弱点由测试正面钉住
  （`tests/ime-guard.spec.ts` 的伪造品牌用例）。
- **E. 保留 `isComposing === true` 严格比较**：落选。与组件内联判定不同集合这件事本身就是缺陷
  （同一语义两套口径），且严格比较在跨 realm 与框架注入场景下会漏拦真值。
- **F. 顺带把 `HomeWorkspace.vue` 三处内联判定统一改为调用 `isImeComposingEvent`**：本轮落选。
  那是行为变更（守卫粒度）与 `backlog:B098` 同一写域，混装会让「修守卫判定口径」与「修守卫过宽
  吞掉 Ctrl/Cmd+Z」两件事互相掩护；最小闭包优先，登记为后续可选统一。

## 影响

- **行为变化（用户可见）**：跨 realm / 非布尔真值场景下，IME 选词回车不再误触提交；单文档上下文内
  判定结果与修订前完全一致（wave-17 冻结用例 116 例仍全绿）。
- **契约面**：`src/lib/ime.ts` 的判定口径变更（模块级纯函数，6 处调用方签名与语义不变）；
  不新增依赖、不新增 IPC、不改 Rust、不 bump 版本。
- **测试面**：新增 `tests/ime-guard.spec.ts`（11 例）——同 realm 真值矩阵、跨 realm 四种组合、
  非布尔真值、与组件内联判定的逐档一致性、伪造品牌 fail-closed。红相证据：修订前 4 例断言失败
  （跨 realm 组合态判为 `false`、非布尔真值判为 `false`），绿相：11 例全通过。
- **可观测性**：四维归属不变（`tracing` / `issues` 仍 applicable，`metrics` / `logs` 维持结构性 N-A）。
  本次只改一个前端纯谓词，无新增网络 IO、后台任务、LLM 调用、外部接口或 UI 交互面，
  不新增 enforcement_point。
- **残留**：`HomeWorkspace.vue` 三处内联判定仍为独立实现（候选方案 F），其守卫粒度缺陷仍归
  `backlog:B098`。
