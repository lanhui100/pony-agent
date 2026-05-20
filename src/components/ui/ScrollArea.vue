<script setup lang="ts">
import { computed, ref, type HTMLAttributes } from "vue";
import { ScrollAreaCorner, ScrollAreaRoot, ScrollAreaViewport } from "reka-ui";
import ScrollBar from "@/components/ui/ScrollBar.vue";
import { cn } from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  viewportClass?: HTMLAttributes["class"];
}>();

const rootClassName = computed(() => cn("relative overflow-hidden", props.class));
const viewportClassName = computed(() => cn("h-full w-full rounded-[inherit]", props.viewportClass));

const viewportEl = ref<HTMLElement | null>(null);

function scrollToBottom(behavior: ScrollBehavior = "smooth") {
  if (!viewportEl.value) {
    return;
  }

  viewportEl.value.scrollTo({
    top: viewportEl.value.scrollHeight,
    behavior
  });
}

defineExpose({
  viewportEl,
  scrollToBottom
});
</script>

<template>
  <ScrollAreaRoot :class="rootClassName">
    <ScrollAreaViewport ref="viewportEl" :class="viewportClassName">
      <slot />
    </ScrollAreaViewport>
    <ScrollBar orientation="vertical" />
    <ScrollAreaCorner class="bg-transparent" />
  </ScrollAreaRoot>
</template>
