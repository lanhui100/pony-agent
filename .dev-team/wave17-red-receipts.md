# Wave-17 红相验收收据（L2-T 独立测试者）

- **执行者**：test-agent（L2-T）
- **共享任务**：`task-2`
- **契约（只读）**：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2
- **基线 commit**：`5d7d6912f3cb5f608e3888a8ece482bd4526b190`（工作区在该 commit 上叠加本波未提交产物）
- **执行环境**：linux / node `v22.21.0` / vitest `4.1.9` / Playwright `1.60.0`（chromium，viewport 1440×900）
- **测试分级判定**：**L2**（跨模块契约 + 组件级交互状态语义；无网络/线程/锁面变更）。C3 追加真实浏览器物理门禁。

---

## 1. 交付物清单（冻结锚点）

| 文件 | 内容 | SHA-256 |
| :--- | :--- | :--- |
| `tests/acceptance/wave17/ime-composition-guard.spec.ts` | C1-1 ~ C1-8 + C1-5 反向断言 + 事件透传探针 | `c9e94f124cf6e3ae60f7e43a0d366d67c5310beb5679b735f48ad94bd0a197fa` |
| `tests/acceptance/wave17/tabular-nums-contract.spec.ts` | C2-1 / C2-3 / C2-4 class 契约层 | `f2ca6327bfc0f732d60e053e59772aebdea0b3b2c3fca4aeaa29ecba0f83bc06` |
| `tests/e2e/wave17-browser-gate.spec.ts` | C3-1 / C3-2 / C3-3（REV-3 修订）+ C2-2a/b/c/d 真实浏览器物理测量 | `f0e130b2ea2ad817566640b547e53d8b5709b2bdc5a3ea08689501bf95333660`（REV-3 后；REV-1 后为 `79ff24706e12f3850228b4f5a5adc15e580db7d476502f35e924ed2b96965cf5`） |
| `.dev-team/evidence/wave-17/wave17-browser-gate-home.png` | C3-3 关键旅程截图（REV-3 后重拍：1280×720，85,132 字节，工作区可见） | `64d9de9638de88d7cb34476eb3695a9eedf77716dcd23d82827ab7fbf2c73786` |

---

## 2. 红相物理收据（命令 + 真实 EXIT）

### 2.1 契约指定命令（全量目录）

```
$ npx vitest run tests/acceptance/wave17
EXIT=1
Test Files  3 failed (3)
     Tests  27 failed | 4 passed (31)
  Duration  4.24s (transform 3.63s, setup 0ms, import 7.88s, tests 1.76s, environment 1.50s)
```

> ⚠️ 该命令为**前缀匹配**，除本任务两个 spec 外还会拾取并行会话的
> `tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts`（L2-AT 所有，非本任务写域）。
> 该文件有 1 处 `ReferenceError: EXPECTED_NON_COMPOSING is not defined`（第 602 行，测试自身缺陷），
> 本任务**未修改**该文件，已在 §5 登记并交由 Lead 转交其 owner。

### 2.2 本任务两个 spec 的隔离结果（红相锚定口径）

```
$ npx vitest run tests/acceptance/wave17/ime-composition-guard.spec.ts tests/acceptance/wave17/tabular-nums-contract.spec.ts
EXIT=1
 ❯ tests/acceptance/wave17/tabular-nums-contract.spec.ts (6 tests | 6 failed) 355ms
 ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts (11 tests | 9 failed) 371ms
 Test Files  2 failed (2)
      Tests  15 failed | 2 passed (17)
```

**通过的两条（设计使然，非假绿）**：

| 用例 | 为何必须绿 | 作用 |
| :--- | :--- | :--- |
| `PROBE: isComposing 与 keyCode 必须真实抵达 DOM 监听器` | 测试工具链自检 | 若该条红，说明后续"组合态不提交"断言全部是假测试 |
| `C1-5（反向断言·防吞）: isComposing === false 的 Enter 必须照常提交` | 契约 C1-5 明确要求守卫**不得过度拦截** | 与 C1-1/C1-2 构成双向约束 |

### 2.3 失败归属：**全部为断言失败（SUT 栈），零 import / setup 失败**

```
AssertionError 计数（全量 3 文件）：26
Failed to load / Cannot find module / SyntaxError（本任务两文件）：0
```

本任务 15 条失败逐条为断言失败，样例（原样 stdout）：

```
AssertionError: 组合态（isComposing=true）回车被误判为提交：askStore.answer 不应被调用:
  expected "wrappedAction" to not be called at all, but actually been called 1 times
  ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts:238:11

AssertionError: 组合态回车穿透到选项按钮：handleOptionEnter → submitAskAnswer 不应被触发:
  expected "wrappedAction" to not be called at all, but actually been called 1 times
  ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts:283:11

AssertionError: 组合态回车穿透会话重命名：不应 emit submit-rename: expected [ [] ] to be undefined
  ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts:432:7

AssertionError: 组合态回车穿透工作区重命名：store.renameWorkspace 不应被调用:
  expected "wrappedAction" to not be called at all, but actually been called 1 times
  ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts:661:11

AssertionError: 组合态回车穿透到「显示思考」：toggleReasoningVisibility 不应被触发: expected [ [ true ] ] to be undefined
  ❯ tests/acceptance/wave17/ime-composition-guard.spec.ts:521:7

AssertionError: 轮次计数位 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：
  <span class="inline-flex items-center gap-1">
  ↑ <div class="flex items-center gap-x-3 text-[11px] leading-5 text-stone-600">
  ↑ <div class="flex w-full items-center justify-between gap-2 text-left">: expected false to be true
  ❯ tests/acceptance/wave17/tabular-nums-contract.spec.ts:128:9

AssertionError: 指标行#0 承载位#0（文本「1,234」） 必须由自身或 ≤2 层祖先承载 tabular-nums；实际链路：
  <span class="inline-flex items-center gap-0.5 whitespace-nowrap">
  ↑ <div class="pl-[1.125rem] flex flex-nowrap items-center gap-x-3 text-[10px] leading-[1.15] text-stone-400">
  ↑ <div class="min-w-0 space-y-0.5">: expected false to be true
  ❯ tests/acceptance/wave17/tabular-nums-contract.spec.ts:253:11

AssertionError: 格式倒计时位（AskPanel）… 实际链路：
  <span class="inline-flex shrink-0 items-center gap-1">
  ↑ <div class="flex items-center gap-2 text-[10px] leading-4 text-stone-500">
  ↑ <div class="flex items-center justify-between gap-2">: expected false to be true
  ❯ tests/acceptance/wave17/tabular-nums-contract.spec.ts:369:7
```

**结论**：红相为"断言失败"型真红，失败根因全部落在被测组件（SUT）未实现守卫 / 未加 `tabular-nums`，不存在测试自身缺陷导致的红。

---

## 3. 事件透传探针结论（防假测试前置）

探针用例 `PROBE: isComposing 与 keyCode 必须真实抵达 DOM 监听器（原生构造器 + VTU trigger 双路径）` → **通过**。

- 原生路径：`new KeyboardEvent("keydown", { key:"Enter", isComposing:true, bubbles:true, cancelable:true })` → 监听器读到 `isComposing === true` ✅
- 原生路径：`new KeyboardEvent("keydown", { key:"Enter", keyCode:229, bubbles:true, cancelable:true })` → 监听器读到 `keyCode === 229` ✅
- VTU 路径：`wrapper.trigger("keydown", { key:"Enter", isComposing:true })` → `isComposing === true` ✅
- VTU 路径：`wrapper.trigger("keydown", { key:"Enter", keyCode:229 })` → `keyCode === 229` ✅

**根因核实**（避免"断言恒真"）：VTU `createDOMEvent` 会把 options 透传给 `new KeyboardEvent(type, init)`，
因此 `isComposing` / `keyCode` 走构造器 init dict 生效；而**直接赋值** `event.isComposing = true`
在 jsdom 上会被原型 getter 静默吞掉（实测赋值后仍为 `false`）。
→ 本 spec 全部组合态事件统一走 `new KeyboardEvent(...)` + `dispatchEvent`，不使用属性赋值。

---

## 4. C3 浏览器真实链路门禁收据（Playwright / chromium）

### 4.1 REV-1 后的口径（红相锚定时的历史记录；C3-3 已于 REV-3 修订，见 §8）

```
$ npx playwright test tests/e2e/wave17-browser-gate.spec.ts --reporter=list
EXIT=1
  ✓ C3-1: 真实 Chromium 加载构建产物，核心旅程可达 (1.1s)
  ✓ C3-2: 首页加载 + 一次真实提交旅程全程 Console Error = 0 且未捕获异常 = 0 (4.0s)
  ✓ C3-3: 关键旅程截图落盘 .dev-team/evidence/wave-17/ (901ms)
  ✘ C2-2: HomeStatusPanel 指标位 tabular-nums 计算样式 + 等位数宽度稳定 + 相邻位左边缘不位移 (762ms)
  1 failed / 3 passed (11.0s)
```

失败断言（原样 stdout，红相驱动项 = C2-2a）：

```
Error: 轮次计数位: getComputedStyle(...).fontVariantNumeric 必须为 "tabular-nums"，实际 "normal"
  ❯ tests/e2e/wave17-browser-gate.spec.ts:328:7
```

- **C3-1 / C3-2 / C3-3 基线即绿**（`app-layout-shell` + `home-layout-shell` + `workspace-composer-input` 可见；
  完整"输入 → 提交 → 用户消息落入 `workspace-content-column`"旅程全程 Console Error = 0、pageerror = 0）。
- 截图物理凭据：`.dev-team/evidence/wave-17/wave17-browser-gate-home.png`（141,523 字节，非空断言通过）。
- 该 spec 可独立运行，**不依赖** `tests/e2e/browser-preview.spec.ts`（BASE-E2E-1 存量 8 failed 与本 spec 无关）。

### 4.2 C2-2 四条子断言的逐条红相实测（REV-1 口径）

`C2-2a` 是断言链首项，故测试输出只暴露 a；以下 b/c/d 的基线真红状态由**独立物理探针**测得（同一
dist 构建产物、同一 Chromium、viewport 1440×900，`getBoundingClientRect()` 实测）：

| 子断言 | 基线实测 | 判定 |
| :--- | :--- | :--- |
| **C2-2a** 计算样式 `font-variant-numeric` | 7/7 位点均为 `"normal"` | **红**（测试输出已暴露） |
| **C2-2b** 等位数异字形宽度漂移 | `1`=20px vs `9`/`0`=23px（Δ=3px）；`1111`=40px vs `9999`/`0000`=44px（Δ=4px） | **红** |
| **C2-2c** 相邻位左边缘位移 | 最大 **17px**（`模型调用计数位` / `工具调用计数位` 被推移 17px；token 行 6.375–10.625px） | **红** |
| **C2-2d** 单调不减 | `9`=23 ≤ `12`=26 ≤ `1234`=40 | 绿（合理性哨兵，设计使然） |

**C2-2c 逐位点基线位移明细**（单位 px，`9` → `12` → `1234`）：

| 位点 | 自身左边缘位移 | 相邻位最大左边缘位移 |
| :--- | ---: | ---: |
| 轮次计数位 | 17 | 0 |
| 模型调用计数位 | 17 | **17** |
| 工具调用计数位 | 17 | **17** |
| 输入 token 位 | 2.125 | **10.625** |
| 输出 token 位 | 6.375 | **6.375** |
| 缓存 token 位 | 10.625 | **6.375** |
| 生成速度位 | 14.875 | **10.625** |

### 4.3 ⚠️ 交 task-4 的工程结论：`min-w` 必须按"最宽预期数值"定尺

Lead 裁定 C2-2c 由容器 `min-w` 保证。实测证明**任意 `min-w` 不足以保证绿相**，必须按最宽预期数值定尺：

| 场景 | C2-2a | 相邻位最大位移 | C2-2c |
| :--- | :--- | ---: | :--- |
| 基线（无修复） | `normal` | 17px | 红 |
| 仅加 `tabular-nums` | `tabular-nums` | **21px** | **仍红** |
| `tabular-nums` + `min-width: 3rem` | `tabular-nums` | 3.125px（生成速度位超宽） | **仍红** |
| `tabular-nums` + `min-width: 4rem` | `tabular-nums` | **0px** | **绿（可绿性已验证）** |
| `tabular-nums` + `min-width: 5rem` | `tabular-nums` | **0px** | **绿** |

原因：位点内容 = 图标 + gap + 数字，`1234`（4 位 tabular）实测需 ~53px；`min-width: 4ch`（≈24.5px）
与 `3rem`（48px）均不足以吸收 4 位数值。**建议 `min-w-16`（4rem）或按位点实测最大宽度定尺**；
另注意 `min-w` 会使位点常态变宽，属可见布局变化，需在绿相复核时一并目视确认。

---

## 5. ✅ 契约缺陷升级已闭环：C2-2 口径修订（契约 §6 REV-1）

**本 Agent 升级**：原契约 §2 C2-2 字面口径「`9` → `12` → `1234` 自身渲染宽度变化 ≤ 0.5px」物理不可满足。

**Lead 裁决（2026-10-16，红相锚定前，契约 §6 REV-1）**：**批准升级**，认定原措辞把两个不同物理现象
混为一谈 —— ①字形宽度抖动（同位数不同数字 advance width 不等，`tabular-nums` 可完全消除）；
②位数重排（1 位→4 位必然变宽，字体层面不可消除，只能由容器 `min-w` 吸收）。
原 C2-2 拆为 **C2-2a/b/c/d**，其中 **C2-2c（相邻指标位左边缘位移 ≤ 0.5px）才是用户可见缺陷本体**。
Lead 同时明确：**C2-5 从未禁止 `min-width`**，本 Agent 原先"与 C2-5 矛盾"的判断不成立，已撤回。

**决定性的原始实测证据**（dist 构建产物，Chromium 1440×900）：

| 场景 | `9` | `12` | `1234` | Δ |
| :--- | ---: | ---: | ---: | ---: |
| 基线（无 tabular-nums） | 23px | 26px | 40px | **17px** |
| 注入 `font-variant-numeric: tabular-nums` | 23px | 30px | 44px | **21px（仍 ≠ 0）** |

即：`tabular-nums` 只能统一**等位数**数字的 advance width，无法让 1 位与 4 位等宽。

**本测试最终落地口径**（`tests/e2e/wave17-browser-gate.spec.ts` 顶部注释已对齐 §6 REV-1 命名）：

| 契约条目 | 对应断言 | 判定内容 |
| :--- | :--- | :--- |
| C2-2a | Assert 3 | `getComputedStyle(el).fontVariantNumeric === "tabular-nums"`（红相驱动） |
| C2-2b | Assert 4 | 等位数异字形 `1/9/0` 与 `1111/9999/0000` 宽度漂移 ≤ 0.5px |
| C2-2c | Assert 5 | 位数变化时**同行相邻位点包围盒左边缘位移** ≤ 0.5px |
| C2-2d | Assert 6 | 自身宽度 `9` ≤ `12` ≤ `1234` 单调不减（不断言相等） |

**解冻范围**：本次仅修改 `tests/e2e/wave17-browser-gate.spec.ts`（Lead 明确授权的最小增量）。
`ime-composition-guard.spec.ts` / `tabular-nums-contract.spec.ts` 的 sha256 与红相数字**完全未变**，
仍为上表锚定值。

**Lead 裁决原文摘要**：「`tabular-nums` 只能统一等位数数字的 advance width，无法让 1 位与 4 位等宽 ——
原 C2-2 措辞把『字形抖动』与『位数重排』混为一谈，是我的契约缺陷。」→ 已按 §6 REV-1 落地，本条闭环。

---

## 6. 写域合规声明

- 本任务**仅写入**：`tests/acceptance/wave17/**`、`tests/e2e/wave17-browser-gate.spec.ts`、
  `.dev-team/wave17-red-receipts.md`、`.dev-team/evidence/wave-17/wave17-browser-gate-home.png`（契约 C3-3 要求）。
- **未触碰** `src/**`、`.dev-team/contracts/**`、其他既有测试文件、`tests/acceptance/wave17-adversarial/**`。
- **零 git 写操作**（未执行 `git add` / `git commit` / 任何写命令）。
- 测试数据生命周期：所有 mount 均在用例内 `unmount()`；侧栏用例 `afterEach` 清空 `document.body` 与全局桩；
  `vi.restoreAllMocks()` / `vi.unstubAllGlobals()` 已就位；无临时文件残留（探针 scratch spec 已删除）。
- 无 `skip`、无 `sleep` 硬编码等待（Playwright 全部走 `expect(...).toBeVisible()/toContainText()` 显式条件轮询）。

---

## 7. 冻结声明

上述 3 个测试文件 + 本收据经 Lead 红相锚定后即**冻结只读**。

**冻结前已授权的解冻事件（共 2 次，均已用毕）**：
1. **REV-1** 口径修订 —— Lead 批准对 `tests/e2e/wave17-browser-gate.spec.ts` 做最小增量
   （对齐 §6 REV-1 命名 + 补 C2-2c 相邻位位移断言）。
2. **REV-3** C3-3 物理截图凭据修订 —— 见 §8。

其余两个 acceptance spec（`ime-composition-guard.spec.ts` / `tabular-nums-contract.spec.ts`）
全程未解冻、未修改，sha256 保持锚定值。锚定后任何进一步修改均需 Lead 再次明确批准。

---

## 8. REV-3（绿相阶段）：C3-3 物理截图凭据不成立 → 修订并重拍

**提出者**：Lead 在 Join Barrier 目视复核时发现。**归属**：测试自身缺陷（豁免熔断计数）。

### 8.1 缺陷事实

旧版 C3-3 产出的 `.dev-team/evidence/wave-17/wave17-browser-gate-home.png`（1280×720，141,523 字节）
画面是**启动闪屏**（PONY AGENT + 加载进度条），完全没有工作区外壳 / 侧栏 / 输入框。
该文件**不构成**契约 §2 C3-3 要求的「关键旅程真实截图」。

**根因**：
- `index.html:17` 静态注入 `#app-loading-screen`（`position:fixed; inset:0; z-index:999999` 全屏遮罩）。
- `src/main.ts:91-96`：挂载后 `setTimeout(3000)` 加 `fade-out`，再 350ms 后 `splash.remove()`。
- Playwright `toBeVisible()` 对**被覆盖但未隐藏**的元素仍判"可见" → C3-1/C3-2 断言成立，
  但 `openHome()` 返回时闪屏仍在最上层 → 截图拍到闪屏。
- 旧断言只校验「文件存在且 >10KB」，**无法证伪「拍到的是闪屏」** → 该用例是弱证据。

### 8.2 修订内容（仅 C3-3 用例；C3-1/C3-2、C1、C2 断言未动）

1. **闪屏前置存在性确认**（防空等待）：导航后紧贴 `domcontentloaded` 断言
   `expect(splash).toHaveCount(1)`。若闪屏从不出现，下方 `toHaveCount(0)` 会恒真 → 假测试；
   此断言使该退化路径不可能静默通过。
2. **显式等待闪屏移除**（无硬编码 sleep）：截图前 `await expect(splash).toHaveCount(0)`。
3. **证伪组合**：截图前断言 composer 同时满足 可见 + `toBeEnabled()` + `toBeInViewport()`
   + 持有本次填入文本；`app-layout-shell` `toBeInViewport()`；右侧栏 `data-open="true"`。
4. **遮挡命中测试（核心证伪器）**：`document.elementFromPoint()` 取 composer 与
   `app-layout-shell` **中心点最上层可命中元素**，断言命中元素属于目标自身或其后代。
   闪屏不是 `#app` 的后代且为全屏遮罩 → 若它覆盖画面，命中结果必然是闪屏，断言失败。
5. 截图后复核 `splash` 仍 `toHaveCount(0)`。

### 8.3 证伪器有效性物理实测（决定性）

独立 Playwright 探针，同一构建产物、viewport 1280×720：

```
WHILE_SPLASH_PRESENT = {"splashCount":1,"composer":{"ok":false,"hit":"div#app-loading-screen"},"shell":{"ok":false,"hit":"h1"}}
AFTER_SPLASH_REMOVED = {"splashCount":0,"composer":{"ok":true,"hit":"textarea"},"shell":{"ok":true,"hit":"section"}}
FALSIFIER_HAS_TEETH = true
GREEN_CONDITION_OK  = true
```

- 闪屏存在时：composer 中心点最上层命中 `div#app-loading-screen`、shell 中心点命中闪屏内 `h1`
  → `ok=false`。**证明修订后的 C3-3 在旧缺陷现场会真红**，该断言具备真实证伪能力（非恒真）。
- 闪屏移除后：命中 `textarea` / `section`（工作区自身）→ `ok=true`，绿相成立。

### 8.4 修订后物理收据

```
$ npm run build            → EXIT=0（dist/index.html md5 581d0ac4b884941f8c836cea744bfbbd，构建确定性一致）
$ npx playwright test tests/e2e/wave17-browser-gate.spec.ts --reporter=list
EXIT=0
  ✓ C3-1: 真实 Chromium 加载构建产物，核心旅程可达 (836ms)
  ✓ C3-2: 首页加载 + 一次真实提交旅程全程 Console Error = 0 且未捕获异常 = 0 (3.9s)
  ✓ C3-3: 关键旅程截图落盘 .dev-team/evidence/wave-17/（须拍到工作区，非启动闪屏） (4.1s)
  ✓ C2-2: HomeStatusPanel 指标位 tabular-nums 计算样式 + 等位数宽度稳定 + 相邻位左边缘不位移 (766ms)
  4 passed (11.7s)
```

- 修订后 spec sha256：`f0e130b2ea2ad817566640b547e53d8b5709b2bdc5a3ea08689501bf95333660`
- 新截图 sha256：`64d9de9638de88d7cb34476eb3695a9eedf77716dcd23d82827ab7fbf2c73786`
  （1280×720，85,132 字节；连跑两次字节级一致 → 截图确定性）

### 8.5 目视结论（本次修订执行者自检）

新截图画面内容逐区确认：

| 区域 | 目视所见 |
| :--- | :--- |
| **左侧栏** | `Pony Agent` 品牌行 + 折叠按钮；`+ 新对话` 入口；`工作区` 分组（含 `···` 菜单）；会话行 `新对话 / 未保存`（选中态高亮）；底部 `设置` 入口 |
| **中间消息区** | 空态标题「我能帮你做些什么？」 |
| **底部 composer** | 输入框内文本 **`wave-17 evidence screenshot`**；工具行 `工作区修改`、`GPT 5.4 默认`、发送按钮 |
| **右侧状态栏** | `状态` 面板：`0 / 0 / 0 / 0` 四个计数 + token 指标行 `↑0 ↓0 ⚡0 ◷—`（与 C2 修订后的位点结构一致） |
| **启动闪屏** | **完全不可见**（全屏遮罩已从 DOM 移除） |


