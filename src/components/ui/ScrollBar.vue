<script setup lang="ts">
import { computed, type HTMLAttributes } from "vue";
import {
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  type ScrollAreaScrollbarProps
} from "reka-ui";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    class?: HTMLAttributes["class"];
    orientation?: ScrollAreaScrollbarProps["orientation"];
  }>(),
  {
    orientation: "vertical"
  }
);

const className = computed(() =>
  cn(
    "flex touch-none select-none p-[1px] transition-colors",
    props.orientation === "vertical" && "h-full w-2.5 border-l border-l-transparent",
    props.orientation === "horizontal" && "h-2.5 flex-col border-t border-t-transparent",
    props.class
  )
);
</script>

<template>
  <ScrollAreaScrollbar :orientation="orientation" :class="className">
    <ScrollAreaThumb class="relative flex-1 rounded-full bg-stone-300/90 hover:bg-stone-400/90" />
  </ScrollAreaScrollbar>
</template>
