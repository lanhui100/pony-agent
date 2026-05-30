<script setup lang="ts">
import brandIconUrl from "@/assets/pony-brand-icon.png";

withDefaults(
  defineProps<{
    className?: string;
  }>(),
  {
    className: "h-6 w-6"
  }
);
</script>

<template>
  <img
    :class="className"
    :src="brandIconUrl"
    alt=""
    aria-hidden="true"
    decoding="async"
  />
</template>
