<script setup lang="ts">
import { computed, type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();

const className = computed(() =>
  cn(
    "rounded-2xl border border-slate-200/80 bg-white/92 shadow-sm backdrop-blur supports-[backdrop-filter]:bg-white/86",
    props.class
  )
);
</script>

<template>
  <section :class="className">
    <slot />
  </section>
</template>
