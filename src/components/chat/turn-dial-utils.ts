import type { TurnBucket } from "@/components/chat/WorkspaceTurnItem.vue";

export interface TurnDialSummary {
  turnId: string;
  turnNumber: number;
  prompt: string;
  response: string;
}

export function clampText(text: string, limit: number): string {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (normalized.length <= limit) {
    return normalized;
  }
  return `${normalized.slice(0, limit).trimEnd()}…`;
}

export function formatTurnSummary(turn: TurnBucket, turnNumber: number): TurnDialSummary {
  const userContent = turn.user?.content ?? "";
  const assistantContent = turn.assistant?.content ?? "";

  return {
    turnId: turn.turnId,
    turnNumber,
    prompt: clampText(userContent, 80),
    response: clampText(assistantContent, 160)
  };
}
