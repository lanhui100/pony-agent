// P1-2 最小切片红相：工具**失败原因**在桌面 UI 可读（B017 主路径正文穿参重构先搁置）。
//
// 契约真源：trace 报告 P1-2 + P0-3 已落地的后端字段 `error.exitCodeClass`。
//
// 被测模块：`src/lib/runtime/tool-result-meta.ts` 的**新增导出**（纯函数，不碰状态机）：
// - `parseToolExitMeta(text)`：从工具结果文本解析 `{ exitCode?, exitCodeClass? }`；
//   任何非法输入都不得抛异常（fail-safe 回落 `{}`）。
// - `exitCodeClassLabel(meta)`：把分类码翻成中文短标签；`general_failure` / 缺失 ⇒ `null`
//   （不显示噪音）。
//
// 红相预期：新导出尚不存在 ⇒ import 解析失败。

import { describe, expect, it } from "vitest";
import {
  exitCodeClassLabel,
  parsePromotedToolResult,
  parseToolExitMeta
} from "@/lib/runtime/tool-result-meta";

/** 只读被测字段的窄视图（避免测试依赖实现内部类型导出）。 */
type ExitMetaView = { exitCode?: number | null; exitCodeClass?: string | null };

describe("parseToolExitMeta - 非零退出载荷", () => {
  it("必须同时解析出 exitCode 与 exitCodeClass", () => {
    // Arrange
    const text = JSON.stringify({
      ok: false,
      error: {
        code: "non_zero_exit",
        exitCode: 101,
        exitCodeClass: "compile_or_panic"
      }
    });

    // Act
    const meta = parseToolExitMeta(text) as ExitMetaView;

    // Assert
    expect(meta.exitCode).toBe(101);
    expect(meta.exitCodeClass).toBe("compile_or_panic");
  });
});

describe("parseToolExitMeta - 成功载荷", () => {
  it("成功载荷只带 exitCode，不得伪造 exitCodeClass", () => {
    // Arrange
    const text = JSON.stringify({ ok: true, exitCode: 0 });

    // Act
    const meta = parseToolExitMeta(text) as ExitMetaView;

    // Assert
    expect(meta.exitCode).toBe(0);
    expect(meta.exitCodeClass).toBeUndefined();
  });
});

describe("parseToolExitMeta - 非法输入 fail-safe", () => {
  it("非 JSON 文本不得抛异常，且回落为空元信息", () => {
    // Act
    const meta = parseToolExitMeta("not-a-json :: <html>boom</html>");

    // Assert
    expect(meta).toEqual({});
  });

  it("空串不得抛异常，且回落为空元信息", () => {
    // Act
    const meta = parseToolExitMeta("");

    // Assert
    expect(meta).toEqual({});
  });

  it("undefined 不得抛异常，且回落为空元信息", () => {
    // Act
    const meta = parseToolExitMeta(undefined);

    // Assert
    expect(meta).toEqual({});
  });
});

describe("exitCodeClassLabel - 中文短标签", () => {
  it("compile_or_panic ⇒ 含「编译」或「panic」", () => {
    // Act
    const label = exitCodeClassLabel({ exitCodeClass: "compile_or_panic" });

    // Assert
    expect(label).not.toBeNull();
    const lowered = (label ?? "").toLowerCase();
    expect(lowered.includes("编译") || lowered.includes("panic")).toBe(true);
  });

  it("command_not_found ⇒ 含「命令不存在」", () => {
    expect(exitCodeClassLabel({ exitCodeClass: "command_not_found" })).toContain("命令不存在");
  });

  it("not_executable ⇒ 含「不可执行」", () => {
    expect(exitCodeClassLabel({ exitCodeClass: "not_executable" })).toContain("不可执行");
  });

  it("interrupted ⇒ 含「中断」", () => {
    expect(exitCodeClassLabel({ exitCodeClass: "interrupted" })).toContain("中断");
  });

  it("killed ⇒ 含「已终止」或「杀死」", () => {
    const label = exitCodeClassLabel({ exitCodeClass: "killed" }) ?? "";
    expect(label.includes("已终止") || label.includes("杀死")).toBe(true);
  });

  it("general_failure ⇒ null（兜底分类不显示噪音）", () => {
    expect(exitCodeClassLabel({ exitCodeClass: "general_failure" })).toBeNull();
  });

  it("缺失 exitCodeClass ⇒ null", () => {
    expect(exitCodeClassLabel({ exitCode: 7 })).toBeNull();
  });
});

describe("与 parsePromotedToolResult 互不干扰", () => {
  it("同一段 promote 载荷：promote 字段与 exit 字段各自独立正确", () => {
    // Arrange
    const text = JSON.stringify({
      ok: true,
      timedOut: true,
      promoted: true,
      jobId: "abc",
      command: "sleep 5",
      exitCode: null
    });

    // Act
    const promoted = parsePromotedToolResult(text);
    const exitMeta = parseToolExitMeta(text) as ExitMetaView;

    // Assert：promote 解析不受 exit 字段影响
    expect(promoted.promoted).toBe(true);
    expect(promoted.timedOut).toBe(true);
    expect(promoted.jobId).toBe("abc");
    // Assert：exit 解析不伪造分类
    expect(exitMeta.exitCodeClass).toBeUndefined();
    expect(exitMeta.exitCode ?? null).toBeNull();
  });
});
