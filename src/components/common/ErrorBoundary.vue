<template>
  <div v-if="hasError" class="error-boundary-container rounded-lg border border-red-200 bg-red-50/80 p-4" data-testid="error-boundary-fallback">
    <div class="flex items-start gap-3">
      <div class="flex-1">
        <h4 class="text-sm font-semibold text-red-800">组件渲染异常</h4>
        <p class="mt-1 text-xs text-red-600">
          {{ errorMessage || "当前组件遇到不可预期错误，已安全隔离降级。" }}
        </p>
        <button
          type="button"
          class="mt-3 inline-flex items-center rounded bg-red-100 px-2.5 py-1 text-xs font-medium text-red-800 hover:bg-red-200 focus:outline-none focus:ring-2 focus:ring-red-500 focus:ring-offset-2"
          data-testid="error-boundary-retry"
          @click="handleRetry"
        >
          重试
        </button>
      </div>
    </div>
  </div>
  <slot v-else :key="componentKey" />
</template>

<script setup lang="ts">
import { ref, onErrorCaptured } from "vue";
import { reportError } from "@/lib/telemetry";
import { useRuntimeStore } from "@/stores/runtime";

const props = withDefaults(
  defineProps<{
    componentTag?: string;
  }>(),
  {
    componentTag: "AnonymousComponent",
  }
);

const emit = defineEmits<{
  (e: "error", error: unknown): void;
}>();

const hasError = ref(false);
const errorMessage = ref("");
const componentKey = ref(0);

function getActiveTraceId(): string | undefined {
  try {
    return useRuntimeStore().activeTraceId;
  } catch {
    return undefined;
  }
}

onErrorCaptured((err: unknown, _instance, info: string) => {
  hasError.value = true;
  const errorObj = err instanceof Error ? err : new Error(String(err));
  errorMessage.value = errorObj.message || String(err);

  reportError({
    errorType: errorObj.name || "ErrorBoundaryCaptured",
    message: errorMessage.value,
    stack: errorObj.stack,
    trace_id: getActiveTraceId(),
    extra: {
      lifecycleHook: info,
      componentTag: props.componentTag,
    },
    tags: {
      source: "error_boundary",
      componentTag: props.componentTag,
    },
  });

  emit("error", err);

  // 阻止异常继续向上冒泡至全局顶层，实现局部优雅降级
  return false;
});

function handleRetry() {
  hasError.value = false;
  errorMessage.value = "";
  componentKey.value++;
}
</script>
