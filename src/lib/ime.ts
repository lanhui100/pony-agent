/**
 * Wave-17 C1 —— IME 组合态（输入法选词）回车守卫的**唯一新增接缝**。
 *
 * 契约（冻结于 `.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C1）：
 * 判定一个键盘/鼠标事件是否处于输入法组合态（即用户正在用拼音/假名等选词，
 * 此时回车是「确认候选词」而非「提交」）。
 *
 * 语义（B099 修订）：键盘事件**按品牌串**（`Object.prototype.toString`）识别，不用
 * `instanceof` —— `instanceof` 只认同当前 realm 的构造器，iframe / WebView / 多 window
 * 产生的事件品牌正确却会被判成非键盘事件 ⇒ 组合态漏判 ⇒ 选词回车误触提交。
 * - 判定为键盘事件后，满足任一条件返回 `true`：
 *   1. `event.isComposing` 为真值（标准组合态标记；非布尔真值同权，与组件内联判定一致）；
 *   2. `event.keyCode === 229`（旧式 IME 组合态标记，与 isComposing 等价生效）。
 * - 非键盘事件（例如 `MouseEvent`）、`null`、`undefined` 一律返回 `false` ——
 *   **不得抛异常**（边界穿透安全；读取异常 fail-closed 为 `false`）。
 * - 反向断言（防吞）：`isComposing` 为假且 `keyCode !== 229` 时必须返回 `false`，
 *   使调用方照常提交，守卫不得过度拦截。
 * - 纯函数：无副作用、无 IO、无缓存、无外部状态读取。
 *
 * 注：形参类型签名与上述契约完全一致，调用方不受影响。
 *
 * @param event 待判定的原始事件对象（可空，可非键盘事件）
 * @returns 是否处于输入法组合态（true = 应当吞掉本次回车）
 */
const KEYBOARD_EVENT_BRAND = "[object KeyboardEvent]";

function isKeyboardEventLike(event: Event): boolean {
  try {
    return Object.prototype.toString.call(event) === KEYBOARD_EVENT_BRAND;
  } catch {
    return false;
  }
}

export function isImeComposingEvent(event: Event | null | undefined): boolean {
  if (event === null || event === undefined) return false;
  if (!isKeyboardEventLike(event)) return false;
  try {
    const candidate = event as KeyboardEvent;
    return Boolean(candidate.isComposing) || candidate.keyCode === 229;
  } catch {
    return false;
  }
}
