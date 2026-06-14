<script setup lang="ts">
import { TooltipRoot, TooltipTrigger, TooltipContent, TooltipPortal } from "reka-ui";
import { cn } from "@/lib/utils";

withDefaults(
  defineProps<{
    text?: string;
    side?: "top" | "bottom" | "left" | "right";
    sideOffset?: number;
    delayDuration?: number;
  }>(),
  {
    side: "top",
    sideOffset: 4,
    delayDuration: 300,
  }
);
</script>

<template>
  <TooltipRoot :delay-duration="delayDuration">
    <TooltipTrigger as-child>
      <slot />
    </TooltipTrigger>
    <TooltipPortal>
      <TooltipContent
        :side="side"
        :side-offset="sideOffset"
        :class="
          cn(
            'z-50 overflow-hidden rounded-md border border-stone-200 bg-white px-3 py-1.5 text-xs text-stone-700 shadow-sm',
            'animate-in fade-in-0 zoom-in-95',
            'data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95',
            'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2'
          )
        "
      >
        {{ text }}
        <slot name="content" />
      </TooltipContent>
    </TooltipPortal>
  </TooltipRoot>
</template>
