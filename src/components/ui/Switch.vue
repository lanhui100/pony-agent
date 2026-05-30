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
        'peer inline-flex h-4 w-7 shrink-0 items-center rounded-full border border-stone-300/90 bg-stone-200/70 px-px transition-colors duration-200 data-[state=checked]:border-[#8b5e34]/90 data-[state=checked]:bg-[#8b5e34] data-[disabled]:cursor-not-allowed data-[disabled]:opacity-45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70',
        props.class
      )
    "
    @update:model-value="handleUpdate"
  >
    <SwitchThumb
      class="block h-3 w-3 rounded-full bg-white shadow-[0_1px_2px_rgba(28,25,23,0.14)] transition-transform duration-200 data-[state=checked]:translate-x-3"
    />
  </SwitchRoot>
</template>
