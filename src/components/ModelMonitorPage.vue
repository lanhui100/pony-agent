<template>
  <section
    class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden rounded-[0.85rem] border border-stone-200/70 bg-white/78 px-5 py-5 sm:px-6"
  >
    <div class="border-b border-stone-200/70 pb-4">
      <p class="text-[11px] uppercase tracking-[0.22em] text-stone-400">Model Monitor</p>
      <h2 class="mt-2 text-[1.55rem] font-semibold tracking-[-0.03em] text-stone-950">模型监控</h2>
      <p class="mt-2 max-w-2xl text-[13px] leading-6 text-stone-500">
        这里先保留为占位页，后续可以接入模型调用量、成功率、时延、工具调用频次，以及多提供商健康状态观测。
      </p>
    </div>

    <div class="mt-5 grid gap-4 lg:grid-cols-3">
      <section class="rounded-[0.8rem] bg-[#f8f3ea] px-4 py-4">
        <div class="text-[11px] uppercase tracking-[0.18em] text-stone-400">运行概览</div>
        <div class="mt-3 text-[1.8rem] font-semibold tracking-[-0.04em] text-stone-950">--</div>
        <p class="mt-2 text-[12px] leading-5 text-stone-500">预留展示调用总量、活跃会话、当日峰值。</p>
      </section>

      <section class="rounded-[0.8rem] bg-[#f5efe5] px-4 py-4">
        <div class="text-[11px] uppercase tracking-[0.18em] text-stone-400">延迟与稳定性</div>
        <div class="mt-3 text-[1.8rem] font-semibold tracking-[-0.04em] text-stone-950">--</div>
        <p class="mt-2 text-[12px] leading-5 text-stone-500">预留展示首 token 延迟、失败率、重试与降级趋势。</p>
      </section>

      <section class="rounded-[0.8rem] bg-[#efe8db] px-4 py-4">
        <div class="text-[11px] uppercase tracking-[0.18em] text-stone-400">工具协同</div>
        <div class="mt-3 text-[1.8rem] font-semibold tracking-[-0.04em] text-stone-950">--</div>
        <p class="mt-2 text-[12px] leading-5 text-stone-500">预留展示工具成功率、平均耗时和高频工具排行。</p>
      </section>
    </div>

    <div class="mt-5 grid min-h-0 flex-1 gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <section class="rounded-[0.8rem] border border-dashed border-stone-200 bg-white/70 px-4 py-4">
        <div class="text-sm font-medium text-stone-900">计划中的监控面板</div>
        <div class="mt-3 space-y-3 text-[13px] leading-6 text-stone-500">
          <p>按提供商 / 模型切换，观察成功率、超时、429/5xx 分布。</p>
          <p>按会话和任务查看模型调用链路，联动右侧 Trace 做问题定位。</p>
          <p>沉淀日级与周级趋势，为模型切换、限流和预算策略提供依据。</p>
        </div>
      </section>

      <section class="rounded-[0.8rem] bg-stone-900 px-4 py-4 text-stone-100">
        <div class="text-[11px] uppercase tracking-[0.2em] text-stone-400">Next Step</div>
        <div class="mt-3 text-lg font-medium">监控页骨架已就位</div>
        <p class="mt-2 text-[13px] leading-6 text-stone-300">
          后续只需要把真实指标源接进来，就可以沿用当前布局继续扩展，而不需要再改导航结构。
        </p>
      </section>
    </div>
  </section>
</template>
