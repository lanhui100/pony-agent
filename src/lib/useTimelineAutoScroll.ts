import { nextTick, onBeforeUnmount, onMounted, ref, watch, type ComputedRef, type Ref } from "vue";

type ScrollAreaHandle = {
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  viewportEl: HTMLElement | null;
};

type MarkdownRenderPayload = {
  contentLength: number;
  streaming: boolean;
};

type UseTimelineAutoScrollOptions = {
  timelineScrollAreaRef: Ref<ScrollAreaHandle | null>;
  scrollAnchorRef: Ref<HTMLElement | null>;
  composerShellRef: Ref<HTMLElement | null>;
  workspaceContentColumnRef: Ref<HTMLElement | null>;
  isSubmitting: Ref<boolean>;
  latestMessageRole: ComputedRef<"user" | "assistant" | "tool" | null>;
  latestTurnSignature: ComputedRef<string>;
  latestVisibleTurnLayoutSignature: ComputedRef<string>;
  getLatestUserMessageElement: () => HTMLElement | null;
  getLatestAgentMessageElement: () => HTMLElement | null;
  showDebug: (event: string, patch?: Record<string, unknown>) => void;
  updateFloatingUiPositions: () => void;
};

type ScrollTargetMode = "anchor" | "latest-user" | "latest-agent";

const AUTO_SCROLL_THRESHOLD_PX = 260;
const PROGRAMMATIC_SCROLL_MAX_MS = 1600;
const USER_SCROLL_IDLE_MS = 3000;
const SCROLL_LERP_DURATION_MS = 200;
const SCROLL_RAF_STUCK_TIMEOUT_MS = 500;

export function useTimelineAutoScroll(options: UseTimelineAutoScrollOptions) {
  const {
    timelineScrollAreaRef,
    scrollAnchorRef,
    composerShellRef,
    workspaceContentColumnRef,
    isSubmitting,
    latestMessageRole,
    latestTurnSignature,
    latestVisibleTurnLayoutSignature,
    getLatestUserMessageElement,
    getLatestAgentMessageElement,
    showDebug,
    updateFloatingUiPositions
  } = options;

  const showScrollToBottom = ref(false);
  const unreadCount = ref(0);
  const scrollQueued = ref(false);
  const streamAutoFollowEnabled = ref(true);
  const scrollAfterPaintFrameId = ref<number | null>(null);

  let scheduledScrollRequestId = 0;
  let contentResizeObserver: ResizeObserver | null = null;
  let autoFollowIdleTimer: ReturnType<typeof setTimeout> | null = null;
  let programmaticScrollActive = false;
  let programmaticScrollTargetMode: ScrollTargetMode = "anchor";
  let programmaticScrollTargetTop = 0;
  let programmaticScrollUntilMs = 0;
  let userScrollOverrideVersion = 0;
  let lastUserPausedSignature = "";
  let lastUserScrollAtMs = 0;
  let scrollLerpRafId: number | null = null;
  let scrollQueueWatchdogTimer: ReturnType<typeof setTimeout> | null = null;
  let layoutDirtyWhileQueued = false;
  let contentDirtyWhileAnimating = false;
  let isDestroyed = false;

  function getViewport() {
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    return viewport instanceof HTMLElement ? viewport : null;
  }

  function readViewportMetrics(viewport: HTMLElement | null) {
    if (!viewport) {
      return null;
    }

    const scrollTop = viewport.scrollTop;
    const scrollHeight = viewport.scrollHeight;
    const clientHeight = viewport.clientHeight;
    if (![scrollTop, scrollHeight, clientHeight].every((value) => Number.isFinite(value))) {
      return null;
    }

    return {
      scrollTop,
      scrollHeight,
      clientHeight,
      distanceToBottom: scrollHeight - scrollTop - clientHeight
    };
  }

  function clearScrollQueueWatchdog() {
    if (scrollQueueWatchdogTimer != null) {
      clearTimeout(scrollQueueWatchdogTimer);
      scrollQueueWatchdogTimer = null;
    }
  }

  function armScrollQueueWatchdog(requestId: number, behavior: ScrollBehavior, expectedOverrideVersion: number) {
    clearScrollQueueWatchdog();
    scrollQueueWatchdogTimer = setTimeout(() => {
      if (scrollAfterPaintFrameId.value == null) {
        return;
      }

      emit("queue-scroll-request:raf-stuck", {
        requestId,
        behavior,
        expectedOverrideVersion,
        rafId: scrollAfterPaintFrameId.value,
        scheduledScrollRequestId
      });
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
      scrollAfterPaintFrameId.value = null;
      scrollQueued.value = false;
      if (!isDestroyed && streamAutoFollowEnabled.value) {
        queueScrollToLatestTurn(behavior, userScrollOverrideVersion);
      }
    }, SCROLL_RAF_STUCK_TIMEOUT_MS);
  }

  function collectMetrics() {
    const viewport = getViewport();
    const metrics = readViewportMetrics(viewport);
    const anchor = scrollAnchorRef.value;
    const composerShell = composerShellRef.value;
    const latestAgentMessage = getLatestAgentMessageElement();
    const anchorTop = anchor?.getBoundingClientRect().top ?? null;
    const anchorBottom = anchor?.getBoundingClientRect().bottom ?? null;
    const composerTop = composerShell?.getBoundingClientRect().top ?? null;
    const latestAgentTop = latestAgentMessage?.getBoundingClientRect().top ?? null;
    return {
      streamAutoFollowEnabled: streamAutoFollowEnabled.value,
      scrollQueued: scrollQueued.value,
      isSubmitting: isSubmitting.value,
      programmaticScrollActive,
      programmaticScrollTargetMode,
      programmaticScrollTargetTop,
      programmaticScrollUntilMs,
      userScrollOverrideVersion,
      lastUserPausedSignature,
      viewportScrollTop: metrics?.scrollTop ?? null,
      viewportScrollHeight: metrics?.scrollHeight ?? null,
      viewportClientHeight: metrics?.clientHeight ?? null,
      distanceToBottom: metrics?.distanceToBottom ?? null,
      anchorTop,
      anchorBottom,
      composerTop,
      anchorToComposerDistance: anchorBottom != null && composerTop != null ? composerTop - anchorBottom : null,
      latestUserTop: getLatestUserMessageElement()?.getBoundingClientRect().top ?? null,
      latestAgentTop,
      targetDelta: metrics != null ? programmaticScrollTargetTop - metrics.scrollTop : null,
      latestVisibleTurnLayoutSignature: latestVisibleTurnLayoutSignature.value,
      viewportResolved: viewport instanceof HTMLElement,
      viewportMetricsValid: metrics != null
    };
  }

  function emit(event: string, patch: Record<string, unknown> = {}) {
    showDebug(event, {
      ...collectMetrics(),
      ...patch
    });
  }

  function isTimelineNearBottom() {
    const metrics = readViewportMetrics(getViewport());
    if (!metrics) {
      return true;
    }

    return metrics.distanceToBottom <= AUTO_SCROLL_THRESHOLD_PX;
  }

  function getAnchorTargetTop(): number {
    const viewport = getViewport();
    const metrics = readViewportMetrics(viewport);
    const anchor = scrollAnchorRef.value;
    const composerShell = composerShellRef.value;
    if (!viewport || !metrics) return 0;
    if (!anchor) return metrics.scrollHeight;
    const anchorRect = anchor.getBoundingClientRect();
    const viewportRect = viewport.getBoundingClientRect();
    const composerTop = composerShell?.getBoundingClientRect().top ?? viewportRect.bottom;
    if (anchorRect.top === viewportRect.top && anchorRect.bottom === viewportRect.bottom) {
      return metrics.scrollHeight;
    }
    const targetTop = metrics.scrollTop + (anchorRect.bottom - composerTop);
    if (!Number.isFinite(targetTop)) {
      emit("anchor-target:invalid", {
        anchorBottom: anchorRect.bottom,
        composerTop,
        fallbackTargetTop: metrics.scrollHeight
      });
      return metrics.scrollHeight;
    }
    return Math.max(0, targetTop);
  }

  function getLatestUserTargetTop(): number {
    const viewport = getViewport();
    const metrics = readViewportMetrics(viewport);
    const latestUserMessage = getLatestUserMessageElement();
    if (!viewport || !metrics || !latestUserMessage) {
      return getAnchorTargetTop();
    }

    const userRect = latestUserMessage.getBoundingClientRect();
    const viewportRect = viewport.getBoundingClientRect();
    const targetTop = metrics.scrollTop + (userRect.top - viewportRect.top);
    if (!Number.isFinite(targetTop)) {
      emit("latest-user-target:invalid", {
        userTop: userRect.top,
        viewportTop: viewportRect.top,
        fallbackTargetTop: getAnchorTargetTop()
      });
      return getAnchorTargetTop();
    }

    return Math.max(0, Math.min(targetTop, Math.max(metrics.scrollHeight - metrics.clientHeight, 0)));
  }

  function getLatestAgentTargetTop(): number {
    const viewport = getViewport();
    const metrics = readViewportMetrics(viewport);
    const latestAgentMessage = getLatestAgentMessageElement();
    if (!viewport || !metrics || !latestAgentMessage) {
      return getAnchorTargetTop();
    }

    const agentRect = latestAgentMessage.getBoundingClientRect();
    const viewportRect = viewport.getBoundingClientRect();
    const goldenOffset = Math.min(Math.max(metrics.clientHeight * 0.18, 72), 160);
    const targetTop = metrics.scrollTop + (agentRect.top - viewportRect.top) - goldenOffset;
    if (!Number.isFinite(targetTop)) {
      emit("latest-agent-target:invalid", {
        agentTop: agentRect.top,
        viewportTop: viewportRect.top,
        fallbackTargetTop: getAnchorTargetTop()
      });
      return getAnchorTargetTop();
    }

    return Math.max(0, Math.min(targetTop, Math.max(metrics.scrollHeight - metrics.clientHeight, 0)));
  }

  function getTargetTop(mode: ScrollTargetMode) {
    if (mode === "latest-user") {
      return getLatestUserTargetTop();
    }
    if (mode === "latest-agent") {
      return getLatestAgentTargetTop();
    }
    return getAnchorTargetTop();
  }

  function isLatestUserMessageBelowViewport() {
    const viewport = getViewport();
    const latestUserMessage = getLatestUserMessageElement();
    if (!viewport || !latestUserMessage) {
      return false;
    }

    const viewportRect = viewport.getBoundingClientRect();
    const userRect = latestUserMessage.getBoundingClientRect();
    return userRect.top > viewportRect.bottom || userRect.bottom > viewportRect.bottom;
  }

  function shouldShowScrollToLatestButton() {
    return isLatestUserMessageBelowViewport();
  }

  function cancelScrollLerp() {
    if (scrollLerpRafId != null) {
      window.cancelAnimationFrame(scrollLerpRafId);
      scrollLerpRafId = null;
    }
    clearScrollQueueWatchdog();
    scrollQueued.value = false;
  }

  function clearAutoFollowIdleTimer() {
    cancelScrollLerp();
    if (autoFollowIdleTimer != null) {
      clearTimeout(autoFollowIdleTimer);
      autoFollowIdleTimer = null;
      emit("idle-resume:cleared");
    }
  }

  function scheduleAnchorCompensationScroll() {
    window.requestAnimationFrame(() => {
      if (!streamAutoFollowEnabled.value || isDestroyed) {
        return;
      }

      const viewport = getViewport();
      if (!viewport) {
        emit("anchor-compensation-scroll:missing-viewport");
        return;
      }

      const anchorTarget = getAnchorTargetTop();
      if (!Number.isFinite(anchorTarget)) {
        emit("anchor-compensation-scroll:invalid-target", { targetTop: anchorTarget });
        return;
      }
      emit("anchor-compensation-scroll", { targetTop: anchorTarget });
      programmaticScrollActive = true;
      programmaticScrollTargetMode = "anchor";
      programmaticScrollTargetTop = anchorTarget;
      programmaticScrollUntilMs = Date.now() + PROGRAMMATIC_SCROLL_MAX_MS;
      viewport.scrollTo({ top: anchorTarget, behavior: "auto" });
      finalizeProgrammaticScrollCycle();
    });
  }

  function flushDeferredFollowIfNeeded(behavior: ScrollBehavior = "auto") {
    if (!streamAutoFollowEnabled.value) {
      layoutDirtyWhileQueued = false;
      contentDirtyWhileAnimating = false;
      return;
    }

    if (!layoutDirtyWhileQueued && !contentDirtyWhileAnimating) {
      return;
    }

    layoutDirtyWhileQueued = false;
    contentDirtyWhileAnimating = false;
    emit("flush-deferred-follow", { behavior });
    if (behavior === "auto") {
      scheduleAnchorCompensationScroll();
      return;
    }
    queueScrollToLatestTurn(behavior, userScrollOverrideVersion);
  }

  function finalizeProgrammaticScrollCycle() {
    programmaticScrollActive = false;
    programmaticScrollTargetMode = "anchor";
    programmaticScrollTargetTop = 0;
    programmaticScrollUntilMs = 0;
    showScrollToBottom.value = false;
    unreadCount.value = 0;
    streamAutoFollowEnabled.value = true;
    lastUserPausedSignature = "";
    clearAutoFollowIdleTimer();
    flushDeferredFollowIfNeeded("auto");
  }

  function cancelPendingTimelineScroll(reason = "unspecified") {
    scheduledScrollRequestId += 1;
    scrollQueued.value = false;
    programmaticScrollActive = false;
    programmaticScrollTargetMode = "anchor";
    programmaticScrollTargetTop = 0;
    programmaticScrollUntilMs = 0;
    cancelScrollLerp();
    if (scrollAfterPaintFrameId.value != null) {
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
      scrollAfterPaintFrameId.value = null;
    }
    clearScrollQueueWatchdog();
    emit("cancel-pending-scroll", { reason });
  }

  function pauseTimelineAutoFollow() {
    userScrollOverrideVersion += 1;
    streamAutoFollowEnabled.value = false;
    lastUserPausedSignature = latestTurnSignature.value;
    cancelPendingTimelineScroll("pause-auto-follow");
    emit("pause-auto-follow");
    lastUserScrollAtMs = Date.now();
    if (isSubmitting.value) {
      scheduleAutoFollowIdleResume();
    } else {
      clearAutoFollowIdleTimer();
    }
  }

  function resumeTimelineAutoFollow(behavior: ScrollBehavior = "auto", targetMode: ScrollTargetMode = "anchor") {
    streamAutoFollowEnabled.value = true;
    lastUserPausedSignature = "";
    clearAutoFollowIdleTimer();
    emit("resume-auto-follow", { behavior, targetMode });
    queueScrollToLatestTurn(behavior, userScrollOverrideVersion, targetMode);
  }

  function queueScrollToLatestTurn(
    behavior: ScrollBehavior = "smooth",
    expectedOverrideVersion = userScrollOverrideVersion,
    targetMode: ScrollTargetMode = "anchor"
  ) {
    scheduledScrollRequestId += 1;
    const requestId = scheduledScrollRequestId;
    scrollQueued.value = true;
    emit("queue-scroll-request", { requestId, behavior, expectedOverrideVersion, targetMode });
    void nextTick().then(() => {
      if (
        requestId !== scheduledScrollRequestId ||
        expectedOverrideVersion !== userScrollOverrideVersion ||
        !streamAutoFollowEnabled.value
      ) {
        if (requestId === scheduledScrollRequestId) {
          scrollQueued.value = false;
        }
        emit("queue-scroll-request:cancelled-before-raf", { requestId, behavior, expectedOverrideVersion, targetMode });
        return;
      }
      if (scrollAfterPaintFrameId.value != null) {
        emit("queue-scroll-request:replace-pending-raf", {
          requestId,
          behavior,
          expectedOverrideVersion,
          targetMode,
          previousRafId: scrollAfterPaintFrameId.value
        });
        window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
        scrollAfterPaintFrameId.value = null;
        clearScrollQueueWatchdog();
      }

      if (isDestroyed) {
        if (requestId === scheduledScrollRequestId) {
          scrollQueued.value = false;
        }
        return;
      }

      scrollAfterPaintFrameId.value = window.requestAnimationFrame(() => {
        clearScrollQueueWatchdog();
        if (
          requestId !== scheduledScrollRequestId ||
          expectedOverrideVersion !== userScrollOverrideVersion ||
          !streamAutoFollowEnabled.value
        ) {
          if (requestId === scheduledScrollRequestId) {
            scrollQueued.value = false;
            scrollAfterPaintFrameId.value = null;
          }
            emit("queue-scroll-request:cancelled-in-raf", { requestId, behavior, expectedOverrideVersion, targetMode });
            return;
          }

          scrollAfterPaintFrameId.value = null;
          scrollQueued.value = false;
          const scrollArea = timelineScrollAreaRef.value;
          const viewport = getViewport();
          if (viewport) {
          const targetTop = getTargetTop(targetMode);
          if (!Number.isFinite(targetTop)) {
            emit("scroll-to-latest-turn:invalid-target", { requestId, behavior, targetMode, targetTop });
            finalizeProgrammaticScrollCycle();
            return;
          }
          programmaticScrollActive = true;
          programmaticScrollTargetMode = targetMode;
          programmaticScrollTargetTop = targetTop;
          programmaticScrollUntilMs = Date.now() + PROGRAMMATIC_SCROLL_MAX_MS;
          emit("scroll-to-latest-turn", {
            requestId,
            behavior,
            targetMode,
            targetTop,
            expiresAt: programmaticScrollUntilMs
          });
          if (behavior === "auto") {
            viewport.scrollTo({ top: targetTop, behavior: "auto" });
            finalizeProgrammaticScrollCycle();
            return;
          }

          cancelScrollLerp();
          const startTop = viewport.scrollTop;
          const distance = targetTop - startTop;
          if (!Number.isFinite(startTop) || !Number.isFinite(distance)) {
            emit("scroll-to-latest-turn:invalid-distance", { requestId, behavior, targetMode, startTop, targetTop });
            finalizeProgrammaticScrollCycle();
            return;
          }
          if (Math.abs(distance) < 5) {
            finalizeProgrammaticScrollCycle();
            return;
          }

          let lerpStartMs = performance.now();
          let lerpStartTop = startTop;
          let lerpDistance = distance;
          scrollLerpRafId = window.requestAnimationFrame(function lerp(now: number) {
            if (!streamAutoFollowEnabled.value || expectedOverrideVersion !== userScrollOverrideVersion) {
              scrollLerpRafId = null;
              finalizeProgrammaticScrollCycle();
              return;
            }
            const elapsed = now - lerpStartMs;
            const t = Math.min(elapsed / SCROLL_LERP_DURATION_MS, 1);
            const eased = 1 - Math.pow(1 - t, 3);
            viewport.scrollTop = lerpStartTop + lerpDistance * eased;
            if (t < 1) {
              scrollLerpRafId = window.requestAnimationFrame(lerp);
              return;
            }

            const finalTarget = getTargetTop(targetMode);
            if (finalTarget > viewport.scrollTop + 5) {
              lerpStartMs = performance.now();
              lerpStartTop = viewport.scrollTop;
              lerpDistance = finalTarget - lerpStartTop;
              scrollLerpRafId = window.requestAnimationFrame(lerp);
              return;
            }

            scrollLerpRafId = null;
            finalizeProgrammaticScrollCycle();
          });
        } else if (scrollArea && typeof scrollArea.scrollToBottom === "function") {
          emit("scroll-to-latest-turn:fallback", { requestId, behavior, targetMode });
          scrollArea.scrollToBottom(behavior);
        }
      });
      armScrollQueueWatchdog(requestId, behavior, expectedOverrideVersion);
    });
  }

  function handleTimelineViewportScroll() {
    updateFloatingUiPositions();
    const viewport = getViewport();
    const metrics = readViewportMetrics(viewport);
    if (programmaticScrollActive && viewport) {
      const nearBottom = isTimelineNearBottom();
      const reachedProgrammaticTarget = programmaticScrollTargetMode === "anchor"
        ? nearBottom || (metrics != null && metrics.scrollTop >= Math.max(programmaticScrollTargetTop - metrics.clientHeight - AUTO_SCROLL_THRESHOLD_PX, 0))
        : metrics != null && Math.abs(programmaticScrollTargetTop - metrics.scrollTop) <= 12;
      const programmaticScrollExpired = Date.now() >= programmaticScrollUntilMs;
      if (!reachedProgrammaticTarget && !programmaticScrollExpired) {
        emit("viewport-scroll:programmatic-progress", {
          reachedProgrammaticTarget,
          programmaticScrollExpired,
          targetMode: programmaticScrollTargetMode,
          targetDelta: metrics != null ? programmaticScrollTargetTop - metrics.scrollTop : null
        });
        return;
      }

      programmaticScrollActive = false;
      programmaticScrollTargetTop = 0;
      programmaticScrollUntilMs = 0;
      if (reachedProgrammaticTarget) {
        streamAutoFollowEnabled.value = true;
        lastUserPausedSignature = "";
        clearAutoFollowIdleTimer();
        emit("viewport-scroll:programmatic-complete", {
          targetMode: programmaticScrollTargetMode,
          targetDelta: metrics != null ? programmaticScrollTargetTop - metrics.scrollTop : null
        });
        return;
      }

      emit("viewport-scroll:programmatic-expired", {
        targetMode: programmaticScrollTargetMode,
        targetDelta: metrics != null ? programmaticScrollTargetTop - metrics.scrollTop : null
      });
    }

    const distanceFromBottom = metrics?.distanceToBottom ?? 0;

    if (isTimelineNearBottom()) {
      streamAutoFollowEnabled.value = true;
      showScrollToBottom.value = false;
      unreadCount.value = 0;
      lastUserPausedSignature = "";
      lastUserScrollAtMs = 0;
      clearAutoFollowIdleTimer();
      emit("viewport-scroll:near-bottom");
      return;
    }

    showScrollToBottom.value = shouldShowScrollToLatestButton();

    lastUserScrollAtMs = Date.now();
    emit("viewport-scroll:user-away-from-bottom", {
      distanceFromBottom,
      latestUserBelowViewport: showScrollToBottom.value,
      showScrollToBottom: showScrollToBottom.value
    });
    pauseTimelineAutoFollow();
  }

  function handleTimelineUserScrollIntent() {
    emit("user-scroll-intent:wheel-or-touchstart");
    lastUserScrollAtMs = Date.now();
    updateFloatingUiPositions();
    if (isTimelineNearBottom()) {
      cancelPendingTimelineScroll("user-scroll-intent");
    } else {
      pauseTimelineAutoFollow();
    }
  }

  function handleTimelinePointerIntent() {
    emit("user-scroll-intent:pointerdown");
    cancelPendingTimelineScroll("pointer-intent");
  }

  function handleTimelineKeyboardIntent(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    const isEditableTarget =
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLInputElement ||
      target?.isContentEditable === true;
    if (isEditableTarget) {
      return;
    }

    if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)) {
      return;
    }

    emit("user-scroll-intent:keyboard", { key: event.key });
    cancelPendingTimelineScroll("keyboard-intent");
  }

  function handleScrollToBottom() {
    showScrollToBottom.value = false;
    unreadCount.value = 0;
    resumeTimelineAutoFollow("smooth", "latest-agent");
  }

  function handleMarkdownRenderComplete(payload: MarkdownRenderPayload) {
    if (!payload.streaming && !isSubmitting.value) {
      emit("markdown-render-complete:skip", {
        reason: "not-streaming-and-not-submitting",
        contentLength: payload.contentLength
      });
      return;
    }

    if (!streamAutoFollowEnabled.value) {
      return;
    }

    if (scrollQueued.value) {
      layoutDirtyWhileQueued = true;
      emit("markdown-render-complete:defer", { reason: "scroll-queued" });
      return;
    }

    if (programmaticScrollActive) {
      contentDirtyWhileAnimating = true;
      emit("markdown-render-complete:defer", { reason: "programmatic-scroll-active" });
      return;
    }

    emit("markdown-render-complete:queue-follow");
    queueScrollToLatestTurn("auto", userScrollOverrideVersion, "anchor");
  }

  function scheduleAutoFollowIdleResume() {
    clearAutoFollowIdleTimer();
    emit("idle-resume:scheduled", { delayMs: USER_SCROLL_IDLE_MS });
    autoFollowIdleTimer = setTimeout(() => {
      autoFollowIdleTimer = null;
      const idleMs = Date.now() - (lastUserScrollAtMs || Date.now());
      if (
        idleMs >= USER_SCROLL_IDLE_MS &&
        isSubmitting.value &&
        !streamAutoFollowEnabled.value &&
        latestTurnSignature.value &&
        latestTurnSignature.value !== lastUserPausedSignature
      ) {
        emit("idle-resume:resuming-after-idle");
        resumeTimelineAutoFollow("auto");
      }
    }, USER_SCROLL_IDLE_MS);
  }

  function setupContentResizeObserver() {
    teardownContentResizeObserver();
    const contentColumn = workspaceContentColumnRef.value;
    if (!contentColumn || typeof ResizeObserver === "undefined") return;

    contentResizeObserver = new ResizeObserver(() => {
      if (!streamAutoFollowEnabled.value) {
        emit("resize-observer:skip", { reason: "follow-disabled" });
        return;
      }
      if (scrollQueued.value) {
        layoutDirtyWhileQueued = true;
        emit("resize-observer:defer", { reason: "scroll-queued" });
        return;
      }
      if (programmaticScrollActive) {
        contentDirtyWhileAnimating = true;
        emit("resize-observer:defer", { reason: "programmatic-scroll-active" });
        return;
      }
      emit("resize-observer:queue-scroll", { behavior: "auto" });
      scheduleAnchorCompensationScroll();
    });
    contentResizeObserver.observe(contentColumn);
  }

  function teardownContentResizeObserver() {
    if (contentResizeObserver) {
      contentResizeObserver.disconnect();
      contentResizeObserver = null;
    }
  }

  function setAutoFollowEnabled() {
    streamAutoFollowEnabled.value = true;
  }

  function handleLatestTurnSignatureChange(signature: string, previousSignature: string | undefined) {
    if (signature === previousSignature) {
      return;
    }

    emit("latest-turn-signature:changed", { signature, previousSignature });
    void nextTick().then(() => {
      if (!streamAutoFollowEnabled.value) {
        if (
          lastUserScrollAtMs > 0 &&
          isSubmitting.value &&
          autoFollowIdleTimer == null &&
          signature !== lastUserPausedSignature &&
          Date.now() - lastUserScrollAtMs >= USER_SCROLL_IDLE_MS
        ) {
          emit("latest-turn-signature:resume-after-idle-content");
          resumeTimelineAutoFollow("auto", "anchor");
          return;
        }
        emit("latest-turn-signature:follow-disabled", { signature, previousSignature });
        return;
      }

      emit("latest-turn-signature:queue-follow-scroll", { signature, previousSignature });
      queueScrollToLatestTurn("smooth", userScrollOverrideVersion, "anchor");
    });
  }

  function handleSubmittingChange(submitting: boolean, wasSubmitting: boolean) {
    if (submitting) {
      streamAutoFollowEnabled.value = true;
    }
    if (!submitting) {
      clearAutoFollowIdleTimer();
      cancelScrollLerp();
    }
    emit("is-submitting:changed", { submitting });
    if (wasSubmitting && !submitting && streamAutoFollowEnabled.value && latestTurnSignature.value) {
      queueScrollToLatestTurn("smooth", userScrollOverrideVersion, "anchor");
    }
  }

  function handleMessageCountChange(newLen: number, oldLen: number | undefined) {
    if (newLen === oldLen || oldLen == null) return;
    if (newLen < oldLen) {
      unreadCount.value = 0;
      return;
    }
    if (!streamAutoFollowEnabled.value) {
      unreadCount.value += newLen - oldLen;
      showScrollToBottom.value = shouldShowScrollToLatestButton();
      return;
    }

    if (latestMessageRole.value === "user") {
      emit("message-count:user-message-follow", { newLen, oldLen });
      queueScrollToLatestTurn("smooth", userScrollOverrideVersion, "anchor");
    }
  }

  function handleVisibleTurnLayoutChange(signature: string, previousSignature: string | undefined) {
    if (!signature || signature === previousSignature) {
      return;
    }
    emit("visible-turn-layout:changed", { signature, previousSignature });
    if (!streamAutoFollowEnabled.value) {
      emit("visible-turn-layout:skip", { reason: "follow-disabled", signature, previousSignature });
      return;
    }
    if (scrollQueued.value) {
      layoutDirtyWhileQueued = true;
      emit("visible-turn-layout:defer", { reason: "scroll-queued", signature, previousSignature });
      return;
    }
    if (programmaticScrollActive) {
      contentDirtyWhileAnimating = true;
      emit("visible-turn-layout:defer", { reason: "programmatic-scroll-active", signature, previousSignature });
      return;
    }

    emit("visible-turn-layout:queue-follow", { signature, previousSignature });
    scheduleAnchorCompensationScroll();
  }

  watch(
    () => timelineScrollAreaRef.value?.viewportEl ?? null,
    (viewport, previousViewport) => {
      const previousElement = previousViewport instanceof HTMLElement ? previousViewport : null;
      const viewportElement = viewport instanceof HTMLElement ? viewport : null;
      if (previousElement) {
        previousElement.removeEventListener("scroll", handleTimelineViewportScroll);
        previousElement.removeEventListener("wheel", handleTimelineUserScrollIntent);
        previousElement.removeEventListener("touchstart", handleTimelineUserScrollIntent);
        previousElement.removeEventListener("pointerdown", handleTimelinePointerIntent);
      }
      if (viewportElement) {
        viewportElement.addEventListener("scroll", handleTimelineViewportScroll, { passive: true });
        viewportElement.addEventListener("wheel", handleTimelineUserScrollIntent, { passive: true });
        viewportElement.addEventListener("touchstart", handleTimelineUserScrollIntent, { passive: true });
        viewportElement.addEventListener("pointerdown", handleTimelinePointerIntent, { passive: true });
      }
      handleTimelineViewportScroll();
    },
    { flush: "post" }
  );

  watch(latestVisibleTurnLayoutSignature, handleVisibleTurnLayoutChange, { flush: "post" });

  onMounted(() => {
    window.addEventListener("keydown", handleTimelineKeyboardIntent, true);
    setupContentResizeObserver();
    handleTimelineViewportScroll();
    queueScrollToLatestTurn("auto");
  });

  onBeforeUnmount(() => {
    isDestroyed = true;
    window.removeEventListener("keydown", handleTimelineKeyboardIntent, true);
    const viewport = getViewport();
    if (viewport) {
      viewport.removeEventListener("scroll", handleTimelineViewportScroll);
      viewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
      viewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
      viewport.removeEventListener("pointerdown", handleTimelinePointerIntent);
    }
    if (scrollAfterPaintFrameId.value != null) {
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
    }
    clearScrollQueueWatchdog();
    cancelScrollLerp();
    clearAutoFollowIdleTimer();
    teardownContentResizeObserver();
  });

  return {
    showScrollToBottom,
    unreadCount,
    scrollQueued,
    streamAutoFollowEnabled,
    handleScrollToBottom,
    handleMarkdownRenderComplete,
    handleLatestTurnSignatureChange,
    handleSubmittingChange,
    handleMessageCountChange,
    setAutoFollowEnabled,
    collectMetrics
  };
}
