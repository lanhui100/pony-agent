<script setup lang="ts">
import { Minus, Square, X } from "lucide-vue-next";
import { isTauriAvailable } from "@/lib/tauri";

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
    v-if="isTauriAvailable()"
    class="flex items-center gap-1.5 select-none"
    data-tauri-drag-region
    data-testid="window-controls"
  >
    <button
      type="button"
      class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-500/10 text-stone-500 backdrop-blur-md transition-all hover:bg-stone-500/20 hover:text-stone-800"
      title="最小化"
      aria-label="最小化"
      @click="minimize"
    >
      <Minus class="h-3 w-3" />
    </button>
    <button
      type="button"
      class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-500/10 text-stone-500 backdrop-blur-md transition-all hover:bg-stone-500/20 hover:text-stone-800"
      title="最大化"
      aria-label="最大化"
      @click="toggleMaximize"
    >
      <Square class="h-2.5 w-2.5" />
    </button>
    <button
      type="button"
      class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-500/10 text-stone-500 backdrop-blur-md transition-all hover:bg-red-500 hover:text-white"
      title="关闭"
      aria-label="关闭"
      @click="closeWindow"
    >
      <X class="h-3 w-3" />
    </button>
  </div>
</template>
