<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  ArrowUpCircle,
  Check,
  ChevronRight,
  ExternalLink,
  LoaderCircle,
  Pencil,
  RefreshCw,
  Save,
  Shield,
  TriangleAlert
} from "lucide-vue-next";
import { useSettingsStore } from "@/stores/settings";
import { useUpdateStore } from "@/stores/update";
import Input from "@/components/ui/Input.vue";
import Switch from "@/components/ui/Switch.vue";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";

/**
 * PA-096：配置页"通用" tab 内容（原 SettingsPanel 去壳版）。
 * 工作模式双卡 + 服务密钥编辑行；业务逻辑与原实现一致。
 * PA-099：追加"软件更新"卡片（spec：openspec/changes/2026-08-24-add-app-update-check）。
 * - 跳转 URL 由 store 的 releasePageUrl 构造式给出，禁止使用任何远端返回的 html_url；
 * - release 文本一律插值渲染，禁止 v-html；
 * - 浏览器跳转带 noopener,noreferrer 并判空（弹窗拦截时 console.warn）。
 */

const settingsStore = useSettingsStore();
const { settings, saving } = storeToRefs(settingsStore);
const isCoding = computed(() => settings.value.workspaceMode === "coding");

// ── PA-099：软件更新 ────────────────────────────────────────────────
const updateStore = useUpdateStore();
const updateChecking = computed(() => updateStore.status === "checking");
/** 仅在 available 态非空，模板内免空值收窄问题。 */
const availableRelease = computed(() =>
  updateStore.status === "available" ? updateStore.latest : null
);
const lastCheckedLabel = computed(() => formatTimestamp(updateStore.lastCheckedAtMs));

function checkForUpdateNow() {
  if (updateChecking.value) return;
  void updateStore.checkForUpdates(true);
}

const dateFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "short",
  day: "numeric"
});

function formatTimestamp(ms: number | null): string | null {
  if (ms === null || !Number.isFinite(ms)) return null;
  return dateFormatter.format(new Date(ms));
}

function openReleasePage() {
  const url = updateStore.releasePageUrl;
  if (!url) return;

  if (isTauriAvailable()) {
    safeInvoke("open_url", { url }).catch((error) => {
      // 旧宿主二进制等场景下命令可能失败：告警并回退浏览器打开。
      console.warn("[pony-agent][update] open_url failed, falling back to window.open", error);
      window.open(url, "_blank", "noopener,noreferrer");
    });
    return;
  }

  const opened = window.open(url, "_blank", "noopener,noreferrer");
  if (!opened) {
    console.warn("[pony-agent][update] release page popup was blocked");
  }
}
// ────────────────────────────────────────────────────────────────────

function chooseMode(mode: "coding" | "work") {
  void settingsStore.setWorkspaceMode(mode);
}

const exaKey = ref("");
const exaSaving = ref(false);
const exaKeyPresent = ref(false);
const exaEditing = ref(false);

onMounted(async () => {
  if (!isTauriAvailable()) return;
  try {
    const key = await safeInvoke<string>("get_service_api_key", { service: "exa" });
    exaKey.value = key;
    exaKeyPresent.value = !!key;
  } catch {
    // browser mode
  }
});

function beginEditExa() {
  exaEditing.value = true;
}

function cancelEditExa() {
  exaEditing.value = false;
  if (exaKeyPresent.value) {
    safeInvoke<string>("get_service_api_key", { service: "exa" }).then(k => {
      exaKey.value = k;
    });
  } else {
    exaKey.value = "";
  }
}

async function saveExaKey() {
  if (!isTauriAvailable()) return;
  exaSaving.value = true;
  try {
    await safeInvoke("set_service_api_key", { service: "exa", key: exaKey.value });
    exaKeyPresent.value = !!exaKey.value;
    exaEditing.value = false;
  } catch {
    // ignore
  } finally {
    exaSaving.value = false;
  }
}

function openExa() {
  if (isTauriAvailable()) {
    safeInvoke("open_url", { url: "https://exa.ai/" });
  } else {
    window.open("https://exa.ai/", "_blank");
  }
}
</script>

<template>
  <div class="h-full min-h-0 overflow-y-auto px-1 py-1" data-testid="config-general-section">
    <div class="flex min-h-full flex-col gap-6">
      <div class="space-y-2">
        <div class="text-[12px] font-medium text-stone-500">工作模式</div>
        <div class="grid grid-cols-2 gap-2 max-w-[680px]">
          <button
            type="button"
            class="flex items-start gap-3 rounded-[0.5rem] border px-3 py-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
            :class="isCoding ? 'border-stone-900 bg-stone-900 text-amber-50' : 'border-stone-200 bg-white text-stone-800 hover:border-stone-300 hover:bg-stone-50'"
            :disabled="saving"
            data-testid="config-mode-coding"
            @click="chooseMode('coding')"
          >
            <Check v-if="isCoding" class="mt-0.5 h-4 w-4" />
            <ChevronRight v-else class="mt-0.5 h-4 w-4 text-stone-400" />
            <span>
              <span class="block text-sm font-medium">Coding</span>
              <span class="block text-[12px] opacity-80">代码、调试、实现、测试；含 Trace 观测读面</span>
            </span>
          </button>
          <button
            type="button"
            class="flex items-start gap-3 rounded-[0.5rem] border px-3 py-3 text-left transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300/70"
            :class="!isCoding ? 'border-stone-900 bg-stone-900 text-amber-50' : 'border-stone-200 bg-white text-stone-800 hover:border-stone-300 hover:bg-stone-50'"
            :disabled="saving"
            data-testid="config-mode-work"
            @click="chooseMode('work')"
          >
            <Check v-if="!isCoding" class="mt-0.5 h-4 w-4" />
            <ChevronRight v-else class="mt-0.5 h-4 w-4 text-stone-400" />
            <span>
              <span class="block text-sm font-medium">Work</span>
              <span class="block text-[12px] opacity-80">写作、分析、规划、文档；观测仅保留指标</span>
            </span>
          </button>
        </div>
      </div>

      <div class="space-y-2">
        <div class="text-[12px] font-medium text-stone-500">服务密钥</div>
        <div class="flex items-center gap-2 w-full">
          <div class="flex items-center gap-1.5 text-[13px] font-medium text-stone-900 shrink-0">
            EXA 密钥
            <Shield class="h-3.5 w-3.5 text-stone-500" />
          </div>
          <div class="relative flex-1 max-w-[480px]">
            <Input
              :model-value="exaKey"
              type="password"
              :disabled="!exaEditing || exaSaving"
              placeholder="输入 Exa API Key"
              class="bg-stone-100/80 text-[9px] tracking-[-0.03em]"
              @update:model-value="exaKey = $event"
            />
            <div class="absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5">
              <template v-if="exaEditing">
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700 disabled:pointer-events-none disabled:opacity-50"
                  :disabled="exaSaving"
                  @click="saveExaKey()"
                >
                  <LoaderCircle v-if="exaSaving" class="h-3.5 w-3.5 animate-spin" />
                  <Save v-else class="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700"
                  @click="cancelEditExa()"
                >
                  <span class="text-[10px]">✕</span>
                </button>
              </template>
              <template v-else>
                <button
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700"
                  @click="beginEditExa()"
                >
                  <Pencil class="h-3.5 w-3.5" />
                </button>
              </template>
            </div>
          </div>
          <Check
            v-if="exaKeyPresent && !exaEditing"
            class="h-4 w-4 shrink-0 text-emerald-600"
          />
          <button
            type="button"
            class="inline-flex shrink-0 items-center gap-0.5 text-[11px] text-stone-500 hover:text-stone-800"
            @click="openExa()"
          >
            exa.ai
            <ExternalLink class="h-3 w-3" />
          </button>
        </div>
      </div>

      <!-- PA-099：软件更新卡片（GitHub 发版检测 + 发布页跳转 + 自动检查开关）。 -->
      <div class="space-y-2" data-testid="config-update-section">
        <div class="text-[12px] font-medium text-stone-500">软件更新</div>
        <div class="max-w-[680px] space-y-2.5 rounded-[0.5rem] border border-stone-200 bg-white px-3 py-3">
          <div class="flex items-center justify-between gap-2">
            <div class="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-stone-900">
              当前版本
              <span
                class="rounded-[0.3rem] bg-stone-100 px-1.5 py-0.5 text-[11px] font-normal text-stone-600"
                data-testid="config-update-current-version"
              >v{{ updateStore.currentVersion }}</span>
            </div>
            <button
              type="button"
              class="inline-flex shrink-0 items-center gap-1 rounded-[0.35rem] px-2 py-1 text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-50"
              :class="
                updateChecking
                  ? 'bg-stone-100 text-stone-400'
                  : 'bg-[#f3c98d] text-stone-900 hover:bg-[#f6dfb8]'
              "
              :disabled="updateChecking"
              data-testid="config-update-check-button"
              @click="checkForUpdateNow()"
            >
              <LoaderCircle v-if="updateChecking" class="h-3 w-3 animate-spin" />
              <RefreshCw v-else class="h-3 w-3" />
              {{ updateChecking ? "检查中…" : "检查更新" }}
            </button>
          </div>

          <div class="min-h-5 space-y-1.5 text-[12px] leading-5" data-testid="config-update-status">
            <template v-if="updateChecking">
              <div class="flex items-center gap-1.5 text-stone-500">
                <LoaderCircle class="h-3.5 w-3.5 animate-spin" />
                正在检查更新…
              </div>
            </template>
            <template v-else-if="availableRelease">
              <div class="flex items-start gap-1.5 text-stone-800">
                <ArrowUpCircle class="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
                <span>
                  发现新版本
                  <span class="font-medium">{{ availableRelease.tagName }}</span>
                  <span v-if="availableRelease.name"> · {{ availableRelease.name }}</span>
                  <span v-if="formatTimestamp(availableRelease.publishedAtMs)">
                    （发布于 {{ formatTimestamp(availableRelease.publishedAtMs) }}）
                  </span>
                </span>
              </div>
              <button
                type="button"
                class="inline-flex items-center gap-0.5 text-[11px] text-stone-500 transition hover:text-stone-800"
                data-testid="config-update-release-link"
                @click="openReleasePage()"
              >
                查看发布页
                <ExternalLink class="h-3 w-3" />
              </button>
            </template>
            <template v-else-if="updateStore.status === 'up-to-date'">
              <div class="flex items-center gap-1.5 text-stone-600">
                <Check class="h-3.5 w-3.5 text-emerald-600" />
                当前已是最新版本。
              </div>
            </template>
            <template v-else-if="updateStore.status === 'unpublished'">
              <div class="flex items-center gap-1.5 text-stone-500">仓库暂无发布版本。</div>
            </template>
            <template v-else-if="updateStore.status === 'error'">
              <div class="flex items-center gap-1.5 text-rose-600">
                <TriangleAlert class="h-3.5 w-3.5 shrink-0" />
                {{ updateStore.errorMessage || "检查更新失败，请稍后重试。" }}
              </div>
            </template>
            <template v-else>
              <div class="flex items-center gap-1.5 text-stone-400">尚未检查更新。</div>
            </template>
          </div>

          <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 pt-0.5">
            <label class="flex cursor-pointer items-center gap-2 text-[11px] text-stone-500">
              <Switch
                :model-value="updateStore.autoCheck"
                data-testid="config-update-autocheck-switch"
                @update:model-value="updateStore.setAutoCheck($event)"
              />
              <span>自动检查更新（启动时匿名访问 api.github.com）</span>
            </label>
            <span
              v-if="lastCheckedLabel"
              class="text-[10px] text-stone-400"
              data-testid="config-update-last-checked"
            >
              上次检查：{{ lastCheckedLabel }}
            </span>
          </div>
        </div>
      </div>

      <p v-if="settingsStore.notice" class="text-[11px] leading-5 text-stone-400">{{ settingsStore.notice }}</p>
      <p v-if="settingsStore.error" class="text-[11px] leading-5 text-rose-600">{{ settingsStore.error }}</p>
    </div>
  </div>
</template>
