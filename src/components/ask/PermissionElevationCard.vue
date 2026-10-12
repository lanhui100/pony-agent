<script setup lang="ts">
import { computed, ref } from "vue";
import { ShieldAlert, ShieldCheck, XCircle, LoaderCircle } from "lucide-vue-next";
import { safeInvoke } from "@/lib/tauri";

export type PermissionElevationRequest = {
  id: string;
  path: string;
  scope: "read" | "write" | "read-write";
  justification?: string | null;
  timestampMs?: number;
};

const props = defineProps<{
  request: PermissionElevationRequest;
}>();

const emit = defineEmits<{
  (e: "approved", path: string, scope: string): void;
  (e: "rejected", path: string): void;
}>();

const isSubmitting = ref(false);
const errorMsg = ref<string | null>(null);

const scopeLabel = computed(() => {
  switch (props.request.scope) {
    case "read-write":
      return "读写权限 (Read & Write)";
    case "write":
      return "写入权限 (Write)";
    case "read":
    default:
      return "读取权限 (Read)";
  }
});

async function handleAuthorize() {
  isSubmitting.value = true;
  errorMsg.value = null;
  try {
    const scopeToGrant = props.request.scope === "read" ? "read" : "read-write";
    await safeInvoke("authorize_path", {
      path: props.request.path,
      scope: scopeToGrant,
    });
    emit("approved", props.request.path, scopeToGrant);
  } catch (err) {
    errorMsg.value = err instanceof Error ? err.message : String(err);
  } finally {
    isSubmitting.value = false;
  }
}

function handleReject() {
  emit("rejected", props.request.path);
}
</script>

<template>
  <div class="permission-elevation-card rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 text-xs font-mono">
    <div class="flex items-start gap-3">
      <div class="mt-0.5 rounded p-1.5 bg-amber-500/10 text-amber-500">
        <ShieldAlert class="h-4 w-4" />
      </div>

      <div class="flex-1 space-y-2">
        <div class="flex items-center justify-between">
          <div class="font-semibold text-amber-500 flex items-center gap-1.5">
            <span>权限拓展申请 (Permission Escalation)</span>
          </div>
          <span class="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-600 dark:text-amber-400">
            {{ scopeLabel }}
          </span>
        </div>

        <div class="rounded bg-black/10 dark:bg-black/40 p-2 space-y-1">
          <div class="text-zinc-500 dark:text-zinc-400 text-[11px]">目标路径 (Target Path):</div>
          <div class="break-all font-semibold text-zinc-900 dark:text-zinc-100 select-all">
            {{ request.path }}
          </div>
        </div>

        <div v-if="request.justification" class="text-zinc-600 dark:text-zinc-300 italic text-[11px]">
          "{{ request.justification }}"
        </div>

        <div v-if="errorMsg" class="text-rose-500 text-[11px]">
          错误: {{ errorMsg }}
        </div>

        <div class="flex items-center gap-2 pt-1">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded bg-emerald-600 hover:bg-emerald-500 text-white px-2.5 py-1 text-xs transition disabled:opacity-50"
            :disabled="isSubmitting"
            @click="handleAuthorize"
          >
            <LoaderCircle v-if="isSubmitting" class="h-3 w-3 animate-spin" />
            <ShieldCheck v-else class="h-3 w-3" />
            <span>批准并信任路径</span>
          </button>

          <button
            type="button"
            class="flex items-center gap-1.5 rounded border border-zinc-300 dark:border-zinc-700 hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300 px-2.5 py-1 text-xs transition disabled:opacity-50"
            :disabled="isSubmitting"
            @click="handleReject"
          >
            <XCircle class="h-3 w-3" />
            <span>拒绝</span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
