<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useRuntimeStore } from "../stores/runtime";

const runtimeStore = useRuntimeStore();
const { toolActivities, traceSteps } = storeToRefs(runtimeStore);
</script>

<template>
  <article class="panel">
    <div class="panel-title">Graph 与工具轨迹</div>
    <div class="trace-list">
      <div
        v-for="step in traceSteps"
        :key="step.id"
        class="trace-item"
        :data-state="step.state"
      >
        <span class="trace-dot" />
        <span>{{ step.label }}</span>
      </div>
    </div>

    <div class="subsection-title">工具预演</div>
    <div class="tool-list">
      <div v-for="tool in toolActivities" :key="tool.id" class="tool-item">
        <div class="tool-header">
          <strong>{{ tool.name }}</strong>
          <span class="tool-status">{{ tool.status }}</span>
        </div>
        <p>{{ tool.summary }}</p>
      </div>
    </div>
  </article>
</template>
