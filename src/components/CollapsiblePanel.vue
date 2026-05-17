<script setup lang="ts">
import { ChevronDown } from "lucide-vue-next";
import { ref } from "vue";
import InfoTip from "@/components/InfoTip.vue";

const props = withDefaults(
  defineProps<{
    title: string;
    description?: string;
    tip?: string;
    defaultOpen?: boolean;
    tone?: "slate" | "sky" | "sage";
  }>(),
  {
    description: "",
    tip: "",
    defaultOpen: true,
    tone: "slate"
  }
);

const open = ref(props.defaultOpen);

const toneClassMap = {
  slate: "bg-white/72",
  sky: "bg-amber-50/56",
  sage: "bg-[#efe8db]"
};
</script>

<template>
  <section class="rounded-[0.55rem] px-4 py-4" :class="toneClassMap[props.tone]">
    <button
      type="button"
      class="flex w-full items-start justify-between gap-3 text-left"
      @click="open = !open"
    >
      <div class="min-w-0 space-y-1">
        <div class="flex items-center gap-2">
          <slot name="icon" />
          <div class="text-sm font-semibold text-stone-950">{{ title }}</div>
          <InfoTip v-if="tip" :text="tip" />
        </div>
        <p v-if="description" class="text-xs leading-5 text-stone-500">
          {{ description }}
        </p>
      </div>
      <ChevronDown class="mt-0.5 h-4 w-4 shrink-0 text-stone-400 transition" :class="{ 'rotate-180': open }" />
    </button>

    <div v-show="open" class="pt-4">
      <slot />
    </div>
  </section>
</template>
