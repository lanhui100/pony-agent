<script setup lang="ts">
import { cva, type VariantProps } from "class-variance-authority";
import { computed, type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-[0.45rem] px-2.5 py-1 text-[11px] font-medium tracking-wide",
  {
    variants: {
      variant: {
        default: "bg-stone-900 text-amber-50",
        secondary: "bg-white/72 text-stone-700",
        success: "bg-amber-100/75 text-amber-900",
        warning: "bg-amber-200/65 text-amber-950"
      }
    },
    defaultVariants: {
      variant: "secondary"
    }
  }
);

type BadgeVariants = VariantProps<typeof badgeVariants>;

const props = withDefaults(
  defineProps<{
    class?: HTMLAttributes["class"];
    variant?: BadgeVariants["variant"];
  }>(),
  {
    variant: "secondary"
  }
);

const className = computed(() => cn(badgeVariants({ variant: props.variant }), props.class));
</script>

<template>
  <span :class="className">
    <slot />
  </span>
</template>
