/**
 * PA-114: recognize every spelling of the builtin Ask tool across the frontend surface.
 *
 * The backend registers the first-class primitive `ask_user` (product name `Ask`) and keeps the
 * legacy spellings (`ask`, `builtin:ask`, `builtin:ask_user`) resolvable. Matching is
 * case-insensitive for the `Ask`/`ask` spellings so trace projections and merged tool calls never
 * fall back to the generic row for this tool.
 */
const ASK_TOOL_NAMES = new Set([
  "ask_user",
  "ask",
  "builtin:ask",
  "builtin:ask_user"
]);

export function isAskToolName(name: string): boolean {
  const trimmed = String(name ?? "").trim().toLowerCase();
  if (!trimmed) {
    return false;
  }
  return ASK_TOOL_NAMES.has(trimmed);
}
