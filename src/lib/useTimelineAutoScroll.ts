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
  workspaceContentColumnRef: Ref<HTMLElement | null>;
  isSubmitting: Ref<boolean>;
  latestTurnSignature: ComputedRef<string>;
  showDebug: (event: string, patch?: Record<string, unknown>) => void;
  updateFloatingUiPositions: () => void;
};

const AUTO_SCROLL_THRESHOLD_PX = 260;
const SHOW_SCROLL_BOTTOM_THRESHOLD_PX = 400;
const USER_RESUME_THRESHOLD_PX = 50;
const PROGRAMMATIC_SCROLL_MAX_MS = 1600;
const USER_SCROLL_IDLE_MS = 3000;
const SCROLL_LERP_DURATION_MS = 200;

export function useTimelineAutoScroll(options: UseTimelineAutoScrollOptions) {
  const {
    timelineScrollAreaRef,
    scrollAnchorRef,
    workspaceContentColumnRef,
    isSubmitting,
    latestTurnSignature,
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
  let programmaticScrollTargetTop = 0;
  let programmaticScrollUntilMs = 0;
  let userScrollOverrideVersion = 0;
  let lastUserPausedSignature = "";
  let lastUserScrollAtMs = 0;
  let scrollLerpRafId: number | null = null;
  let layoutDirtyWhileQueued = false;
  let contentDirtyWhileAnimating = false;
  let isDestroyed = false;

  function collectMetrics() {
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    return {
      streamAutoFollowEnabled: streamAutoFollowEnabled.value,
      scrollQueued: scrollQueued.value,
      isSubmitting: isSubmitting.value,
      programmaticScrollActive,
      programmaticScrollTargetTop,
      programmaticScrollUntilMs,
      userScrollOverrideVersion,
      lastUserPausedSignature,
      viewportScrollTop: viewport?.scrollTop ?? null,
      viewportScrollHeight: viewport?.scrollHeight ?? null,
      viewportClientHeight: viewport?.clientHeight ?? null,
      distanceToBottom: viewport
        ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
        : null
    };
  }

  function emit(event: string, patch: Record<string, unknown> = {}) {
    showDebug(event, {
      ...collectMetrics(),
      ...patch
    });
  }

  function isTimelineNearBottom() {
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    if (!viewport) {
      return true;
    }

    const distanceToBottom = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    return distanceToBottom <= AUTO_SCROLL_THRESHOLD_PX;
  }

  function getAnchorTargetTop(): number {
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    const anchor = scrollAnchorRef.value;
    if (!viewport) return 0;
    if (!anchor) return viewport.scrollHeight;
    const anchorRect = anchor.getBoundingClientRect();
    const viewportRect = viewport.getBoundingClientRect();
    if (anchorRect.top === viewportRect.top && anchorRect.bottom === viewportRect.bottom) {
      return viewport.scrollHeight;
    }
    return Math.max(0, viewport.scrollTop + (anchorRect.bottom - viewportRect.bottom));
  }

  function cancelScrollLerp() {
    if (scrollLerpRafId != null) {
      window.cancelAnimationFrame(scrollLerpRafId);
      scrollLerpRafId = null;
    }
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

      const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
      if (!viewport) {
        return;
      }

      const anchorTarget = getAnchorTargetTop();
      emit("anchor-compensation-scroll", { targetTop: anchorTarget });
      programmaticScrollActive = true;
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
    programmaticScrollTargetTop = 0;
    programmaticScrollUntilMs = 0;
    cancelScrollLerp();
    if (scrollAfterPaintFrameId.value != null) {
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
      scrollAfterPaintFrameId.value = null;
    }
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

  function resumeTimelineAutoFollow(behavior: ScrollBehavior = "auto") {
    streamAutoFollowEnabled.value = true;
    lastUserPausedSignature = "";
    clearAutoFollowIdleTimer();
    emit("resume-auto-follow", { behavior });
    queueScrollToLatestTurn(behavior, userScrollOverrideVersion);
  }

  function queueScrollToLatestTurn(
    behavior: ScrollBehavior = "smooth",
    expectedOverrideVersion = userScrollOverrideVersion
  ) {
    scheduledScrollRequestId += 1;
    const requestId = scheduledScrollRequestId;
    scrollQueued.value = true;
    emit("queue-scroll-request", { requestId, behavior, expectedOverrideVersion });
    void nextTick().then(() => {
      if (
        requestId !== scheduledScrollRequestId ||
        expectedOverrideVersion !== userScrollOverrideVersion ||
        !streamAutoFollowEnabled.value
      ) {
        if (requestId === scheduledScrollRequestId) {
          scrollQueued.value = false;
        }
        emit("queue-scroll-request:cancelled-before-raf", { requestId, behavior, expectedOverrideVersion });
        return;
      }
      if (scrollAfterPaintFrameId.value != null) {
        if (expectedOverrideVersion !== userScrollOverrideVersion) {
          window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
          scrollAfterPaintFrameId.value = null;
        } else {
          return;
        }
      }

      if (isDestroyed) {
        if (requestId === scheduledScrollRequestId) {
          scrollQueued.value = false;
        }
        return;
      }

      scrollAfterPaintFrameId.value = window.requestAnimationFrame(() => {
        if (
          requestId !== scheduledScrollRequestId ||
          expectedOverrideVersion !== userScrollOverrideVersion ||
          !streamAutoFollowEnabled.value
        ) {
          if (requestId === scheduledScrollRequestId) {
            scrollQueued.value = false;
            scrollAfterPaintFrameId.value = null;
          }
          emit("queue-scroll-request:cancelled-in-raf", { requestId, behavior, expectedOverrideVersion });
          return;
        }

        scrollAfterPaintFrameId.value = null;
        scrollQueued.value = false;
        const scrollArea = timelineScrollAreaRef.value;
        const viewport = scrollArea?.viewportEl ?? null;
        if (viewport) {
          const anchorTarget = getAnchorTargetTop();
          programmaticScrollActive = true;
          programmaticScrollTargetTop = anchorTarget;
          programmaticScrollUntilMs = Date.now() + PROGRAMMATIC_SCROLL_MAX_MS;
          emit("scroll-to-latest-turn", {
            requestId,
            behavior,
            targetTop: anchorTarget,
            expiresAt: programmaticScrollUntilMs
          });
          if (behavior === "auto") {
            viewport.scrollTo({ top: anchorTarget, behavior: "auto" });
            finalizeProgrammaticScrollCycle();
            return;
          }

          cancelScrollLerp();
          const startTop = viewport.scrollTop;
          const distance = anchorTarget - startTop;
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

            const finalTarget = getAnchorTargetTop();
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
          emit("scroll-to-latest-turn:fallback", { requestId, behavior });
          scrollArea.scrollToBottom(behavior);
        }
      });
    });
  }

  function handleTimelineViewportScroll() {
    updateFloatingUiPositions();
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    if (programmaticScrollActive && viewport) {
      const nearBottom = isTimelineNearBottom();
      const reachedProgrammaticTarget =
        nearBottom ||
        viewport.scrollTop >= Math.max(programmaticScrollTargetTop - viewport.clientHeight - AUTO_SCROLL_THRESHOLD_PX, 0);
      const programmaticScrollExpired = Date.now() >= programmaticScrollUntilMs;
      if (!reachedProgrammaticTarget && !programmaticScrollExpired) {
        emit("viewport-scroll:programmatic-progress", { reachedProgrammaticTarget, programmaticScrollExpired });
        return;
      }

      programmaticScrollActive = false;
      programmaticScrollTargetTop = 0;
      programmaticScrollUntilMs = 0;
      if (reachedProgrammaticTarget) {
        streamAutoFollowEnabled.value = true;
        lastUserPausedSignature = "";
        clearAutoFollowIdleTimer();
        emit("viewport-scroll:programmatic-complete");
        return;
      }

      emit("viewport-scroll:programmatic-expired");
    }

    const distanceFromBottom = viewport
      ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
      : 0;

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

    if (distanceFromBottom > USER_RESUME_THRESHOLD_PX || distanceFromBottom > SHOW_SCROLL_BOTTOM_THRESHOLD_PX) {
      showScrollToBottom.value = true;
    }

    lastUserScrollAtMs = Date.now();
    emit("viewport-scroll:user-away-from-bottom", { distanceFromBottom, showScrollToBottom: showScrollToBottom.value });
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
    resumeTimelineAutoFollow("smooth");
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
    queueScrollToLatestTurn("auto", userScrollOverrideVersion);
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
          resumeTimelineAutoFollow("auto");
          return;
        }
        emit("latest-turn-signature:follow-disabled", { signature, previousSignature });
        return;
      }

      emit("latest-turn-signature:queue-follow-scroll", { signature, previousSignature });
      queueScrollToLatestTurn("smooth", userScrollOverrideVersion);
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
      queueScrollToLatestTurn("smooth", userScrollOverrideVersion);
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
      showScrollToBottom.value = true;
    }
  }

  watch(
    () => timelineScrollAreaRef.value?.viewportEl ?? null,
    (viewport, previousViewport) => {
      if (previousViewport && "removeEventListener" in previousViewport) {
        previousViewport.removeEventListener("scroll", handleTimelineViewportScroll);
        previousViewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
        previousViewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
        previousViewport.removeEventListener("pointerdown", handleTimelinePointerIntent);
      }
      if (viewport && "addEventListener" in viewport) {
        viewport.addEventListener("scroll", handleTimelineViewportScroll, { passive: true });
        viewport.addEventListener("wheel", handleTimelineUserScrollIntent, { passive: true });
        viewport.addEventListener("touchstart", handleTimelineUserScrollIntent, { passive: true });
        viewport.addEventListener("pointerdown", handleTimelinePointerIntent, { passive: true });
      }
      handleTimelineViewportScroll();
    },
    { flush: "post" }
  );

  onMounted(() => {
    window.addEventListener("keydown", handleTimelineKeyboardIntent, true);
    setupContentResizeObserver();
    handleTimelineViewportScroll();
    queueScrollToLatestTurn("auto");
  });

  onBeforeUnmount(() => {
    isDestroyed = true;
    window.removeEventListener("keydown", handleTimelineKeyboardIntent, true);
    const viewport = timelineScrollAreaRef.value?.viewportEl ?? null;
    if (viewport && "removeEventListener" in viewport) {
      viewport.removeEventListener("scroll", handleTimelineViewportScroll);
      viewport.removeEventListener("wheel", handleTimelineUserScrollIntent);
      viewport.removeEventListener("touchstart", handleTimelineUserScrollIntent);
      viewport.removeEventListener("pointerdown", handleTimelinePointerIntent);
    }
    if (scrollAfterPaintFrameId.value != null) {
      window.cancelAnimationFrame(scrollAfterPaintFrameId.value);
    }
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
