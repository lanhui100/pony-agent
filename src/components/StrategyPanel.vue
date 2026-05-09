<template>
  <article class="panel accent">
    <div class="panel-title">重构主线</div>
    <ul class="feature-list feature-list-light">
      <li>保留 Hermes 作为参考实现，不直接在里面演进 Pony Agent 新逻辑。</li>
      <li>用 Tauri UI 作为工作台，优先观察 Rust runtime 的状态变化。</li>
      <li>先打通 Vue 工作台和单轮 runtime，再承接 provider、tool、memory。</li>
      <li>所有长期计划和续作都维护在 `management/task-system/`。</li>
    </ul>
  </article>
</template>
