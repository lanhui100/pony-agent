<script setup lang="ts">
import ChatPanel from "./components/ChatPanel.vue";
import GraphTracePanel from "./components/GraphTracePanel.vue";
import RuntimeStatusPanel from "./components/RuntimeStatusPanel.vue";
import StrategyPanel from "./components/StrategyPanel.vue";
</script>

<template>
  <main class="shell">
    <section class="hero">
      <div class="eyebrow">Tauri + Rust Agent Runtime</div>
      <h1>Pony Agent</h1>
      <p class="lede">
        以学习模式重构智能体核心，先做一个可观察、可调试、可逐步承接 Rust runtime 的桌面工作台。
      </p>
    </section>

    <section class="workbench">
      <ChatPanel />
      <RuntimeStatusPanel />
      <GraphTracePanel />
      <StrategyPanel />
    </section>
  </main>
</template>
