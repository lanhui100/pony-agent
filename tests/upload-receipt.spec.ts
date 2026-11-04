// P1-3（B · 前端）红相：上传回执的可读反馈。
//
// 契约真源：trace 报告 P1-3。被测模块（新增）：`src/lib/runtime/upload-receipt.ts`
// 的 `formatUploadFeedback(receipt)` —— 把后端结构化回执
// `{ turns, traceIds, failures }` 翻成一句用户可读的中文回执。
//
// 要求：
// 1. 全成功 ⇒ 含 turn 数与**每个** trace id 的短 id（前 8 位）且提到 Langfuse；
// 2. 部分失败 ⇒ 同时含「部分成功」语义与失败条数；
// 3. 没有任何可上传 trace ⇒ 含「没有可上传」语义，且**不得**出现「成功」字样；
// 4. 空/畸形输入 ⇒ 返回兜底文案且**不抛**。
//
// 红相预期：模块/导出尚不存在 ⇒ import 失败。

import { describe, expect, it } from "vitest";
import { formatUploadFeedback } from "@/lib/runtime/upload-receipt";

/** 短 id 口径：前 8 位（与 UI 展示一致）。 */
const shortId = (id: string) => id.slice(0, 8);

describe("formatUploadFeedback - 全部成功", () => {
  it("必须含 turn 数、每个 trace id 的短 id，并提到 Langfuse", () => {
    // Arrange
    const receipt = {
      turns: 4,
      traceIds: ["aaaaaaaa1111", "bbbbbbbb2222", "cccccccc3333", "dddddddd4444"],
      failures: []
    };

    // Act
    const text = formatUploadFeedback(receipt);

    // Assert
    expect(text).toContain("4");
    expect(text.toLowerCase()).toContain("langfuse");
    for (const id of receipt.traceIds) {
      expect(text).toContain(shortId(id));
    }
  });
});

describe("formatUploadFeedback - 部分成功", () => {
  it("必须同时含「部分成功」语义与失败条数", () => {
    // Arrange
    const receipt = {
      turns: 2,
      traceIds: ["aaaaaaaa1111", "bbbbbbbb2222"],
      failures: ["chunk 2: timeout"]
    };

    // Act
    const text = formatUploadFeedback(receipt);

    // Assert
    expect(text).toContain("部分成功");
    expect(text).toContain(String(receipt.failures.length));
  });
});

describe("formatUploadFeedback - 无可上传内容", () => {
  it("必须提示「没有可上传」且不得出现「成功」字样", () => {
    // Arrange
    const receipt = { turns: 0, traceIds: [], failures: [] };

    // Act
    const text = formatUploadFeedback(receipt);

    // Assert
    expect(text).toContain("没有可上传");
    expect(text).not.toContain("成功");
  });
});

describe("formatUploadFeedback - 畸形输入兜底", () => {
  it("null 不得抛异常，且返回非空兜底文案", () => {
    // Act
    const text = formatUploadFeedback(null as unknown as Parameters<typeof formatUploadFeedback>[0]);

    // Assert
    expect(typeof text).toBe("string");
    expect(text.trim().length).toBeGreaterThan(0);
  });

  it("缺字段对象不得抛异常，且返回非空兜底文案", () => {
    // Act
    const text = formatUploadFeedback({} as unknown as Parameters<typeof formatUploadFeedback>[0]);

    // Assert
    expect(typeof text).toBe("string");
    expect(text.trim().length).toBeGreaterThan(0);
  });
});
