<script setup lang="ts">
import { computed } from "vue";
import { renderMarkdown } from "@/lib/markdown";

const props = defineProps<{
  content: string;
  toneClass?: string;
  wrapperClass?: string;
}>();

const renderedHtml = computed(() => renderMarkdown(props.content));
</script>

<template>
  <div class="markdown-body" :class="[wrapperClass, toneClass]" v-html="renderedHtml" />
</template>
