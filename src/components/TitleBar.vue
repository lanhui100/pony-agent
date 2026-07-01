<script setup lang="ts">
import { Minus, Square, X } from "lucide-vue-next";
import { isTauriAvailable } from "@/lib/tauri";
import PonyBrandIcon from "@/components/PonyBrandIcon.vue";

async function minimize() {
  if (!isTauriAvailable()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().minimize();
}

async function toggleMaximize() {
  if (!isTauriAvailable()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().toggleMaximize();
}

async function closeWindow() {
  if (!isTauriAvailable()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}
</script>

<template>
  <div
    class="flex h-9 shrink-0 items-center justify-between bg-[#eee5d9] mb-2 px-3 select-none"
    data-tauri-drag-region
  >
    <div class="flex items-center gap-2 text-sm font-medium text-stone-700" data-tauri-drag-region>
      <PonyBrandIcon class-name="h-5 w-5" />
      Pony Agent
    </div>

    <div class="flex items-center" data-tauri-drag-region>
      <button
        type="button"
        class="inline-flex h-9 w-11 items-center justify-center text-stone-400 transition-colors hover:bg-stone-200 hover:text-stone-600"
        title="最小化"
        @click="minimize"
      >
        <Minus class="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        class="inline-flex h-9 w-11 items-center justify-center text-stone-400 transition-colors hover:bg-stone-200 hover:text-stone-600"
        title="最大化"
        @click="toggleMaximize"
      >
        <Square class="h-3 w-3" />
      </button>
      <button
        type="button"
        class="inline-flex h-9 w-11 items-center justify-center text-stone-400 transition-colors hover:bg-red-500 hover:text-white"
        title="关闭"
        @click="closeWindow"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>
  </div>
</template>
