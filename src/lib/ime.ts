/**
 * Wave-17 C1 —— IME 组合态（输入法选词）回车守卫的**唯一新增接缝**。
 *
 * 契约（冻结于 `.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C1）：
 * 判定一个键盘/鼠标事件是否处于输入法组合态（即用户正在用拼音/假名等选词，
 * 此时回车是「确认候选词」而非「提交」）。
 *
 * 语义（**实现归 executor-a**，本文件当前为纯空桩）：
 * - 当 `event` 是 `KeyboardEvent` 且满足以下任一条件时返回 `true`：
 *   1. `event.isComposing === true`（标准组合态标记）；
 *   2. `event.keyCode === 229`（旧式 IME 组合态标记，与 isComposing 等价生效）。
 * - 当 `event` 不是 `KeyboardEvent`（例如 `MouseEvent`）、或为 `null`、或为
 *   `undefined` 时返回 `false` —— **不得抛异常**（边界穿透安全）。
 * - 反向断言（防吞）：`isComposing === false` 且 `key === "Enter"` 时必须返回
 *   `false`，使调用方照常提交，守卫不得过度拦截。
 * - 纯函数：无副作用、无 IO、无缓存、无外部状态读取。
 *
 * 注：形参以 `_` 前缀绑定仅为满足 `tsconfig.json` 的 `noUnusedParameters`
 * （空桩未使用形参）；形参类型签名与上述契约完全一致，调用方不受影响。
 *
 * @param event 待判定的原始事件对象（可空，可非键盘事件）
 * @returns 是否处于输入法组合态（true = 应当吞掉本次回车）
 */
export function isImeComposingEvent(_event: Event | null | undefined): boolean {
  throw new Error("NotImplemented");
}
