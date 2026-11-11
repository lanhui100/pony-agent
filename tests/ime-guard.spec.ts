/**
 * B099 —— IME 组合态判定与组件原生真值判定对齐（跨 realm / 非布尔状态）。
 *
 * 缺陷本体：`isImeComposingEvent` 用 `event instanceof KeyboardEvent` 作为唯一闸门。
 * `instanceof` 只认同一个 realm 的构造器；iframe / WebView / 多 window 里产生的事件对象
 * 品牌是 `KeyboardEvent` 但构造器来自另一个 realm ⇒ 守卫返回 `false` ⇒ 组合态回车被
 * 当成「显式提交」放行，中文/日文/韩文选词回车误触提交（wave-17 C1 的原始缺陷复活）。
 *
 * 判定口径真源（对齐 `HomeWorkspace.vue` 的存量内联判定 `event.isComposing || event.keyCode === 229`）：
 *   1. 键盘类事件：`isComposing` 真值 或 `keyCode === 229`；
 *   2. 非键盘事件 / null / undefined：`false`，且**不得抛异常**；
 *   3. 真值语义与内联判定一致（`isComposing = 1` 之类非布尔真值同权生效），
 *      不得因 `=== true` 的严格比较而漏拦。
 *
 * 零假测试律：每个 `it` 都断言真实返回值；跨 realm 用例先自证「构造器确实来自另一个 realm」
 * （`instanceof` 为假），否则用例自身即假测试。
 */
import { afterEach, describe, expect, it } from "vitest";
import { isImeComposingEvent } from "@/lib/ime";

const KEYBOARD_BRAND = "[object KeyboardEvent]";

function defineProps<T extends object>(target: T, props: Record<string, unknown>): T {
  for (const [key, value] of Object.entries(props)) {
    Object.defineProperty(target, key, { value, configurable: true, writable: true });
  }
  return target;
}

/** 构造一个来自**另一个 realm**（iframe contentWindow）的键盘事件，并自证其跨 realm 身份。 */
function createForeignKeyboardEvent(init: KeyboardEventInit): KeyboardEvent {
  const iframe = document.createElement("iframe");
  document.body.appendChild(iframe);
  const foreignWindow = iframe.contentWindow as (Window & typeof globalThis) | null;
  expect(foreignWindow, "iframe contentWindow 必须存在").toBeTruthy();
  const ForeignKeyboardEvent = foreignWindow!.KeyboardEvent;
  expect(ForeignKeyboardEvent, "iframe 必须暴露 KeyboardEvent 构造器").toBeTypeOf("function");
  const event = new ForeignKeyboardEvent("keydown", { key: "Enter", ...init });
  // 自证：品牌串是键盘事件，但 instanceof 本地构造器为假 —— 这正是缺陷的触发条件。
  expect(Object.prototype.toString.call(event)).toBe(KEYBOARD_BRAND);
  expect(event instanceof KeyboardEvent, "跨 realm 事件不得被本地 instanceof 认同").toBe(false);
  return event as unknown as KeyboardEvent;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("B099 · 同 realm 基础真值矩阵（保持 wave-17 C1/C4 冻结语义）", () => {
  it("null / undefined / 非键盘事件返回 false 且不抛异常", () => {
    expect(() => isImeComposingEvent(undefined)).not.toThrow();
    expect(isImeComposingEvent(undefined)).toBe(false);
    expect(isImeComposingEvent(null)).toBe(false);

    const mouseEvent = new MouseEvent("click", { bubbles: true });
    expect(() => isImeComposingEvent(mouseEvent)).not.toThrow();
    expect(isImeComposingEvent(mouseEvent)).toBe(false);

    const bare = { key: "Enter" } as unknown as Event;
    expect(() => isImeComposingEvent(bare)).not.toThrow();
    expect(isImeComposingEvent(bare)).toBe(false);
    expect(isImeComposingEvent({} as unknown as Event)).toBe(false);
  });

  it("isComposing=true 与 keyCode=229 判定为组合态；false / 0 / 13 判定为非组合态", () => {
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true }))).toBe(true);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229 }))).toBe(true);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: false }))).toBe(false);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 0 }))).toBe(false);
    expect(isImeComposingEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 13 }))).toBe(false);
  });

  it("字段被显式定义为 undefined 时不误判（防吞）", () => {
    const hostile = defineProps(new KeyboardEvent("keydown", { key: "Enter" }), {
      isComposing: undefined,
      keyCode: undefined
    });
    expect(() => isImeComposingEvent(hostile)).not.toThrow();
    expect(isImeComposingEvent(hostile)).toBe(false);
  });
});

describe("B099 · 跨 realm 判定（instanceof 闸门的失效面）", () => {
  it("跨 realm 组合态回车（isComposing=true）必须判定为组合态", () => {
    const event = createForeignKeyboardEvent({ isComposing: true });
    expect(isImeComposingEvent(event)).toBe(true);
  });

  it("跨 realm 旧式 IME 标记（keyCode=229）必须判定为组合态", () => {
    const event = createForeignKeyboardEvent({ keyCode: 229 });
    expect(isImeComposingEvent(event)).toBe(true);
  });

  it("跨 realm 非组合态回车必须判定为非组合态（防吞反向断言）", () => {
    const event = createForeignKeyboardEvent({ isComposing: false });
    expect(isImeComposingEvent(event)).toBe(false);
  });

  it("跨 realm 的非键盘事件（MouseEvent）仍返回 false", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    const foreignWindow = iframe.contentWindow as Window & typeof globalThis;
    const foreignMouse = new foreignWindow.MouseEvent("click", { bubbles: true });
    expect(Object.prototype.toString.call(foreignMouse)).toBe("[object MouseEvent]");
    expect(isImeComposingEvent(foreignMouse as unknown as Event)).toBe(false);
  });
});

describe("B099 · 非布尔真值口径对齐（与组件内联判定同集合）", () => {
  it("isComposing 为非布尔真值时按真值语义判定为组合态", () => {
    const truthy = defineProps(new KeyboardEvent("keydown", { key: "Enter" }), { isComposing: 1 });
    expect(isImeComposingEvent(truthy)).toBe(true);

    const truthyString = defineProps(new KeyboardEvent("keydown", { key: "Enter" }), { isComposing: "true" });
    expect(isImeComposingEvent(truthyString)).toBe(true);
  });

  it("isComposing 为非布尔假值时判定为非组合态", () => {
    const falsyZero = defineProps(new KeyboardEvent("keydown", { key: "Enter" }), { isComposing: 0 });
    expect(isImeComposingEvent(falsyZero)).toBe(false);

    const falsyEmpty = defineProps(new KeyboardEvent("keydown", { key: "Enter" }), { isComposing: "" });
    expect(isImeComposingEvent(falsyEmpty)).toBe(false);
  });

  it("判定集合与组件原生内联判定（isComposing || keyCode === 229）逐档一致", () => {
    const native = (event: Event): boolean => {
      const candidate = event as KeyboardEvent;
      return Boolean(candidate.isComposing) || candidate.keyCode === 229;
    };
    const matrix: Event[] = [
      new KeyboardEvent("keydown", { key: "Enter" }),
      new KeyboardEvent("keydown", { key: "Enter", isComposing: true }),
      new KeyboardEvent("keydown", { key: "Enter", isComposing: false }),
      new KeyboardEvent("keydown", { key: "Enter", keyCode: 229 }),
      new KeyboardEvent("keydown", { key: "Enter", keyCode: 13 }),
      createForeignKeyboardEvent({ isComposing: true }),
      createForeignKeyboardEvent({ isComposing: false }),
      createForeignKeyboardEvent({ keyCode: 229 })
    ];
    for (const event of matrix) {
      expect(isImeComposingEvent(event), `判定漂移：${Object.prototype.toString.call(event)}`).toBe(native(event));
    }
  });
});

describe("B099 · 品牌串判定不得被伪造成误吞", () => {
  it("非键盘对象通过 Symbol.toStringTag 伪造品牌串时仍不误判为组合态", () => {
    const forged = {
      key: "Enter",
      [Symbol.toStringTag]: "KeyboardEvent"
    } as unknown as Event;
    expect(Object.prototype.toString.call(forged)).toBe(KEYBOARD_BRAND);
    // 伪造品牌但无任何键盘组合态字段 ⇒ 不得吞掉回车
    expect(isImeComposingEvent(forged)).toBe(false);
  });
});
