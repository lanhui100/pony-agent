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
    class="flex h-9 shrink-0 items-center justify-between bg-white/76 px-3 shadow-[0_1px_3px_rgba(60,40,20,0.04)] backdrop-blur-[8px] select-none"
    data-tauri-drag-region
  >
    <div class="flex items-center gap-2 text-sm font-medium text-stone-700" data-tauri-drag-region>
      <PonyBrandIcon class-name="h-5 w-5" />
      Pony Agent
    </div>

    <div class="flex items-center gap-1.5" data-tauri-drag-region>
      <button
        type="button"
        class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-transparent text-stone-500 transition-all hover:bg-stone-500/20 hover:text-stone-800"
        title="最小化"
        @click="minimize"
      >
        <Minus class="h-3 w-3" />
      </button>
      <button
        type="button"
        class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-transparent text-stone-500 transition-all hover:bg-stone-500/20 hover:text-stone-800"
        title="最大化"
        @click="toggleMaximize"
      >
        <Square class="h-2.5 w-2.5" />
      </button>
      <button
        type="button"
        class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-transparent text-stone-500 transition-all hover:bg-red-500 hover:text-white"
        title="关闭"
        @click="closeWindow"
      >
        <X class="h-3 w-3" />
      </button>
    </div>
  </div>
</template>
