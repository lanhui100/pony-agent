import { test, expect } from 'vitest';

test('B098: IME composition guard should not swallow Ctrl+Z', () => {
    const isEnterKey = (key: string, keyCode: number) => key === 'Enter' || keyCode === 13;
    const shouldBlock = (e: { keyCode: number, key: string, ctrlKey: boolean }) => {
        // 守卫收窄：仅在组合态且为 Enter 时拦截
        if (e.keyCode === 229 && isEnterKey(e.key, e.keyCode)) return true;
        return false;
    };

    const ctrlZEvent = { keyCode: 229, key: 'z', ctrlKey: true };
    expect(shouldBlock(ctrlZEvent)).toBe(false);

    const enterEvent = { keyCode: 229, key: 'Enter', ctrlKey: false };
    expect(shouldBlock(enterEvent)).toBe(true);
});
