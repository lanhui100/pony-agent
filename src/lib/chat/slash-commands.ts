/**
 * 桌面端斜杠命令（契约 C-8，真值源 .dev-team/contract-matrix-slash-commands.md）。
 *
 * 设计约束：
 * - 本模块必须是**纯函数 + 纯元数据**，不依赖 pinia / Tauri / DOM，便于独立单测。
 * - 解析口径与 TUI 保持一致：仅取 `trim()` 后首个空白分隔 token，大小写不敏感，
 *   允许尾随空白/参数；未知或非命令一律返回 null（由调用方保持“按普通消息发送”的既有行为）。
 */

import type { GoalData } from "@/types/goal";

export type DesktopSlashCommand = "plan" | "tasks" | "compact" | "goal";

export interface DesktopSlashCommandSpec {
  name: DesktopSlashCommand;
  usage: string;
  description: string;
}

export const DESKTOP_SLASH_COMMANDS: ReadonlyArray<DesktopSlashCommandSpec> = [
  { name: "plan", usage: "/plan", description: "查看并展开当前会话计划" },
  { name: "tasks", usage: "/tasks", description: "查看 Team 任务看板" },
  { name: "compact", usage: "/compact", description: "压缩当前会话上下文" },
  { name: "goal", usage: "/goal", description: "设置或查看当前会话目标" }
];

/**
 * 手动压缩结果（对应 Rust `ManualCompactionOutcome`，`serde(rename_all = "camelCase")`）。
 * 字段全部来自后端真实计数，前端不得伪造。
 */
export interface ManualCompactionOutcome {
  compacted: boolean;
  reason: string;
  compressedCount: number;
  keptCount: number;
  beforeTokens: number;
  afterTokens: number;
}

/**
 * `/goal` 命令结果（对应 Rust `GoalCommandOutcome`，`serde(rename_all = "camelCase")`）。
 *
 * `message` 是后端渲染好的中文回执（语法/状态机真值源在 Rust，前端不得自行编造语义）；
 * `goal` 是变更后的目标快照，`clear` 与「无目标 show」为 `null`。
 */
export interface GoalCommandOutcome {
  action: string;
  message: string;
  goal: GoalData | null;
}

const KNOWN_COMMAND_NAMES: ReadonlySet<string> = new Set(
  DESKTOP_SLASH_COMMANDS.map((command) => command.name)
);

/** 仅精确匹配已知命令（首 token、大小写不敏感、允许尾随空白/参数）；未知或非命令返回 null。 */
export function parseSlashCommand(input: string): DesktopSlashCommand | null {
  if (typeof input !== "string") {
    return null;
  }

  const trimmed = input.trim();
  if (!trimmed.startsWith("/")) {
    return null;
  }

  const token = trimmed.slice(1).split(/\s+/)[0] ?? "";
  if (!token) {
    return null;
  }

  const name = token.toLowerCase();
  return KNOWN_COMMAND_NAMES.has(name) ? (name as DesktopSlashCommand) : null;
}

/**
 * `/compact` 结果的用户可读文案。
 * `compacted === false` 属“成功但未压缩”，必须如实说明原因，不得谎报已压缩。
 */
export function describeCompactionOutcome(
  outcome: ManualCompactionOutcome | null | undefined
): string {
  if (!outcome) {
    return "压缩已执行，但未返回结果";
  }

  if (!outcome.compacted) {
    if (outcome.reason === "history_too_short") {
      return "历史过短，无需压缩";
    }
    if (outcome.reason === "below_threshold") {
      return "未达压缩阈值，无需压缩";
    }
    return `未压缩（${outcome.reason.trim() || "原因未知"}）`;
  }

  const compressedCount = Number.isFinite(outcome.compressedCount) ? outcome.compressedCount : 0;
  const keptCount = Number.isFinite(outcome.keptCount) ? outcome.keptCount : 0;
  // 文案必须与可见状态一致：压缩的是**后端上下文**，转录区（前端消息列表）保留原始历史。
  return `已压缩上下文：压缩 ${compressedCount} 条，保留 ${keptCount} 条；后端上下文已压缩，转录区保留原始历史`;
}

/**
 * `/goal` 之后的参数文本（未命中 `/goal` ⇒ 空串）。
 *
 * 口径（契约 C-5）：`trim()` 后必须以 `/` 开头、首 token 大小写不敏感等于 `goal`；
 * 返回首个空白之后的原文（仅做首尾空白裁剪，正文不做任何改写）；`/goa`、`/goalx` 一律不命中。
 */
export function goalCommandArgs(input: string): string {
  if (typeof input !== "string") {
    return "";
  }

  const trimmed = input.trim();
  if (!trimmed.startsWith("/")) {
    return "";
  }

  const match = /^\/goal(?:\s+([\s\S]*))?$/i.exec(trimmed);
  if (!match) {
    return "";
  }

  return (match[1] ?? "").trim();
}

/**
 * `/goal` 结果的状态条文案（C-9 info 通道是**单行**截断渲染，多行回执必须折叠）。
 *
 * 只在渲染层折叠空白，不改写语义：`null`/空文案属「后端未返回结果」，必须如实说明，
 * 不得谎报命令成功。
 */
export function describeGoalOutcome(
  outcome: GoalCommandOutcome | null | undefined
): string {
  if (!outcome || typeof outcome.message !== "string") {
    return "目标命令已执行，但未返回结果";
  }

  const message = outcome.message.trim();
  if (!message) {
    return "目标命令已执行，但未返回结果";
  }

  return message.replace(/\s*\n+\s*/g, " · ");
}
