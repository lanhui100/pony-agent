// C2（P1-2 收尾）红相：turn 健康度徽标文案。
//
// 契约真源：P1-2（`degraded` / `toolFailures` 从 OTel span 属性带到本地会话数据后，UI 要能显示）。
// 被测模块（新增）：`src/lib/runtime/turn-health.ts` 的 `turnHealthLabel(turn)`。
//
// 要求：
// 1. `degraded === true` ⇒ 返回含「降级」的短标签；
// 2. `toolFailures > 0` 且未 degraded ⇒ 返回含「工具失败」与失败条数的短标签；
// 3. 两者皆无（false/0/缺失）⇒ `null`（不显示噪音）；
// 4. 字段缺失 / 畸形输入（null、非对象字段类型错）⇒ `null`，**不得抛**。
//
// 红相预期：模块/导出尚不存在 ⇒ import 失败。

import { describe, expect, it } from "vitest";
import { turnHealthLabel } from "@/lib/runtime/turn-health";

describe("turnHealthLabel - degraded", () => {
  it("degraded=true ⇒ 返回含「降级」的标签", () => {
    // Act
    const label = turnHealthLabel({ degraded: true, toolFailures: 2 });

    // Assert
    expect(label).not.toBeNull();
    expect(label!).toContain("降级");
  });
});

describe("turnHealthLabel - 工具失败（未降级）", () => {
  it("toolFailures>0 且未 degraded ⇒ 含「工具失败」与条数", () => {
    // Act
    const label = turnHealthLabel({ degraded: false, toolFailures: 3 });

    // Assert
    expect(label).not.toBeNull();
    expect(label!).toContain("工具失败");
    expect(label!).toContain("3");
  });
});

describe("turnHealthLabel - 健康轮次不显示噪音", () => {
  it("两者皆无 ⇒ null", () => {
    expect(turnHealthLabel({})).toBeNull();
  });

  it("degraded=false 且 toolFailures=0 ⇒ null", () => {
    expect(turnHealthLabel({ degraded: false, toolFailures: 0 })).toBeNull();
  });

  it("degraded=false 且 toolFailures 缺失 ⇒ null", () => {
    expect(turnHealthLabel({ degraded: false })).toBeNull();
  });
});

describe("turnHealthLabel - 畸形输入兜底", () => {
  it("null ⇒ null 且不抛", () => {
    expect(turnHealthLabel(null as unknown as Parameters<typeof turnHealthLabel>[0])).toBeNull();
  });

  it("字段类型畸形（字符串/负数值）⇒ null 且不抛", () => {
    const malformed = {
      degraded: "true",
      toolFailures: -1
    } as unknown as Parameters<typeof turnHealthLabel>[0];
    expect(turnHealthLabel(malformed)).toBeNull();
  });
});
