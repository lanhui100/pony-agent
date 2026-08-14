import type { TraceTimelineEntry, TurnTraceRecord } from "@/types/runtime";

/**
 * trace 面板 turn 级虚拟化投影（PA-085）。
 *
 * 设计约束（3 路对抗审核采纳）：
 * - turn 级虚拟化：外层 turn 列表只渲染视口内窗口（OVERSCAN 缓冲），
 *   单个 turn 内部保持嵌套渲染（timeline 条目 + 详情行）。
 * - 高度为估算基准（turn header 固定 + 条目数 × 条目高度 + 展开详情估算），
 *   虚拟窗口用 OVERSCAN 缓冲吸收估算偏差。
 * - 纯函数、可测：输入 turns + 展开状态，输出扁平行高度与窗口。
 */

/** 虚拟化常量：基准高度（px） */
export const TURN_HEADER_BASE_HEIGHT = 40;
export const TIMELINE_ENTRY_BASE_HEIGHT = 34;
export const TIMELINE_ENTRY_EXPANDED_HEIGHT = 42;
export const DETAIL_ROW_HEIGHT = 20;
export const TURN_GAP_HEIGHT = 6;
export const OVERSCAN_TURNS = 3;

/** 一个 turn 的估算高度（px）；折叠 turn 只算 header（body 被 CSS grid 压缩为 0fr，不可见） */
export function estimateTurnHeight(
  timeline: TraceTimelineEntry[],
  isExpanded: boolean,
  expandedTimelineEntryCount: number
): number {
  let height = TURN_HEADER_BASE_HEIGHT + TURN_GAP_HEIGHT;
  if (!isExpanded) {
    return height;
  }
  for (const entry of timeline) {
    height += entry.state === "active" ? TIMELINE_ENTRY_EXPANDED_HEIGHT : TIMELINE_ENTRY_BASE_HEIGHT;
  }
  height += expandedTimelineEntryCount * DETAIL_ROW_HEIGHT;
  return height;
}

/** turn 前缀和高度（用于窗口定位） */
export function buildTurnPrefixHeights(turns: readonly TurnTraceRecord[], getHeight: (turn: TurnTraceRecord, index: number) => number): number[] {
  const prefix: number[] = new Array(turns.length);
  let acc = 0;
  for (let i = 0; i < turns.length; i++) {
    acc += getHeight(turns[i]!, i);
    prefix[i] = acc;
  }
  return prefix;
}

/** 二分查找：scrollTop 落在哪个 turn 索引 */
export function findTurnIndexAtScroll(prefixHeights: readonly number[], scrollTop: number): number {
  if (prefixHeights.length === 0) {
    return 0;
  }
  let low = 0;
  let high = prefixHeights.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (prefixHeights[mid]! <= scrollTop) {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  return low;
}

/** 虚拟窗口：scrollTop 与 viewportHeight 决定可见 turn 范围 */
export interface VirtualTurnWindow {
  startIndex: number;
  endIndex: number; // exclusive
  paddingTop: number;
  paddingBottom: number;
}

export function computeVirtualTurnWindow(
  prefixHeights: readonly number[],
  scrollTop: number,
  viewportHeight: number
): VirtualTurnWindow {
  const total = prefixHeights.length;
  if (total === 0) {
    return { startIndex: 0, endIndex: 0, paddingTop: 0, paddingBottom: 0 };
  }

  const start = Math.max(0, findTurnIndexAtScroll(prefixHeights, scrollTop) - OVERSCAN_TURNS);
  const viewportBottom = scrollTop + viewportHeight;
  let end = findTurnIndexAtScroll(prefixHeights, viewportBottom) + 1 + OVERSCAN_TURNS;
  end = Math.min(total, end);

  const paddingTop = start === 0 ? 0 : prefixHeights[start - 1]!;
  const paddingBottom = total - end > 0 ? prefixHeights[total - 1]! - prefixHeights[end - 1]! : 0;

  return { startIndex: start, endIndex: end, paddingTop, paddingBottom };
}