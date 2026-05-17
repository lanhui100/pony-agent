<script setup lang="ts">
import { cva, type VariantProps } from "class-variance-authority";
import { computed, type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap rounded-[0.5rem] text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default: "bg-stone-900 text-amber-50 hover:bg-stone-800",
        secondary: "bg-amber-100/80 text-stone-900 hover:bg-amber-200/70",
        outline: "bg-white text-stone-900 hover:bg-amber-50/70",
        ghost: "bg-transparent text-stone-700 hover:bg-white/55"
      },
      size: {
        default: "h-10 px-4 py-2",
        sm: "h-9 rounded-[0.45rem] px-3 text-xs",
        lg: "h-11 px-5"
      }
    },
    defaultVariants: {
      variant: "default",
      size: "default"
    }
  }
);

type ButtonVariants = VariantProps<typeof buttonVariants>;

const props = withDefaults(
  defineProps<{
    class?: HTMLAttributes["class"];
    variant?: ButtonVariants["variant"];
    size?: ButtonVariants["size"];
    type?: "button" | "submit" | "reset";
    disabled?: boolean;
  }>(),
  {
    type: "button",
    variant: "default",
    size: "default",
    disabled: false
  }
);

const className = computed(() => cn(buttonVariants({ variant: props.variant, size: props.size }), props.class));
</script>

<template>
  <button :type="type" :disabled="disabled" :class="className">
    <slot />
  </button>
</template>
