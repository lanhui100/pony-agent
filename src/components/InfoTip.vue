<script setup lang="ts">
import { CircleHelp } from "lucide-vue-next";
import { nextTick, onBeforeUnmount, ref } from "vue";

const props = defineProps<{
  text: string;
}>();

const bubbleRef = ref<HTMLElement | null>(null);
const triggerRef = ref<HTMLElement | null>(null);
const isOpen = ref(false);
const top = ref(0);
const left = ref(0);

function closeTip() {
  isOpen.value = false;
}

function updatePosition() {
  const trigger = triggerRef.value;
  const bubble = bubbleRef.value;
  if (!trigger || !bubble) {
    return;
  }

  const padding = 12;
  const offset = 10;
  const triggerRect = trigger.getBoundingClientRect();
  const bubbleRect = bubble.getBoundingClientRect();

  let nextLeft = triggerRect.left + triggerRect.width / 2 - bubbleRect.width / 2;
  nextLeft = Math.min(
    Math.max(padding, nextLeft),
    window.innerWidth - bubbleRect.width - padding
  );

  let nextTop = triggerRect.bottom + offset;
  if (nextTop + bubbleRect.height + padding > window.innerHeight) {
    nextTop = Math.max(padding, triggerRect.top - bubbleRect.height - offset);
  }

  left.value = nextLeft;
  top.value = nextTop;
}

async function openTip() {
  isOpen.value = true;
  await nextTick();
  updatePosition();
}

function onViewportChange() {
  if (isOpen.value) {
    updatePosition();
  }
}

window.addEventListener("resize", onViewportChange);
window.addEventListener("scroll", onViewportChange, true);

onBeforeUnmount(() => {
  window.removeEventListener("resize", onViewportChange);
  window.removeEventListener("scroll", onViewportChange, true);
});
</script>

<template>
  <span
    ref="triggerRef"
    class="inline-flex"
    @mouseenter="openTip"
    @mouseleave="closeTip"
    @focusin="openTip"
    @focusout="closeTip"
  >
    <button
      type="button"
      class="inline-flex h-5 w-5 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white/70 hover:text-stone-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300"
      aria-label="查看说明"
    >
      <CircleHelp class="h-3.5 w-3.5" />
    </button>

    <Teleport to="body">
      <div
        v-if="isOpen"
        ref="bubbleRef"
        class="pointer-events-none fixed z-50 max-w-[18rem] rounded-[0.45rem] bg-stone-950 px-3 py-2 text-xs leading-5 text-amber-50"
        :style="{ top: `${top}px`, left: `${left}px` }"
      >
        {{ props.text }}
      </div>
    </Teleport>
  </span>
</template>
