<script setup lang="ts">
import { computed } from "vue";
import { renderMarkdown } from "@/lib/markdown";

const props = defineProps<{
  content: string;
  toneClass?: string;
}>();

const renderedHtml = computed(() => renderMarkdown(props.content));
</script>

<template>
  <div class="markdown-body" :class="toneClass" v-html="renderedHtml" />
</template>
