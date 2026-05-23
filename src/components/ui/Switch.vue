<script setup lang="ts">
import { computed } from "vue";
import { SwitchRoot, SwitchThumb, type SwitchRootProps } from "reka-ui";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    modelValue?: boolean;
    disabled?: boolean;
    id?: string;
    name?: string;
    class?: string;
  }>(),
  {
    modelValue: false,
    disabled: false,
    id: undefined,
    name: undefined,
    class: undefined
  }
);

const emit = defineEmits<{
  "update:modelValue": [value: boolean];
}>();

const checked = computed<SwitchRootProps["modelValue"]>(() => props.modelValue);

function handleUpdate(value: SwitchRootProps["modelValue"]) {
  emit("update:modelValue", value === true);
}
</script>

<template>
  <SwitchRoot
    :id="id"
    :name="name"
    :disabled="disabled"
    :model-value="checked"
    :class="
      cn(
        'peer inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-stone-300/90 bg-stone-200/80 px-[2px] shadow-[inset_0_1px_2px_rgba(28,25,23,0.08)] transition-colors duration-200 data-[state=checked]:border-amber-500/90 data-[state=checked]:bg-amber-400/90 data-[disabled]:cursor-not-allowed data-[disabled]:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70',
        props.class
      )
    "
    @update:model-value="handleUpdate"
  >
    <SwitchThumb
      class="block h-4 w-4 rounded-full bg-white shadow-[0_1px_2px_rgba(28,25,23,0.18)] transition-transform duration-200 data-[state=checked]:translate-x-4"
    />
  </SwitchRoot>
</template>
