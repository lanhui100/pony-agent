<script setup lang="ts">
import { computed, ref, watch, nextTick } from "vue";
import type { TurnBucket } from "@/components/chat/WorkspaceTurnItem.vue";
import { formatTurnSummary, type TurnDialSummary } from "./turn-dial-utils";

const props = withDefaults(
  defineProps<{
    turns: TurnBucket[];
    activeTurnId: string | null;
    stickToSidebar?: boolean;
  }>(),
  {
    stickToSidebar: false
  }
);

const emit = defineEmits<{
  (e: "select-turn", turnId: string): void;
}>();

const hoveredTurnIndex = ref<number | null>(null);
const scrollerRef = ref<HTMLElement | null>(null);
const previewTop = ref<number>(0);
const isHovering = ref(false);

const summaries = computed<TurnDialSummary[]>(() => {
  return props.turns.map((turn, index) => formatTurnSummary(turn, index + 1));
});

const activeIndex = computed<number>(() => {
  if (!props.activeTurnId) {
    return props.turns.length > 0 ? props.turns.length - 1 : -1;
  }
  const idx = props.turns.findIndex((t) => t.turnId === props.activeTurnId);
  return idx >= 0 ? idx : (props.turns.length > 0 ? props.turns.length - 1 : -1);
});

const hoveredSummary = computed<TurnDialSummary | null>(() => {
  if (hoveredTurnIndex.value === null) return null;
  return summaries.value[hoveredTurnIndex.value] ?? null;
});

function handleTickClick(turnId: string) {
  emit("select-turn", turnId);
}

function handlePointerEnterTick(index: number, event: MouseEvent) {
  hoveredTurnIndex.value = index;
  isHovering.value = true;
  updatePreviewPosition(event.currentTarget as HTMLElement);
}

function handlePointerLeaveDial() {
  hoveredTurnIndex.value = null;
  isHovering.value = false;
}

function updatePreviewPosition(targetEl: HTMLElement | null) {
  if (!targetEl || !scrollerRef.value) return;
  const scrollerRect = scrollerRef.value.getBoundingClientRect();
  const targetRect = targetEl.getBoundingClientRect();
  previewTop.value = targetRect.top - scrollerRect.top + targetRect.height / 2;
}

// 自动让激活的刻度在拨盘滚动区域中居中
watch(activeIndex, (newIdx) => {
  if (newIdx < 0 || !scrollerRef.value) return;
  void nextTick(() => {
    const scroller = scrollerRef.value;
    if (!scroller) return;
    const tickEl = scroller.querySelector(`[data-turn-index="${newIdx}"]`) as HTMLElement | null;
    if (!tickEl) return;
    const offset = tickEl.offsetTop - scroller.clientHeight / 2 + tickEl.clientHeight / 2;
    scroller.scrollTo({
      top: Math.max(0, offset),
      behavior: "smooth"
    });
  });
});
</script>

<template>
  <nav
    v-if="turns.length >= 2"
    :class="[
      'turn-dial-root',
      stickToSidebar ? 'turn-dial-stick' : ''
    ]"
    aria-label="Turn Navigation Dial"
    @pointerleave="handlePointerLeaveDial"
  >
    <!-- 悬停预览卡片 -->
    <Transition name="preview-fade">
      <aside
        v-if="isHovering && hoveredSummary"
        class="turn-preview-card"
        :style="{ top: `${previewTop}px` }"
        data-testid="turn-preview-card"
      >
        <header class="preview-header">
          <span class="preview-badge">Turn {{ hoveredSummary.turnNumber }}</span>
        </header>
        <p v-if="hoveredSummary.prompt" class="preview-prompt">
          {{ hoveredSummary.prompt }}
        </p>
        <p v-if="hoveredSummary.response" class="preview-response">
          {{ hoveredSummary.response }}
        </p>
      </aside>
    </Transition>

    <!-- 刻度轴容器 -->
    <div ref="scrollerRef" class="dial-scroller">
      <div class="dial-track">
        <button
          v-for="(turn, index) in turns"
          :key="turn.turnId"
          type="button"
          :data-turn-index="index"
          :data-turn-id="turn.turnId"
          :aria-current="index === activeIndex ? 'true' : undefined"
          :class="[
            'dial-tick',
            index === activeIndex ? 'dial-tick-active' : '',
            index === hoveredTurnIndex ? 'dial-tick-hovered' : ''
          ]"
          :title="`跳转到第 ${index + 1} 轮`"
          @click="handleTickClick(turn.turnId)"
          @pointerenter="handlePointerEnterTick(index, $event)"
        >
          <span class="tick-line" />
        </button>
      </div>
    </div>
  </nav>
</template>

<style scoped>
.turn-dial-root {
  position: absolute;
  right: 12px;
  top: 50%;
  transform: translateY(-50%);
  z-index: 25;
  display: flex;
  align-items: center;
  user-select: none;
  pointer-events: none;
  transition: right 300ms cubic-bezier(0.16, 1, 0.3, 1);
}

.turn-dial-root.turn-dial-stick {
  right: 2px;
}

.dial-scroller {
  pointer-events: auto;
  position: relative;
  max-height: min(70vh, 420px);
  overflow-y: auto;
  overflow-x: hidden;
  padding: 8px 4px;
  scrollbar-width: none;
  overscroll-behavior: contain;
  mask-image: linear-gradient(
    to bottom,
    transparent 0px,
    black 16px,
    black calc(100% - 16px),
    transparent 100%
  );
}

.dial-scroller::-webkit-scrollbar {
  display: none;
}

.dial-track {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
  padding: 8px 0;
}

.dial-tick {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  height: 12px;
  width: 24px;
  padding: 0;
  margin: 0;
  background: transparent;
  border: none;
  cursor: pointer;
  outline: none;
}

.tick-line {
  display: block;
  height: 2px;
  width: 12px;
  border-radius: 9999px;
  background-color: rgb(214 211 209); /* stone-300 */
  transform-origin: right center;
  transform: scaleX(0.7);
  transition: transform 160ms cubic-bezier(0.16, 1, 0.3, 1),
              background-color 160ms ease,
              width 160ms cubic-bezier(0.16, 1, 0.3, 1);
}

.dial-tick-hovered .tick-line {
  background-color: rgb(120 113 108); /* stone-500 */
  transform: scaleX(1.1);
  width: 16px;
}

.dial-tick-active .tick-line {
  background-color: rgb(28 25 23); /* stone-900 */
  transform: scaleX(1.4);
  width: 18px;
}

/* 预览卡片 */
.turn-preview-card {
  position: absolute;
  right: calc(100% + 12px);
  transform: translateY(-50%);
  width: 240px;
  padding: 10px 12px;
  background-color: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(8px);
  border: 1px solid rgb(231 229 228); /* stone-200 */
  border-radius: 8px;
  box-shadow: 0 4px 16px -2px rgba(0, 0, 0, 0.08), 0 2px 4px -2px rgba(0, 0, 0, 0.04);
  pointer-events: none;
  z-index: 30;
}

.preview-header {
  display: flex;
  align-items: center;
  margin-bottom: 4px;
}

.preview-badge {
  font-size: 11px;
  font-weight: 600;
  color: rgb(120 113 108); /* stone-500 */
  background-color: rgb(245 245 244); /* stone-100 */
  padding: 2px 6px;
  border-radius: 4px;
}

.preview-prompt {
  font-size: 12px;
  font-weight: 500;
  line-height: 1.4;
  color: rgb(28 25 23); /* stone-900 */
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow: hidden;
  margin-bottom: 4px;
}

.preview-response {
  font-size: 11px;
  line-height: 1.4;
  color: rgb(120 113 108); /* stone-500 */
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
}

/* 动效 */
.preview-fade-enter-active,
.preview-fade-leave-active {
  transition: opacity 120ms ease, transform 120ms ease;
}

.preview-fade-enter-from,
.preview-fade-leave-to {
  opacity: 0;
  transform: translateY(-50%) translateX(4px);
}

@media (max-width: 640px) {
  .turn-dial-root {
    display: none;
  }
}
</style>
