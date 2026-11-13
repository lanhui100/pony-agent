/**
 * 指标诚实百分比验收（ADR `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`）。
 *
 * 主诉：TUI / 桌面右上角「缓存 x%」把 0.996 这类「几乎全命中」整数四舍五入成 100%，
 * 等于把部分命中谎报成全命中。本文件锁死前端侧规则，与 Rust
 * `agent/metrics_snapshot.rs::format_ratio_percent` / `format_unit_fraction_percent` 同构。
 *
 * 纪律：固定夹具、无随机、无 sleep、无 skip；每条用例均有具体字符串断言。
 * 独立于冻结文件 `tests/metrics-snapshot-live.spec.ts`（该文件由 Test Agent 独占写域）。
 */
import { describe, expect, it } from "vitest";
import {
  formatUnitFractionPercent,
  resolveMetricsSnapshotDisplay,
} from "@/lib/runtime/token-metrics";
import type { TurnMetricsSnapshot } from "@/types/runtime";

describe("诚实百分比 formatUnitFractionPercent", () => {
  it("H1: 0.996 ⇒ 99.6%（整数四舍五入会顶到 100，必须降精度向下取整）", () => {
    expect(formatUnitFractionPercent(0.996)).toBe("99.6%");
  });

  it("H2: 0.99999 ⇒ 99.99%", () => {
    expect(formatUnitFractionPercent(0.99999)).toBe("99.99%");
  });

  it("H3: 0.9999999（贴顶）⇒ 形态为 99.x，绝不输出 100%", () => {
    const formatted = formatUnitFractionPercent(0.9999999);
    expect(formatted.startsWith("99.")).toBe(true);
    expect(formatted).not.toBe("100%");
  });

  it("H4: 无值 / 非有限 / 越界 ⇒ 空串（fail-closed，不出 0% 也不 clamp）", () => {
    expect(formatUnitFractionPercent(null)).toBe("");
    expect(formatUnitFractionPercent(Number.NaN)).toBe("");
    expect(formatUnitFractionPercent(Number.POSITIVE_INFINITY)).toBe("");
    expect(formatUnitFractionPercent(-0.1)).toBe("");
    expect(formatUnitFractionPercent(1.2)).toBe("");
  });

  it("H5: 真全命中 1 ⇒ 100%；普通值仍整数四舍五入（0.6667 ⇒ 67%，0 ⇒ 0%）", () => {
    expect(formatUnitFractionPercent(1)).toBe("100%");
    expect(formatUnitFractionPercent(0.6667)).toBe("67%");
    expect(formatUnitFractionPercent(0)).toBe("0%");
  });
});

describe("resolveMetricsSnapshotDisplay 接入诚实百分比", () => {
  it("H6: cacheHitRatio 0.996 ⇒ cacheRatioPercent 为 99.6%，不是 100%", () => {
    const snapshot = { cacheHitRatio: 0.996 } as unknown as TurnMetricsSnapshot;
    const display = resolveMetricsSnapshotDisplay(snapshot);
    expect(display.cacheRatioPercent).toBe("99.6%");
    expect(display.cacheRatioPercent).not.toBe("100%");
  });

  it("H7: cacheHitRatio 缺失 ⇒ 空串（既有 I2/I6 纪律不回退）", () => {
    const snapshot = { cacheHitRatio: null } as unknown as TurnMetricsSnapshot;
    expect(resolveMetricsSnapshotDisplay(snapshot).cacheRatioPercent).toBe("");
  });
});
