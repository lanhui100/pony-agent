// D4 消息与输入（10 键 / 0 getters / 14 actions）
//
// 实现自 `2f11dc2f:src/stores/runtime.ts` **逐字搬迁**，语义零改动。键清单与签名真源见 `./types.ts`，
// 域归属与切分口径见 `.dev-team/runtime-split/fact-map.md`。与原实现的机械差异：import 路径按新层级
// 调整；action 方法补 `this: RuntimeStoreThis` 形参（Pinia options 式 `this` 口径）；类型
// `RuntimeStateW59` 改名 `RuntimeStoreState`（types.ts 的等价迁出物）。
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import { deriveGraphRunFromRunState, extractActiveTaskFocus } from "@/types/runtime";
import type {
  AttachmentAssetFilter,
  ChatMessage,
  GraphRun,
  MessageStateSnapshot,
  RetrievedContextState,
  SessionSnapshot,
  ToolActivity,
} from "@/types/runtime";
import { debugLog, measureHostRead } from "@/lib/runtime/utils";
import { RETRY_RESUME_PROMPT } from "@/lib/runtime/turn-retry";
import { toolStatusToMessageStatus } from "@/lib/runtime/trace";
import {
  buildToolMessageDetail,
  chatMessagesFromMessageState,
  reuseStableChatMessages,
} from "@/lib/runtime/messages";
import {
  attachmentDedupKey,
  bytesToDataUrl,
  importAttachment,
  MAX_TURN_IMAGES,
  mimeForSniffed,
  mimeFromExtension,
  readFileAsArrayBuffer,
  resolveAttachmentRoute,
  sniffImageKind,
  truncateTextByBytes,
  type PendingAttachment,
  type PendingAttachmentStatus,
  type ImportAttachmentOptions,
} from "@/lib/runtime/file-attachments";
import { filterAttachmentAssets } from "@/lib/runtime/attachments";
import {
  cloneRetrievedContext,
  createSnapshotFromRuntimeState,
  deriveRetrievedContextFromSnapshot,
} from "@/lib/runtime/sessions";
import type { MessagesActions, MessagesGetters, RuntimeStoreThis } from "./types";

/** D4 消息与输入（10 键 / 0 getters / 14 actions） getters（键清单见 `types.ts` 的 `MessagesGetters` 与事实图 §2）。 */
export const messagesGetters = {} as MessagesGetters;

/** D4 消息与输入（10 键 / 0 getters / 14 actions） actions（键清单见 `types.ts` 的 `MessagesActions` 与事实图 §3）。 */
export const messagesActions = {
  applyMessageStateSnapshot(this: RuntimeStoreThis, snapshot?: MessageStateSnapshot | null) {
    if (!snapshot || snapshot.sessionId !== this.sessionId) {
      return false;
    }

    this.messages = reuseStableChatMessages(this.messages, chatMessagesFromMessageState(snapshot));
    this.messageRevision = snapshot.revision;
    return true;
  },
  async resolveDerivedSessionRun(this: RuntimeStoreThis, options?: {
    sessionId?: string | null;
    runId?: string | null;
    preferRefresh?: boolean;
    snapshot?: SessionSnapshot | null;
    nodeId?: string | null;
  }): Promise<GraphRun | null> {
    const targetSessionId = options?.sessionId?.trim() || this.sessionId;
    if (!targetSessionId) {
      return null;
    }

    const deriveRun = (retrieved: RetrievedContextState | null | undefined) => {
      if (!retrieved) {
        return null;
      }

      return deriveGraphRunFromRunState(retrieved.runState, targetSessionId, Date.now(), {
        activeTaskFocus: extractActiveTaskFocus(retrieved.longTermMemory?.entries)
      });
    };

    if (!options?.preferRefresh) {
      const localRun = deriveRun(
        this.sessionId === targetSessionId ? this.retrievedContext : null
      );
      if (localRun) {
        return localRun;
      }
    }

    const refreshedRetrieved = await this.loadRetrievedContextState(targetSessionId, {
      runId: options?.runId ?? this.activeRunId,
      snapshot: options?.snapshot ?? createSnapshotFromRuntimeState(this, targetSessionId),
      nodeId: options?.nodeId ?? this.visibleNodeId
    });
    if (this.sessionId === targetSessionId) {
      this.retrievedContext = refreshedRetrieved;
    }
    return deriveRun(refreshedRetrieved);
  },
  applyTurnTokenStats(this: RuntimeStoreThis, turnId: string, inputTokens?: number | null, outputTokens?: number | null, persist = true) {
    const userMessage = this.messages.find((item) => item.turnId === turnId && item.role === "user");
    const assistantMessage = this.messages.find((item) => item.turnId === turnId && item.role === "assistant");

    if (userMessage && inputTokens != null) {
      userMessage.tokenCount = inputTokens;
      this.messageRevision = null;
    }

    if (assistantMessage && outputTokens != null) {
      assistantMessage.tokenCount = outputTokens;
      this.messageRevision = null;
    }

    if (persist) {
      this.persistHistory();
    }
  },
  ensureAssistantMessage(this: RuntimeStoreThis, turnId: string, modelName?: string | null) {
    const messageId = `assistant-${turnId}`;
    const existingMessage = this.messages.find((item) => item.id === messageId && item.role === "assistant");

    if (existingMessage) {
      if (modelName?.trim()) {
        existingMessage.modelName = modelName;
      }
      return existingMessage;
    }

    const assistantMessage: ChatMessage = {
      id: messageId,
      turnId,
      role: "assistant",
      reasoningContent: null,
      content: "",
      status: "pending",
      tokenCount: null,
      modelName: modelName?.trim() || undefined
    };

    this.messages.push(assistantMessage);
    this.messageRevision = null;
    // 延迟合并落盘：新建 assistant 消息时避免同步全量序列化阻塞主线程
    // （流式期间高频调用 ensureAssistantMessage，同步 persist 会放大卡顿）。
    this.scheduleDeferredPersist();
    return this.messages.find((item) => item.id === messageId && item.role === "assistant") ?? assistantMessage;
  },
  syncToolMessages(this: RuntimeStoreThis, turnId: string, toolActivities?: ToolActivity[] | null, persist = true) {
    if (!toolActivities) {
      return;
    }

    const activeTools = toolActivities.filter((tool) => tool.status !== "planned");
    const existingMessagesById = new Map<string, ChatMessage>();
    for (const message of this.messages) {
      if (message.role === "tool" && message.turnId === turnId) {
        existingMessagesById.set(message.id, message);
      }
    }
    const pendingMessages: ChatMessage[] = [];
    let didMutate = false;

    for (const tool of activeTools) {
      const messageId = `tool-${turnId}-${tool.id}`;
      const existingMessage = existingMessagesById.get(messageId);
      const nextContent = tool.resultText ?? "";
      const nextDetail = buildToolMessageDetail(tool);
      const nextStatus = toolStatusToMessageStatus(tool.status);
      const nextDurationSeconds = tool.durationSeconds ?? null;
      const nextCanonicalToolName = tool.canonicalToolName ?? null;
      const nextDisplayNameZh = tool.displayNameZh ?? null;
      const nextCallId = tool.id ?? null;
      const nextArgumentsText = tool.argumentsText ?? null;

      if (existingMessage) {
        if (existingMessage.content !== nextContent) {
          existingMessage.content = nextContent;
          didMutate = true;
        }
        if (existingMessage.status !== nextStatus) {
          existingMessage.status = nextStatus;
          didMutate = true;
        }
        if (existingMessage.toolName !== tool.name) {
          existingMessage.toolName = tool.name;
          didMutate = true;
        }
        if (existingMessage.canonicalToolName !== nextCanonicalToolName) {
          existingMessage.canonicalToolName = nextCanonicalToolName;
          didMutate = true;
        }
        if (existingMessage.displayNameZh !== nextDisplayNameZh) {
          existingMessage.displayNameZh = nextDisplayNameZh;
          didMutate = true;
        }
        if (existingMessage.detail !== nextDetail) {
          existingMessage.detail = nextDetail;
          didMutate = true;
        }
        if (existingMessage.durationSeconds !== nextDurationSeconds) {
          existingMessage.durationSeconds = nextDurationSeconds;
          didMutate = true;
        }
        if (existingMessage.callId !== nextCallId) {
          existingMessage.callId = nextCallId;
          didMutate = true;
        }
        if (existingMessage.argumentsText !== nextArgumentsText) {
          existingMessage.argumentsText = nextArgumentsText;
          didMutate = true;
        }
        continue;
      }

      pendingMessages.push({
        id: messageId,
        turnId,
        role: "tool",
        content: nextContent,
        status: nextStatus,
        toolName: tool.name,
        canonicalToolName: nextCanonicalToolName,
        displayNameZh: nextDisplayNameZh,
        detail: nextDetail,
        durationSeconds: nextDurationSeconds,
        callId: nextCallId,
        argumentsText: nextArgumentsText
      });
      didMutate = true;
    }

    if (pendingMessages.length > 0) {
      this.messages.push(...pendingMessages);
    }

    if (didMutate) {
      this.messageRevision = null;
    }

    if (persist && didMutate) {
      this.persistHistory();
    }
  },
  setDraftMessage(this: RuntimeStoreThis, message: string) {
    this.draftMessage = message;
  },
  setPendingAttachments(this: RuntimeStoreThis, attachments: PendingAttachment[]) {
    this.pendingAttachments = attachments;
  },
  clearPendingAttachments(this: RuntimeStoreThis) {
    this.pendingAttachments = [];
  },
  removePendingAttachment(this: RuntimeStoreThis, id: string) {
    this.pendingAttachments = this.pendingAttachments.filter((attachment) => attachment.id !== id);
  },
  /**
   * 处理选中的文件并加入待发送附件列表。逐文件：类型解析（扩展名优先/MIME 兜底）→
   * 大小上限 → 图片数量上限（600）→ 浏览器模式二进制文档拒绝 → 去重 → 图片魔数嗅探 →
   * 文本内容截断 → 宿主导入（Tauri）或内存引用（浏览器）。
   */
  async addPendingAttachments(this: RuntimeStoreThis, 
    files: File[],
    options?: ImportAttachmentOptions
  ): Promise<{ added: number; errors: string[] }> {
    const errors: string[] = [];
    let added = 0;

    for (const file of files) {
      const resolution = resolveAttachmentRoute(file.name, file.type);
      if (!resolution) {
        errors.push(`暂不支持该文件类型：${file.name}`);
        continue;
      }
      const { route, spec } = resolution;

      if (file.size > spec.maxBytes) {
        errors.push(`${file.name} 超过大小上限`);
        continue;
      }
      if (
        route === "image" &&
        this.pendingAttachments.filter(
          (attachment) => attachment.route === "image" && attachment.status === "ok"
        ).length >= MAX_TURN_IMAGES
      ) {
        errors.push(`最多添加 ${MAX_TURN_IMAGES} 张图片（${file.name} 未添加）`);
        continue;
      }
      if (route === "document" && !isTauriAvailable()) {
        errors.push(`预览模式暂不支持二进制文档：${file.name}`);
        continue;
      }
      // 去重只看成功条目（失败条目允许重试）；键含 lastModified 降低同名同尺寸误伤
      const dedupKey = attachmentDedupKey({
        name: file.name,
        sizeBytes: file.size,
        lastModified: file.lastModified
      });
      if (
        this.pendingAttachments.some(
          (attachment) => attachment.status === "ok" && attachmentDedupKey(attachment) === dedupKey
        )
      ) {
        errors.push(`已添加过该文件：${file.name}`);
        continue;
      }

      let status: PendingAttachmentStatus = "ok";
      let errorDetail: string | null = null;
      let dataUrl: string | null = null;
      let path: string | null = null;
      let relativePath: string | null = null;
      let content: string | null = null;
      let truncated = false;
      let attachmentMime = file.type || mimeFromExtension(file.name);

      try {
        const bytes = new Uint8Array(await readFileAsArrayBuffer(file));

        if (route === "image") {
          const sniffed = sniffImageKind(bytes);
          if (!sniffed) {
            errors.push(`无效图片文件：${file.name}`);
            continue;
          }
          // I-2：dataUrl mime 必须与真实内容一致（由嗅探结果派生），防"PNG 内容改名 .jpg"
          attachmentMime = mimeForSniffed(sniffed);
          dataUrl = bytesToDataUrl(bytes, attachmentMime);
          if (bytes.length > 0) {
            try {
              const importResult = await importAttachment(file.name, bytes, attachmentMime, options);
              path = importResult.path;
              relativePath = importResult.relativePath ?? null;
            } catch (importError) {
              console.warn("[runtime] 图片受控落盘失败，保持内存多模态引用:", importError);
            }
          }
        } else {
          if (route === "text") {
            const decoded = new TextDecoder().decode(bytes);
            const result = truncateTextByBytes(decoded, spec.injectMaxBytes);
            content = result.text;
            truncated = result.truncated;
          }
          if (bytes.length > 0) {
            const importResult = await importAttachment(file.name, bytes, attachmentMime, options);
            path = importResult.path;
            relativePath = importResult.relativePath ?? null;
          }
          // 空文本（0 字节）：跳过导入（宿主拒绝空字节），path/relativePath 为 null，
          // 内容为空——与浏览器模式行为一致（T-8）。
        }
      } catch (error) {
        status = "error";
        errorDetail = String(error);
      }

      // I-6：在途导入期间用户已提交 → 丢弃本次结果，避免 chip 遗留给下一轮
      if (this.isSubmitting) {
        break;
      }

      this.pendingAttachments = [
        ...this.pendingAttachments,
        {
          id: `pending-${Date.now()}-${this.pendingAttachments.length}-${added}`,
          name: file.name,
          sizeBytes: file.size,
          mimeType: attachmentMime,
          route,
          spec,
          path,
          relativePath,
          dataUrl,
          content,
          truncated,
          lastModified: file.lastModified,
          status,
          errorDetail
        }
      ];
      if (status === "ok") {
        added += 1;
      }
    }

    return { added, errors };
  },
  /**
   * P2-7：一键续跑——上游失败（可重试）的轮次直接继续。
   *
   * 语义：把「继续」写进草稿并提交，与用户手打「继续」**逐字一致**（不另造隐藏协议，
   * 也不重放整轮工具链）。不可重试的失败（配额/鉴权/未知）返回 `false`，由调用方
   * 展示不可重试原因，避免无效重试掩盖真实问题。
   */
  async retryLastTurn(this: RuntimeStoreThis): Promise<boolean> {
    if (this.isSubmitting) {
      return false;
    }
    const hint = this.failedTurnRetry;
    if (!hint || !hint.retryable) {
      return false;
    }
    this.draftMessage = RETRY_RESUME_PROMPT;
    await this.submitTurn();
    return true;
  },
  /**
   * 历史回填（shell 风格）：返回最近一条用户消息正文；无用户消息时返回 `null`。
   *
   * 只读，不改动 `draftMessage` —— 由调用方在「输入框为空 + ArrowUp」时取用，
   * 以守住「仅当输入框为空才触发」的守卫（见 `HomeWorkspace.handleComposerKeydown`）。
   */
  recallLastUserMessage(this: RuntimeStoreThis): string | null {
    for (let index = this.messages.length - 1; index >= 0; index -= 1) {
      const message = this.messages[index];
      if (message?.role === "user") {
        const content = message.content ?? "";
        if (content.trim().length > 0) {
          return content;
        }
      }
    }
    return null;
  },
  getAttachmentAssets(this: RuntimeStoreThis, filter?: AttachmentAssetFilter | null) {
    return filterAttachmentAssets(this.attachmentAssets, filter);
  },
  async loadRetrievedContextState(this: RuntimeStoreThis, 
    sessionId: string,
    options?: { runId?: string | null; snapshot?: SessionSnapshot | null; nodeId?: string | null }
  ) {
    const fallbackSnapshot = options?.snapshot ?? createSnapshotFromRuntimeState(this, sessionId);
    if (!isTauriAvailable()) {
      return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
    }

    try {
      const payload: Record<string, unknown> = {
        sessionId,
        runId: options?.runId ?? null,
        turnId: null
      };
      if (options?.nodeId) {
        payload.nodeId = options.nodeId;
      }
      const retrieved = await measureHostRead("load_retrieved_context", {
        sessionId,
        nodeId: options?.nodeId ?? null,
        runId: options?.runId ?? null
      }, () => safeInvoke<RetrievedContextState>("load_retrieved_context", payload));
      return cloneRetrievedContext(retrieved);
    } catch (error) {
      debugLog("retrieved-context:load:error", {
        sessionId,
        runId: options?.runId ?? null,
        error: String(error)
      });
      return deriveRetrievedContextFromSnapshot(fallbackSnapshot);
    }
  },
} as MessagesActions;
