// messages domain：消息克隆/比对/稳定性复用、消息状态归一化、历史消息水合、
// 以及展示文案构建（标题/图片摘要/工具详情）。依赖 trace（从 trace 派生消息字段）。
import type {
  ChatMessage,
  MessageStateDelta,
  MessageStateEntry,
  MessageStateSnapshot,
  ToolActivity,
  TurnHistoryMessage,
  TurnInputImage,
  TurnTraceRecord
} from "../../types/runtime";
import type { PendingAttachment } from "./file-attachments";
import {
  toolStatusToMessageStatus,
  traceErrorDetail,
  traceModelLabel,
  traceReasoningContent,
  traceToolActivities
} from "./trace";

export function cloneMessages(messages?: ChatMessage[] | null) {
  return (messages ?? []).map((message) => ({ ...message }));
}

export function chatMessageAttachmentsMatch(left?: ChatMessage["attachments"], right?: ChatMessage["attachments"]) {
  const leftAttachments = left ?? [];
  const rightAttachments = right ?? [];
  if (leftAttachments.length !== rightAttachments.length) {
    return false;
  }

  return leftAttachments.every((attachment, index) => {
    const other = rightAttachments[index];
    return (
      attachment.id === other?.id &&
      attachment.assetId === other?.assetId &&
      attachment.name === other?.name &&
      attachment.mimeType === other?.mimeType &&
      attachment.relativePath === other?.relativePath &&
      attachment.sizeBytes === other?.sizeBytes &&
      attachment.createdAtMs === other?.createdAtMs
    );
  });
}

export function chatMessagesMatch(left: ChatMessage, right: ChatMessage) {
  return (
    left.id === right.id &&
    left.turnId === right.turnId &&
    left.role === right.role &&
    left.content === right.content &&
    left.status === right.status &&
    left.reasoningContent === right.reasoningContent &&
    left.modelName === right.modelName &&
    left.tokenCount === right.tokenCount &&
    left.toolName === right.toolName &&
    left.canonicalToolName === right.canonicalToolName &&
    left.displayNameZh === right.displayNameZh &&
    left.detail === right.detail &&
    left.durationSeconds === right.durationSeconds &&
    left.errorDetail === right.errorDetail &&
    chatMessageAttachmentsMatch(left.attachments, right.attachments)
  );
}

export function reuseStableChatMessages(previous: ChatMessage[], next: ChatMessage[]) {
  const previousById = new Map(previous.map((message) => [message.id, message]));
  return next.map((message) => {
    const previousMessage = previousById.get(message.id);
    return previousMessage && chatMessagesMatch(previousMessage, message)
      ? previousMessage
      : message;
  });
}

export function normalizeMessageStateStatus(status?: MessageStateEntry["status"]): ChatMessage["status"] {
  return status === "pending" || status === "error" || status === "done" ? status : "done";
}

export function chatMessageFromMessageStateEntry(entry: MessageStateEntry): ChatMessage | null {
  if (entry.role !== "user" && entry.role !== "assistant" && entry.role !== "tool") {
    return null;
  }

  return {
    id: entry.messageId,
    turnId: entry.turnId,
    role: entry.role,
    content: entry.content,
    attachments: (entry.attachments ?? []).map((attachment) => ({ ...attachment })),
    reasoningContent: entry.reasoningContent ?? null,
    status: normalizeMessageStateStatus(entry.status),
    modelName: entry.modelName ?? null,
    tokenCount: entry.tokenCount ?? null,
    toolName: entry.toolName ?? null,
    canonicalToolName: entry.canonicalToolName ?? null,
    displayNameZh: entry.displayNameZh ?? null,
    detail: entry.detail ?? null,
    durationSeconds: entry.durationSeconds ?? null,
    errorDetail: entry.errorDetail ?? null
  };
}

export function chatMessagesFromMessageState(snapshot?: MessageStateSnapshot | null) {
  return (snapshot?.messages ?? [])
    .map((entry) => chatMessageFromMessageStateEntry(entry))
    .filter((message): message is ChatMessage => message != null);
}

export function previewMessageStateDeltaMessages(
  previousMessages: ChatMessage[],
  delta: MessageStateDelta | null | undefined,
  sessionId: string | null,
  messageRevision: string | null
) {
  if (!delta || delta.sessionId !== sessionId) {
    return null;
  }
  if (messageRevision !== null && delta.baseRevision !== messageRevision) {
    return null;
  }

  let nextMessages = [...previousMessages];
  for (const op of delta.ops) {
    if (op.kind === "truncateAfter") {
      if (!op.messageId) {
        nextMessages = [];
        continue;
      }
      const index = nextMessages.findIndex((message) => message.id === op.messageId);
      if (index < 0) {
        return null;
      }
      nextMessages = nextMessages.slice(0, index + 1);
      continue;
    }

    if (op.kind === "append") {
      nextMessages = [
        ...nextMessages,
        ...op.messages
          .map((entry) => chatMessageFromMessageStateEntry(entry))
          .filter((message): message is ChatMessage => message != null)
      ];
      continue;
    }

    if (op.kind === "replaceAll") {
      nextMessages = op.messages
        .map((entry) => chatMessageFromMessageStateEntry(entry))
        .filter((message): message is ChatMessage => message != null);
    }
  }

  return nextMessages;
}

// 查找 content 以"从某 index 开始的连续拼接"为前缀的 index（返回第一个满足项，
// 与 findIndex + slice(index).join("") 语义一致，但避免每次候选都重新拼接字符串）。
export function completedPrefixIndex(completedContents: string[], content: string): number {
  if (!content || completedContents.length === 0) {
    return -1;
  }
  const offsets: number[] = [];
  let total = 0;
  for (const part of completedContents) {
    offsets.push(total);
    total += part.length;
  }
  const joined = completedContents.join("");
  for (let index = 0; index < completedContents.length; index++) {
    const candidate = joined.slice(offsets[index]);
    if (candidate.length > 0 && content.startsWith(candidate)) {
      return index;
    }
  }
  return -1;
}

export function buildToolMessageDetail(tool: ToolActivity) {
  const blocks = [(tool.description ?? "").trim()];

  if (tool.argumentsText?.trim()) {
    blocks.push(`参数\n${tool.argumentsText.trim()}`);
  }

  if (tool.resultText?.trim()) {
    blocks.push(`结果\n${tool.resultText.trim()}`);
  }

  return blocks.filter(Boolean).join("\n");
}

export function buildTurnTitle(message: string) {
  const compact = message.replace(/\s+/g, " ").trim();
  if (!compact) {
    return "空白输入";
  }

  return compact.length > 44 ? `${compact.slice(0, 44)}…` : compact;
}

export function buildTurnTraceTitleFromMessages(messages: ChatMessage[], turnId: string) {
  const userMessage = messages.find((message) => message.turnId === turnId && message.role === "user");
  if (!userMessage?.content.trim()) {
    return "未命名轮次";
  }

  return buildTurnTitle(userMessage.content);
}

export function summarizeImageNames(images: TurnInputImage[]) {
  return images
    .map((image, index) => image.name?.trim() || `图片 ${index + 1}`)
    .slice(0, 2)
    .join("、");
}

export function buildDisplayedUserMessage(message: string, images: TurnInputImage[]) {
  if (!images.length) {
    return message;
  }

  const imageSummary = `[已附图片 ${images.length} 张${summarizeImageNames(images) ? `：${summarizeImageNames(images)}` : ""}]`;
  if (!message.trim()) {
    return imageSummary;
  }

  return `${message}\n\n${imageSummary}`;
}

export function buildProviderUserMessage(message: string, images: TurnInputImage[]) {
  if (message.trim()) {
    return message;
  }

  if (!images.length) {
    return message;
  }

  return "请基于附图回答。";
}

/** 附件消息块：文本注入块 + 二进制文档引用行。 */
export type AttachmentMessageBlocks = {
  text: string[];
  documents: string[];
};

export function buildAttachmentMessageBlocks(
  attachments: PendingAttachment[]
): AttachmentMessageBlocks {
  const text: string[] = [];
  const documents: string[] = [];
  for (const attachment of attachments) {
    if (attachment.route === "text" && attachment.content != null) {
      const label = attachment.truncated
        ? `${attachment.name}（内容过长，已截断）`
        : attachment.name;
      text.push(`[附件: ${label}]\n\n${attachment.content}`);
    } else if (attachment.route === "document" && attachment.path) {
      documents.push(`[附件: ${attachment.name}，路径: ${attachment.path}]`);
    }
  }
  return { text, documents };
}

/** 附件感知的 provider 用户消息：用户文本在前，随后文本内容注入块 → 文档引用行。
 * 附件块置于用户消息尾部，不影响 ADR-0007 稳定前缀（稳定前缀 = 请求前部 system/tools/history）。 */
export function buildProviderUserMessageWithAttachments(
  message: string,
  images: TurnInputImage[],
  blocks: AttachmentMessageBlocks
): string {
  const parts: string[] = [];
  if (message.trim()) {
    parts.push(message.trim());
  }
  if (blocks.text.length) {
    parts.push(blocks.text.join("\n\n"));
  }
  if (blocks.documents.length) {
    parts.push(blocks.documents.join("\n"));
  }
  const joined = parts.join("\n\n");
  if (joined.trim()) {
    return joined;
  }
  return images.length ? "请基于附图回答。" : message;
}

/** 附件感知的展示消息：用户文本 + 图片/文件摘要。 */
export function buildDisplayedUserMessageWithAttachments(
  message: string,
  images: TurnInputImage[],
  blocks: AttachmentMessageBlocks
): string {
  const summaryParts: string[] = [];
  if (images.length) {
    const imageSummary = `[已附图片 ${images.length} 张${summarizeImageNames(images) ? `：${summarizeImageNames(images)}` : ""}]`;
    summaryParts.push(imageSummary);
  }
  const fileCount = blocks.text.length + blocks.documents.length;
  if (fileCount) {
    summaryParts.push(`[已附文件 ${fileCount} 个]`);
  }
  const summary = summaryParts.join(" ");
  if (!message.trim()) {
    return summary || message;
  }
  return `${message}\n\n${summary}`.trim();
}

/** 把附件条目投影为消息 `attachments` 元数据（AttachmentMeta 合同）。 */
export function buildAttachmentMetas(
  attachments: PendingAttachment[],
  requestId: string
): Array<import("../../types/runtime").AttachmentMeta> {
  return attachments
    .filter((attachment) => attachment.route !== "image")
    .map((attachment, index) => ({
      id: `pending-${requestId}-file-${index + 1}`,
      name: attachment.name,
      mimeType: attachment.mimeType,
      relativePath: attachment.relativePath,
      sizeBytes: attachment.sizeBytes,
      createdAtMs: Date.now()
    }));
}

export function buildTurnHistory(messages: ChatMessage[]): TurnHistoryMessage[] {
  return messages
    .filter(
      (message) =>
        (message.role === "user" || message.role === "assistant") &&
        message.status !== "pending" &&
        message.content.trim().length > 0
    )
    .slice(-8)
    .map((message) => ({
      role: message.role === "user" ? "user" : "assistant",
      content: message.content,
      attachments: (message.attachments ?? [])
        .filter(
          (attachment) =>
            typeof attachment.relativePath === "string" && attachment.relativePath.trim().length > 0
        )
        .map((attachment) => ({ ...attachment })),
      turnId: message.turnId,
      status: message.status === "done" || message.status === "error" ? message.status : null,
      modelName: message.modelName,
      tokenCount: message.tokenCount,
      reasoningContent: message.reasoningContent
    }));
}

export function buildSessionTitleFromMessages(messages: ChatMessage[]) {
  const firstUserMessage = messages.find((message) => message.role === "user");
  return firstUserMessage ? buildTurnTitle(firstUserMessage.content) : "新对话";
}

export function createHistoryTurnId(index: number) {
  return `history-turn-${index + 1}`;
}

export function collectPersistedHistoryMessages(messages?: ChatMessage[] | null) {
  return (messages ?? []).filter(
    (message) =>
      (message.role === "user" || message.role === "assistant") &&
      message.status !== "pending" &&
      message.content.trim().length > 0
  );
}

export function buildToolMessagesFromTrace(trace: TurnTraceRecord | null | undefined, turnId: string): ChatMessage[] {
  const activities = traceToolActivities(trace);
  if (!activities.length) {
    return [];
  }

  return activities.map((tool) => ({
    id: `tool-${turnId}-${tool.id}`,
    turnId,
    role: "tool",
    content: tool.resultText ?? "",
    status: toolStatusToMessageStatus(tool.status),
    toolName: tool.name,
    canonicalToolName: tool.canonicalToolName ?? null,
    displayNameZh: tool.displayNameZh ?? null,
    detail: buildToolMessageDetail(tool),
    durationSeconds: tool.durationSeconds ?? null
  }));
}

export function hydrateMessagesFromHistory(
  history: TurnHistoryMessage[],
  persistedMessages?: ChatMessage[] | null,
  turnTraceHistory?: TurnTraceRecord[] | null
): ChatMessage[] {
  const messages: ChatMessage[] = [];
  const restoredHistoryMessages = collectPersistedHistoryMessages(persistedMessages);
  const toolMessagesByTurnId = new Map<string, ChatMessage[]>();
  const orderedTurnTraceHistory = [...(turnTraceHistory ?? [])].sort((left, right) => {
    const updatedAtDiff = (left.updatedAt ?? 0) - (right.updatedAt ?? 0);
    if (updatedAtDiff !== 0) {
      return updatedAtDiff;
    }

    return left.turnId.localeCompare(right.turnId);
  });
  let currentTurnId: string | null = null;
  let currentTrace: TurnTraceRecord | null = null;
  let traceIndex = 0;
  let turnIndex = 0;
  let restoredHistoryIndex = 0;

  for (const message of persistedMessages ?? []) {
    if (message.role !== "tool") {
      continue;
    }

    const turnMessages = toolMessagesByTurnId.get(message.turnId) ?? [];
    turnMessages.push({ ...message });
    toolMessagesByTurnId.set(message.turnId, turnMessages);
  }

  const appendToolMessagesForTurn = (turnId: string | null, trace?: TurnTraceRecord | null) => {
    if (!turnId) {
      return;
    }

    const toolMessages = toolMessagesByTurnId.get(turnId);
    if (toolMessages?.length) {
      messages.push(...toolMessages.map((message) => ({ ...message })));
      toolMessagesByTurnId.delete(turnId);
      return;
    }

    messages.push(...buildToolMessagesFromTrace(trace, turnId));
  };

  for (const item of history) {
    const restoredMessage = restoredHistoryMessages[restoredHistoryIndex];

    if (item.role === "user") {
      currentTrace = orderedTurnTraceHistory[traceIndex] ?? null;
      currentTurnId = item.turnId ?? restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
      restoredHistoryIndex += 1;
      messages.push({
        id: restoredMessage?.id ?? `history-user-${turnIndex}`,
        turnId: currentTurnId,
        role: "user",
        content: item.content,
        attachments: item.attachments ?? [],
        status: "done",
        tokenCount: item.tokenCount ?? restoredMessage?.tokenCount ?? null
      });
      continue;
    }

    currentTrace = currentTrace ?? orderedTurnTraceHistory[traceIndex] ?? null;
    if (!currentTurnId) {
      currentTurnId = item.turnId ?? restoredMessage?.turnId ?? currentTrace?.turnId ?? createHistoryTurnId(turnIndex);
      turnIndex += 1;
    }

    restoredHistoryIndex += 1;
    const restoredErrorDetail = restoredMessage?.errorDetail ?? null;
    const traceError = traceErrorDetail(currentTrace);
    const hasTraceError = currentTrace?.phase === "failed" || Boolean(traceError);
    const hasErrorState = item.status === "error" || hasTraceError || (!currentTrace && restoredMessage?.status === "error");
    const errorDetail = hasTraceError ? (traceError || restoredErrorDetail) : (currentTrace ? null : restoredErrorDetail);
    messages.push({
      id: restoredMessage?.id ?? `history-assistant-${turnIndex}`,
      turnId: currentTurnId,
      role: "assistant",
      content: item.content,
      attachments: [],
      status: hasErrorState ? "error" : "done",
      reasoningContent: item.reasoningContent ?? restoredMessage?.reasoningContent ?? traceReasoningContent(currentTrace),
      tokenCount: item.tokenCount ?? restoredMessage?.tokenCount ?? currentTrace?.outputTokens ?? null,
      modelName: item.modelName ?? restoredMessage?.modelName ?? traceModelLabel(currentTrace),
      errorDetail
    });
    appendToolMessagesForTurn(currentTurnId, currentTrace);
    currentTurnId = null;
    currentTrace = null;
    traceIndex += 1;
  }

  appendToolMessagesForTurn(currentTurnId, currentTrace);
  return messages;
}
