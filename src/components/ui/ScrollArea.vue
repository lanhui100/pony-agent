<script setup lang="ts">
import { computed, ref, type ComponentPublicInstance, type HTMLAttributes } from "vue";
import { ScrollAreaCorner, ScrollAreaRoot, ScrollAreaViewport } from "reka-ui";
import ScrollBar from "@/components/ui/ScrollBar.vue";
import { cn } from "@/lib/utils";

type ScrollAreaViewportInstance = ComponentPublicInstance<{
  viewportElement?: HTMLElement | { value?: HTMLElement | null };
}>;

const props = defineProps<{
  class?: HTMLAttributes["class"];
  viewportClass?: HTMLAttributes["class"];
}>();

const rootClassName = computed(() => cn("relative overflow-hidden", props.class));
const viewportClassName = computed(() => cn("h-full w-full rounded-[inherit]", props.viewportClass));

const viewportRef = ref<ScrollAreaViewportInstance | null>(null);

function resolveViewportElement() {
  const viewportInstance = viewportRef.value;
  const viewportElement = viewportInstance?.viewportElement;
  if (viewportElement instanceof HTMLElement) {
    return viewportElement;
  }
  if (viewportElement && "value" in viewportElement) {
    return viewportElement.value instanceof HTMLElement ? viewportElement.value : null;
  }
  return null;
}

function scrollToBottom(behavior: ScrollBehavior = "smooth") {
  const viewportEl = resolveViewportElement();
  if (!viewportEl) {
    return;
  }

  viewportEl.scrollTo({
    top: viewportEl.scrollHeight,
    behavior
  });
}

defineExpose({
  get viewportEl() {
    return resolveViewportElement();
  },
  scrollToBottom
});
</script>

<template>
  <ScrollAreaRoot :class="rootClassName">
    <ScrollAreaViewport ref="viewportRef" :class="viewportClassName">
      <slot />
    </ScrollAreaViewport>
    <ScrollBar orientation="vertical" />
    <ScrollAreaCorner class="bg-transparent" />
  </ScrollAreaRoot>
</template>
