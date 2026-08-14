import { describe, expect, it } from "vitest";
import {
  TURN_HEADER_BASE_HEIGHT,
  TIMELINE_ENTRY_BASE_HEIGHT,
  buildTurnPrefixHeights,
  computeVirtualTurnWindow,
  estimateTurnHeight,
  findTurnIndexAtScroll
} from "@/lib/runtime/trace-virtual-scroll";
import type { TraceTimelineEntry } from "@/types/runtime";

const timeline = (count: number): TraceTimelineEntry[] =>
  Array.from({ length: count }, (_, i) => ({
    id: `entry-${i}`,
    kind: "model" as const,
    label: `ENTRY ${i}`,
    state: "completed" as const,
    sequence: i + 1
  }));

describe("trace-virtual-scroll", () => {
  it("estimateTurnHeight：展开时 header + 条目数 × 条目高度", () => {
    const height = estimateTurnHeight(timeline(3), true, 0);
    expect(height).toBe(TURN_HEADER_BASE_HEIGHT + 6 + 3 * TIMELINE_ENTRY_BASE_HEIGHT);
  });

  it("estimateTurnHeight：折叠时只算 header（body 不可见）", () => {
    const collapsed = estimateTurnHeight(timeline(5), false, 0);
    expect(collapsed).toBe(TURN_HEADER_BASE_HEIGHT + 6);
  });

  it("estimateTurnHeight：展开详情增加行高", () => {
    const collapsed = estimateTurnHeight(timeline(2), false, 0);
    const expanded = estimateTurnHeight(timeline(2), true, 5);
    expect(expanded).toBeGreaterThan(collapsed);
    expect(expanded - collapsed).toBe(TIMELINE_ENTRY_BASE_HEIGHT * 2 + 5 * 20);
  });

  it("buildTurnPrefixHeights：前缀和递增", () => {
    const heights = [10, 20, 30];
    const turns = [0, 1, 2].map(() => ({ turnId: "t" }) as never);
    const prefix = buildTurnPrefixHeights(turns, (_t, i) => heights[i]!);
    expect(prefix).toEqual([10, 30, 60]);
  });

  it("findTurnIndexAtScroll：二分定位", () => {
    const prefix = [10, 30, 60];
    expect(findTurnIndexAtScroll(prefix, 0)).toBe(0);
    expect(findTurnIndexAtScroll(prefix, 10)).toBe(1);
    expect(findTurnIndexAtScroll(prefix, 50)).toBe(2);
    expect(findTurnIndexAtScroll(prefix, 100)).toBe(2);
  });

  it("computeVirtualTurnWindow：空列表", () => {
    expect(computeVirtualTurnWindow([], 0, 100)).toEqual({
      startIndex: 0,
      endIndex: 0,
      paddingTop: 0,
      paddingBottom: 0
    });
  });

  it("computeVirtualTurnWindow：视口在顶部，paddingTop 为 0", () => {
    const prefix = [100, 200, 300];
    const window = computeVirtualTurnWindow(prefix, 0, 150);
    expect(window.startIndex).toBe(0);
    expect(window.paddingTop).toBe(0);
    // 总高 300，视口 150 + OVERSCAN 覆盖全部 3 个 turn
    expect(window.endIndex).toBe(3);
    expect(window.paddingBottom).toBe(0);
  });

  it("computeVirtualTurnWindow：滚动到中部，padding 两侧都有", () => {
    // 每 turn 约 40px，20 个 turn 总高 800px
    const prefix = Array.from({ length: 20 }, (_, i) => (i + 1) * 40);
    const window = computeVirtualTurnWindow(prefix, 350, 100);
    expect(window.startIndex).toBeGreaterThan(0);
    expect(window.paddingTop).toBeGreaterThan(0);
    expect(window.paddingBottom).toBeGreaterThan(0);
    // 可见窗口（含 OVERSCAN）小于总行数
    expect(window.endIndex - window.startIndex).toBeLessThan(prefix.length);
  });

  it("computeVirtualTurnWindow：滚动到底部，paddingBottom 为 0", () => {
    const prefix = [100, 200, 300, 400];
    const window = computeVirtualTurnWindow(prefix, 390, 100);
    expect(window.paddingBottom).toBe(0);
    expect(window.endIndex).toBe(prefix.length);
  });
});