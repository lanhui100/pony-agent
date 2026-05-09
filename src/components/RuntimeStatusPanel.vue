<script setup lang="ts">
import { storeToRefs } from "pinia";
import { onMounted } from "vue";
import { useRuntimeStore } from "../stores/runtime";

const runtimeStore = useRuntimeStore();
const { error, health, phaseLabel } = storeToRefs(runtimeStore);

onMounted(() => {
  void runtimeStore.fetchHealth();
});
</script>

<template>
  <article class="panel">
    <div class="panel-title">运行时状态</div>
    <div v-if="health" class="health ready">
      <div><strong>{{ health.appName }}</strong> {{ health.appVersion }}</div>
      <div>runtime: {{ health.runtime }}</div>
      <div>graph: {{ health.graphEngine }}</div>
      <div>phase: {{ phaseLabel }}</div>
    </div>
    <div v-else-if="error" class="health failed">
      {{ error }}
    </div>
    <div v-else class="health loading">
      正在连接 Rust 后端...
    </div>

    <div class="runtime-highlights">
      <div class="badge">Session</div>
      <div class="badge">Provider</div>
      <div class="badge">Tool Router</div>
      <div class="badge">Trace</div>
    </div>
  </article>
</template>
