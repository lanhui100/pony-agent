// B011 前端「可呈现性」最小切片（Linux 上可机器验证部分）。
//
// 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §波次四 · B-2 → B011
// （promote 结果 `timedOut/promoted` 无消费方，UI 按 status 判成「成功完成」）。
//
// 本文件验收纯函数模块 `src/lib/runtime/tool-result-meta.ts`（最小可呈现层）：
// - `parsePromotedToolResult(text)`：把 Run 工具结果文本解析成可呈现元信息
//   `{ promoted, timedOut, jobId }`，**任何非法输入都不得抛异常**（fail-safe 到非 promote）；
// - `promotedToolBadgeLabel(meta)`：promoted 时给出中文徽标文案（含「后台」），否则 `null`。
//
// 红相预期：模块尚不存在 ⇒ 解析 import 失败。

import { describe, expect, it } from "vitest";
import {
  parsePromotedToolResult,
  promotedToolBadgeLabel
} from "@/lib/runtime/tool-result-meta";

describe("parsePromotedToolResult - promote 载荷", () => {
  it("promote 成功载荷必须解析出 promoted/timedOut/jobId 三项", () => {
    // Arrange
    const text = JSON.stringify({
      ok: true,
      timedOut: true,
      promoted: true,
      jobId: "abc",
      command: "sleep 5"
    });

    // Act
    const meta = parsePromotedToolResult(text);

    // Assert：三项必须齐备（用 toMatchObject 允许实现附带额外字段，但不放宽这三项）。
    expect(meta).toMatchObject({ promoted: true, timedOut: true, jobId: "abc" });
    expect(meta.promoted).toBe(true);
    expect(meta.timedOut).toBe(true);
    expect(meta.jobId).toBe("abc");
  });
});

describe("parsePromotedToolResult - 普通成功载荷", () => {
  it("普通成功载荷不得被误判为 promote 或超时", () => {
    // Arrange
    const text = JSON.stringify({ ok: true, timedOut: false, exitCode: 0 });

    // Act
    const meta = parsePromotedToolResult(text);

    // Assert
    expect(meta.promoted).toBe(false);
    expect(meta.timedOut).toBe(false);
    expect(meta.jobId).toBeUndefined();
  });
});

describe("parsePromotedToolResult - 超时错误载荷（promote 关闭路径）", () => {
  it("超时错误载荷必须标出 timedOut，但不得标成 promoted", () => {
    // Arrange：promote 关闭时仍然是 error.code=timeout（CT-P7 / CT-P6a）。
    const text = JSON.stringify({
      ok: false,
      error: { code: "timeout" },
      timedOut: true
    });

    // Act
    const meta = parsePromotedToolResult(text);

    // Assert
    expect(meta.promoted).toBe(false);
    expect(meta.timedOut).toBe(true);
    // 契约只要求「不是有效 jobId」；null 与 undefined 都可接受（非 promote 就没有运行中的作业）。
    expect(meta.jobId == null).toBe(true);
  });
});

describe("parsePromotedToolResult - 非法输入 fail-safe", () => {
  it("非 JSON 文本不得抛异常，且回落为非 promote / 非超时", () => {
    // Arrange
    const text = "not-a-json :: <html>boom</html>";

    // Act
    const meta = parsePromotedToolResult(text);

    // Assert
    expect(meta.promoted).toBe(false);
    expect(meta.timedOut).toBe(false);
    expect(meta.jobId).toBeUndefined();
  });

  it("空串不得抛异常，且回落为非 promote / 非超时", () => {
    // Arrange
    const text = "";

    // Act
    const meta = parsePromotedToolResult(text);

    // Assert
    expect(meta.promoted).toBe(false);
    expect(meta.timedOut).toBe(false);
  });

  it("undefined 不得抛异常，且回落为非 promote / 非超时", () => {
    // Arrange：历史工具结果可能没有 output 文本（契约要求 undefined 也不炸）。

    // Act
    const meta = parsePromotedToolResult(undefined);

    // Assert
    expect(meta.promoted).toBe(false);
    expect(meta.timedOut).toBe(false);
  });
});

describe("promotedToolBadgeLabel", () => {
  it("promoted 时返回含「后台」的非空中文标签", () => {
    // Arrange
    const meta = { promoted: true, timedOut: true, jobId: "abc" };

    // Act
    const label = promotedToolBadgeLabel(meta);

    // Assert
    expect(label).not.toBeNull();
    expect(typeof label).toBe("string");
    expect(label!.trim().length).toBeGreaterThan(0);
    expect(label).toContain("后台");
  });

  it("非 promoted 时返回 null（不得渲染空徽标）", () => {
    // Arrange
    const meta = { promoted: false, timedOut: true, jobId: undefined };

    // Act
    const label = promotedToolBadgeLabel(meta);

    // Assert
    expect(label).toBeNull();
  });
});
