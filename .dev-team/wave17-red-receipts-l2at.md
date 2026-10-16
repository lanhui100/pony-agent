# Wave-17 · task-5 · L2-AT 红相物理收据（C4 对抗破坏）

- 执行者：L2-AT 对抗破坏测试者（Adversarial QA，subagent）
- 写域（严格）：`tests/acceptance/wave17-adversarial/**`、`.dev-team/wave17-red-receipts-l2at.md`
- 未触碰：`src/**`（只读）、`tests/acceptance/wave17/**`（L2-T 写域）、`.dev-team/contracts/**`、任何 git 写操作
- 契约（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 **C4**
- 交付物：`tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts`（14 例）
- 基线：`5d7d691` 工作区 + Protocol Agent 落盘的 `src/lib/ime.ts` 空桩（`throw new Error("NotImplemented")`）

> 工具可用性说明：本会话未暴露 `team_task_get` / task board 工具（已检索工作区与 `~/.dsh`，无该接口）。
> task-5 的完整描述以 `.dev-team/state-wave17.json` 的 `tasks.task-5` + 契约 §2 C4 + Lead 下发的任务书为准，逐条实现。
> 因此未执行 claim/complete 的任务板动作，改由本收据 + 回执消息向 Lead 交付。

---

## 0. 探针验证（写断言之前先验证环境透传，杜绝假测试）

**问题**：`new KeyboardEvent("keydown", { isComposing: true })` 在 jsdom 上读回 `ev.isComposing` 是否真为 `true`？
若否，必须改用 `Object.defineProperty` 显式定义属性。

**探针原始 stdout**（jsdom 26.1.0，临时探针文件已删除）：

```
PROBE_CTOR_isComposing= true key= "Enter" keyCode= 0 which= 0
PROBE_CTOR_keyCode229= 229 isComposing= false
PROBE_DEFAULT_isComposing= false keyCode= 0
PROBE_MOUSE_instanceof_KeyboardEvent= false keyCode= undefined
PROBE_DEFPROP_isComposing= true
PROBE_DEFPROP_keyCode= 229 isComposing= false
PROBE_DISPATCH_isComposing_true= {"isComposing":true,"keyCode":0,"key":"Enter","type":"keydown","ctor":"KeyboardEvent"}
PROBE_DISPATCH_isComposing_false= {"isComposing":false,"keyCode":0,"key":"Enter","type":"keydown","ctor":"KeyboardEvent"}
```

**`@vue/test-utils` `trigger()` 透传探针**（在真实挂载元素上挂原生监听读取派发结果）：

```
PROBE3_trigger_seen= [{"isComposing":true,"keyCode":229,"key":"Enter","ctor":"KeyboardEvent"},{"isComposing":false,"keyCode":0,"key":"Enter","ctor":"KeyboardEvent"}]
```

**结论（探针事实）**：
1. jsdom 26.1.0 **真实透传** `isComposing` 与 `keyCode` 到 `KeyboardEvent` 实例，构造器与 `@vue/test-utils trigger()` 两条路径均已实测确认 → **无需 defineProperty 兜底**。
2. `MouseEvent` 不是 `KeyboardEvent` 子类，`keyCode` 为 `undefined` → 可直接作为「非键盘事件穿透」向量。
3. 本波仍保留一条 `Object.defineProperty` **显式定义** `isComposing: undefined` / `keyCode: undefined` 的敌意事件形状用例（属性形状不受构造器默认值保护），作为 C4-1 的额外对抗向量。

**六处入口的基线（红）探针实测**（当前未修复代码，`@/lib/ime` 未被任何入口调用）：

```
PROBE2_composing_submit_calls= 1        # ASK 自定义输入框 isComposing=true 回车 → 误触提交
PROBE2_noncomposing_submit_calls= 1     # 非组合态回车 → 提交（基线正确）
PROBE2_keycode229_submit_calls= 1       # keyCode=229 → 误触提交
PROBE2_keycode0_submit_calls= 1         # keyCode=0 → 提交（基线正确）
PROBE2_option_composing_calls= 1        # ASK 选项按钮组合态回车 → 误触提交
PROBE2_textarea_composing_calls= 1      # ASK textarea 组合态回车 → 误触提交
PROBE3_toggle_composing_emits= 1        # 「显示思考」组合态回车 → 误触切换
PROBE3_session_composing_rename_calls= 1 # 会话重命名组合态回车 → 误触提交
PROBE4_ws_composing_calls= 1            # 工作区重命名组合态回车 → 误触提交
```

---

## 1. 红相物理收据（真实机器 Exit Code）

命令：`npx vitest run tests/acceptance/wave17-adversarial`

**EXIT = 1**（≠ 0，红相成立）

stdout 摘要（连续 3 次运行结果完全一致）：

```
 RUN  v4.1.9 /home/dm/pony-agent

 ❯ tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts (14 tests | 12 failed) 488ms
     × undefined / null / MouseEvent 入参时守卫必须返回 false 且不抛异常 16ms
     × isComposing === undefined（含显式 defineProperty 置 undefined）时守卫返回 false 且不抛异常 3ms
     × keyCode === 0 视为非组合态；isComposing=true / keyCode=229 视为组合态 3ms
     ✓ 边界穿透后非组合态回车仍必须提交（防吞反向断言） 73ms
     × 组合态（isComposing=true / keyCode=229）回车不得提交 15ms
     × compositionstart → 组合态回车 → compositionend 交错序列仅非组合态回车提交 20ms
     × ASK textarea 在组合序列下不得提交，非组合态必须提交 13ms
     × ASK 选项按钮组合态回车不得触发 handleOptionEnter（选中态不得变化） 16ms
     × 会话重命名输入框组合态回车不得 emit submit-rename 86ms
     × 工作区重命名输入框组合态回车不得提交 submitRename 59ms
     × 「显示思考」菜单项组合态回车不得切换 70ms
     × 200 次交替注入组合/非组合回车：提交计数精确等于 100 39ms
     × 第二轮 200 次交替注入：计数仍精确等于 100（确定性可复现） 36ms
     ✓ 反向压力：200 次全非组合态注入必须精确产生 200 次提交（守卫不得过度拦截） 35ms

 Test Files  1 failed (1)
      Tests  12 failed | 2 passed (14)
```

三次重复运行 `Tests` 行：

```
      Tests  12 failed | 2 passed (14)
      Tests  12 failed | 2 passed (14)
      Tests  12 failed | 2 passed (14)
```

### 1.1 失败原因分类（全部为断言失败 / SUT 栈，**无 setup / import 失败**）

| # | 用例 | 失败类型 | 原始失败摘要 |
| :- | :--- | :--- | :--- |
| 1 | C4-1 undefined/null/MouseEvent 不抛异常 | `AssertionError`（SUT `src/lib/ime.ts:25`） | `expected [Function] to not throw an error but 'Error: NotImplemented' was thrown` |
| 2 | C4-1 isComposing===undefined 不抛异常 | `AssertionError`（SUT 栈） | 同上（`src/lib/ime.ts:25`） |
| 3 | C4-1 keyCode===0 / isComposing=true / 229 语义 | `AssertionError`（SUT 栈） | 同上（`src/lib/ime.ts:25`） |
| 5 | C4-1 组合态回车不得提交 | `AssertionError` | `isComposing=true 的回车不得提交: expected "wrappedAction" to be called +0 times, but got 1 times` |
| 6 | C4-2 组合交错序列仅非组合态提交 | `AssertionError` | `组合态回车不得提交: expected "wrappedAction" to be called +0 times, but got 1 times` |
| 7 | C4-2 textarea 组合序列 | `AssertionError` | `textarea 组合态回车不得提交: expected 0 times, but got 1 times` |
| 8 | C4-2 选项按钮不得触发 handleOptionEnter | `AssertionError` | `选项组合态回车不得提交: expected 0 times, but got 1 times` |
| 9 | C4-2 会话重命名组合态回车 | `AssertionError` | `会话重命名组合态回车不得提交: expected 0 times, but got 1 times` |
| 10 | C4-2 工作区重命名组合态回车 | `AssertionError` | `工作区重命名组合态回车不得提交: expected 0 times, but got 1 times` |
| 11 | C4-2 「显示思考」组合态回车 | `AssertionError` | `组合态回车不得切换: expected [ [ true ] ] to have a length of +0 but got 1` |
| 12 | C4-3 200 次交替注入 | `AssertionError` | `提交次数必须精确等于非组合态注入次数: expected to be called 100 times, but got 200 times` |
| 13 | C4-3 第二轮复现 | `AssertionError` | `第一轮提交计数: expected to be called 100 times, but got 200 times` |

`NotImplemented` 是 SUT（`src/lib/ime.ts`）自身抛出的错误，经由 `.not.toThrow()` 断言失败暴露，
**不是** import/setup 失败；`src/lib/ime.ts` 存在且导出签名可用（否则整个 spec 会在收集期报错而非 12 例断言失败）。

---

## 2. 覆盖矩阵（契约 C4 逐条 → 用例）

| 契约条目 | 覆盖用例 | 红相 |
| :--- | :--- | :--- |
| C4-1 `event === undefined` 不抛异常 | `undefined / null / MouseEvent 入参时守卫必须返回 false 且不抛异常` | **红** |
| C4-1 `isComposing === undefined` 不抛异常 | `isComposing === undefined（含显式 defineProperty 置 undefined）...`（含裸对象 `{key:"Enter"}` 与 `{}`） | **红** |
| C4-1 `keyCode === 0` | `keyCode === 0 视为非组合态；...` + `边界穿透后非组合态回车仍必须提交` | **红** / 反向已满足 |
| C4-1 传入 `MouseEvent` 不抛异常 | 同上第 1 例 | **红** |
| C4-1 非组合态仍提交 | `边界穿透后非组合态回车仍必须提交（防吞反向断言）`（keyCode=0 + defineProperty 敌意形状） | 基线已满足（见 §3） |
| C4-2 `compositionstart → keydown(Enter,isComposing=true) → compositionend` 交错 | `compositionstart → 组合态回车 → compositionend 交错序列仅非组合态回车提交`（8 阶段，含 Safari 尾回车与孤立 `compositionend`） | **红** |
| C4-2 仅非组合态产生提交（textarea / 选项 / 重命名 / 菜单项） | textarea、选项按钮、会话重命名、工作区重命名、「显示思考」共 5 例 | **红** |
| C4-3 固定 200 次交替，提交精确等于非组合态次数 | `200 次交替注入组合/非组合回车：提交计数精确等于 100` | **红** |
| C4-3 确定性（无 sleep / 无概率等待 / 无随机） | 同上 + `第二轮 ... 确定性可复现` | **红** |
| 防过度拦截（守卫不得吞掉正常提交） | `反向压力：200 次全非组合态注入必须精确产生 200 次提交` | 基线已满足（见 §3） |

六处入口覆盖对照（`src` 行号 → 用例）：

| 入口 | src 行 | 覆盖用例 |
| :--- | :--- | :--- |
| ASK 选项按钮 `composer-ask-option-item-N` | `WorkspaceComposer.vue:868` | C4-2 选项按钮用例 |
| ASK 自定义输入框 `composer-ask-custom-input` | `WorkspaceComposer.vue:917` | C4-1 组合态/边界、C4-2 交错序列、C4-3 全部 |
| ASK textarea `composer-ask-textarea` | `WorkspaceComposer.vue:938` | C4-2 textarea 用例 |
| 「显示思考」`reasoning-visibility-toggle` | `WorkspaceComposer.vue:1368` | C4-2 菜单项用例 |
| 会话重命名 `session-rename-input` | `HomeSessionSidebarRow.vue:94` | C4-2 会话重命名用例 |
| 工作区重命名 `workspace-rename-input` | `HomeSessionSidebar.vue:828` | C4-2 工作区重命名用例 |

---

## 3. 基线已满足（非本波拦截点）的显式标注 —— 不得伪红

以下 2 例在当前未修复代码上**通过**，属**基线已满足**，本波守卫的价值是「保持其为绿」（防过度拦截反向门禁）：

| 用例 | 通过原因 | 依据 |
| :--- | :--- | :--- |
| C4-1 `边界穿透后非组合态回车仍必须提交（防吞反向断言）` | 当前入口无任何守卫，非组合态回车本来就会提交；该例断言的是「守卫落地后不得把正常提交一起吞掉」，属**反向门禁**，基线天然满足。 | 探针 `PROBE2_noncomposing_submit_calls= 1`、`PROBE2_keycode0_submit_calls= 1` |
| C4-3 `反向压力：200 次全非组合态注入必须精确产生 200 次提交` | 同上：无守卫时 200/200 全提交。落地守卫后若实现过度拦截（例如误判 `keyCode === 0` 为组合态）该例会转红，即本波必须保持其为绿的**回归护栏**。 | 契约 §2 C4-1「且非组合态仍提交」、C1-5「反向断言（防吞）」 |

这两例在红相即为绿，**不构成本波缺陷拦截点**；已在用例标题中以「（防吞反向断言）」「（守卫不得过度拦截）」显式标注。

---

## 4. 确定性来源说明（C4-3）

- **迭代次数硬编码**：`ITERATIONS = 200`（常量，无外部输入）。
- **注入类型为纯函数**：`isComposing = index % 2 === 0` → 组合 100 次、非组合 100 次；期望值由计数器实算并断言（`composingInjected === 100 && nonComposingInjected === 100`），而非硬写。
- **同步派发，无异步窗口**：`await input.trigger("keydown", ...)` → Vue handler 在 `dispatchEvent` 内**同步**执行至 `submitAskAnswer` 的首个 `await`；`askStore.answer` spy 调用发生在该 await 之前，故派发返回时计数已落定。
- **零 timer / 零 sleep / 零 random**：C4-3 三个用例内不含 `setTimeout` / `vi.useFakeTimers` / `Math.random` / 任何等待原语。
- **可复现性双轮验证**：同一 wrapper 上连续两轮各 200 次注入，两轮均须精确等于 100（第二轮前 `mockClear`），排除「首轮侥幸」。
- 非 C4-3 的侧栏用例使用**显式条件轮询** `waitForElement(probe, label, maxRounds=40)`（`flushPromises + nextTick`，有界，无固定 sleep），失败即抛错而非静默通过。

---

## 5. 零假测试自查

- [x] 无 `it.skip` / `it.todo` / `describe.skip`：`grep -c "skip\|todo"` = 0
- [x] 无空断言 / 恒真断言（`expect(true).toBe(true)` 类）＝ 0
- [x] 无 `try/catch` 吞异常放行（守卫边界用例用 `.not.toThrow()` 显式断言，异常向上抛出即失败）
- [x] 每条断言均绑定真实状态突变：store action 调用次数与实参、Vue `emitted` 载荷、选中态 class 突变、守卫返回值
- [x] 断言失败原因全部为 `AssertionError`（12/12），无 setup/import 失败
- [x] 测试数据生命周期：`afterEach` 清空 `document.body`（Teleport 目标）+ `vi.restoreAllMocks()`；无临时文件/无外部服务/无真实凭据（全部 `mockResolvedValue`）
