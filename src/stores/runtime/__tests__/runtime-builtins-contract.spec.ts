/// <reference types="vite/client" />
/**
 * B278 机械守卫 · Pinia 内建成员在 runtime store 的契约边界
 *
 * 裁决登记于 `src/stores/runtime/types.ts` 的 `RuntimeStoreBuiltins`：
 * - `$patch`：纳入契约（仅 turn-events 监听器聚合体的批内原子写 + 测试 Arrange）；
 * - `$reset`：明令禁止（重置一律走 `resetSessionRuntimeState()`，它带清空面/保留面契约）；
 * - `$subscribe`：明令禁止（生产面观测走 action/getter，避免隐式订阅泄漏）。
 *
 * 本测试用源码扫描把「明令禁止」变成可机器验收的承诺（宪法常载命约第 2 条）。
 * 读取源码用 Vite 的 `import.meta.glob(...?raw)`，避免给前端 tsconfig 引入 Node 类型。
 */
import { describe, expect, it } from "vitest";

/** 全部源码模块的原始文本（key 为 `/src/...` 形式的项目相对路径）。 */
const SOURCE_MODULES = import.meta.glob("/src/**/*.{ts,vue}", {
  query: "?raw",
  import: "default",
  eager: true
}) as Record<string, string>;

/** `$patch` 的受控白名单：turn-events 监听器聚合体的批内原子写（B278 裁决唯一豁免面）。 */
const PATCH_ALLOWLIST = new Set(["src/stores/runtime/turn-events.ts"]);

/** 返回命中的「项目相对路径」清单（只匹配调用形态，类型声明不命中）。 */
function scanCalls(pattern: RegExp): string[] {
  return Object.entries(SOURCE_MODULES)
    .filter(([path]) => !path.includes("/__tests__/") && !path.endsWith(".spec.ts"))
    .filter(([, text]) => pattern.test(text))
    .map(([path]) => path.replace(/^\//, ""))
    .sort();
}

describe("B278 · Pinia 内建成员契约守卫", () => {
  it("生产面禁止调用 $reset（重置必须走 resetSessionRuntimeState）", () => {
    expect(scanCalls(/\.\$reset\s*\(/)).toEqual([]);
  });

  it("生产面禁止调用 $subscribe（观测走 action/getter）", () => {
    expect(scanCalls(/\.\$subscribe\s*\(/)).toEqual([]);
  });

  it("$patch 调用只出现在受控白名单", () => {
    const offenders = scanCalls(/\.\$patch\s*\(/).filter((path) => !PATCH_ALLOWLIST.has(path));
    expect(offenders).toEqual([]);
  });

  it("白名单不腐化：turn-events.ts 仍实际持有 $patch 调用", () => {
    expect(scanCalls(/\.\$patch\s*\(/)).toContain("src/stores/runtime/turn-events.ts");
  });
});
