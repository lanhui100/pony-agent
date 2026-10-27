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
import { openExternalUrl } from "@/lib/open-external-url";

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
const signedInstalling = computed(() => ["downloading", "installing"].includes(updateStore.signedStatus));
const signedUpdateAvailable = computed(() => updateStore.signedStatus === "available" && updateStore.signedCandidate !== null);
/** 仅在 available 态非空，模板内免空值收窄问题。 */
const availableRelease = computed(() =>
  updateStore.status === "available" ? updateStore.latest : null
);
const lastCheckedLabel = computed(() => formatTimestamp(updateStore.lastCheckedAtMs));

function checkForUpdateNow() {
  if (updateChecking.value) return;
  void Promise.allSettled([
    updateStore.checkForUpdates(true),
    updateStore.checkSignedUpdate()
  ]);
}

function installSignedUpdate() {
  if (signedInstalling.value || !signedUpdateAvailable.value) return;
  void updateStore.installSignedUpdate();
}

// ── Wave-52 v3：版本行主按钮双模（检查更新 ⇄ 立即升级），不再有独立安装按钮 ──
/** 主按钮是否处于「安装/升级」模式：安装中或存在可装的签名候选。 */
const primaryInstallMode = computed(() => signedInstalling.value || signedUpdateAvailable.value);
/** 主按钮忙碌态（两者共用同一个加载图标）。 */
const primaryBusy = computed(() => signedInstalling.value || updateChecking.value);
/** 四态文案优先级：安装中 > 可安装 > 检查中 > 空闲。 */
const primaryLabel = computed(() => {
  if (signedInstalling.value) return "正在升级…";
  if (signedUpdateAvailable.value) return "立即升级";
  if (updateChecking.value) return "检查中…";
  return "检查更新";
});
const primaryDisabled = computed(() => signedInstalling.value || updateChecking.value);

function onPrimaryAction() {
  if (primaryDisabled.value) return;
  if (signedUpdateAvailable.value) {
    installSignedUpdate();
    return;
  }
  checkForUpdateNow();
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

  // PA-101 F1：统一走 openExternalUrl（白名单拒绝禁 window.open 回退，仅告警；
  // 旧二进制缺命令才由出口内部回退浏览器打开）。
  void openExternalUrl(url);
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
  // PA-101 F1：统一走 openExternalUrl（含 catch 语义；白名单拒绝禁回退仅告警）。
  void openExternalUrl("https://exa.ai/");
}

// ── Wave43 D7：全局轨迹上报开关（UI 已精简为单行标签 + 右端 Switch，无注释与子项） ──
// 持久化复用既有 save_app_settings 通道（不新增 Tauri 命令）；core 侧 gate 由 Rust 落地。
const traceReportingError = ref<string | null>(null);

async function toggleAgentTraceReporting(enabled: boolean) {
  if (settings.value.agentTraceReporting === enabled) return;

  traceReportingError.value = null;
  await settingsStore.setAgentTraceReporting(enabled);

  // store 在落盘失败时已回滚末态，这里补一条就近的可读反馈。
  if (settingsStore.error) {
    traceReportingError.value = "开关保存失败，已恢复原状态，请重试。";
  }
}
// ────────────────────────────────────────────────────────────────────
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
        <div
          class="flex w-[680px] max-w-full items-center justify-between gap-2"
          data-testid="config-update-header-row"
        >
          <div class="text-[12px] font-medium text-stone-500" data-testid="config-update-section-title">软件自动更新</div>
          <Switch
            aria-label="软件自动更新"
            :model-value="updateStore.autoCheck"
            data-testid="config-update-autocheck-switch"
            @update:model-value="updateStore.setAutoCheck($event)"
          />
        </div>
        <div class="max-w-[680px] space-y-2.5 rounded-[0.5rem] border border-stone-200 bg-white px-3 py-3">
          <div class="flex items-center justify-between gap-2" data-testid="config-update-version-row">
            <div class="flex min-w-0 items-center gap-1.5 text-[13px] font-medium text-stone-900">
              当前版本
              <span
                class="rounded-[0.3rem] bg-stone-100 px-1.5 py-0.5 text-[11px] font-normal text-stone-600"
                data-testid="config-update-current-version"
              >v{{ updateStore.currentVersion }}</span>
              <span
                v-if="lastCheckedLabel"
                class="text-[10px] font-normal text-stone-400"
                data-testid="config-update-last-checked"
              >
                上次检查：{{ lastCheckedLabel }}
              </span>
            </div>
            <button
              type="button"
              class="inline-flex shrink-0 items-center gap-1 rounded-[0.35rem] px-2 py-1 text-[11px] font-medium transition disabled:cursor-not-allowed disabled:opacity-50"
              :class="
                primaryBusy
                  ? 'bg-stone-100 text-stone-400'
                  : 'bg-[#f3c98d] text-stone-900 hover:bg-[#f6dfb8]'
              "
              :disabled="primaryDisabled"
              data-testid="config-update-check-button"
              @click="onPrimaryAction()"
            >
              <LoaderCircle v-if="primaryBusy" class="h-3 w-3 animate-spin" />
              <ArrowUpCircle v-else-if="primaryInstallMode" class="h-3 w-3" />
              <RefreshCw v-else class="h-3 w-3" />
              {{ primaryLabel }}
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
              <div class="flex items-center gap-1.5 flex-wrap text-stone-800">
                <ArrowUpCircle class="h-4 w-4 shrink-0 text-amber-600" />
                <span>
                  发现新版本
                  <span class="font-medium">{{ availableRelease.tagName }}</span>
                  <span v-if="availableRelease.name && availableRelease.name !== availableRelease.tagName"> · {{ availableRelease.name }}</span>
                  <span v-if="formatTimestamp(availableRelease.publishedAtMs)">
                    （发布于 {{ formatTimestamp(availableRelease.publishedAtMs) }}）
                  </span>
                </span>
                <button
                  type="button"
                  class="inline-flex items-center gap-0.5 text-[11px] text-stone-500 transition hover:text-stone-800"
                  data-testid="config-update-release-link"
                  @click="openReleasePage()"
                >
                  查看发布页
                  <ExternalLink class="h-3 w-3" />
                </button>
              </div>
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

          <div v-if="signedInstalling && updateStore.signedProgress !== null" class="flex items-center gap-2">
            <div
              class="h-1.5 w-28 overflow-hidden rounded-full bg-stone-100"
              data-testid="config-update-progress"
            >
              <div class="h-full bg-amber-400 transition-all" :style="{ width: `${updateStore.signedProgress}%` }" />
            </div>
          </div>
          <div v-if="updateStore.signedStatus === 'pending-restart'" class="text-[12px] text-emerald-700" data-testid="config-update-pending-restart">更新已提交，请重启应用完成更新。</div>
          <div v-if="updateStore.signedStatus === 'relaunch-failed'" class="text-[12px] text-rose-600" data-testid="config-update-relaunch-failed">更新已提交，但重启失败，请手动重启应用。</div>
          <div v-if="updateStore.signedStatus === 'error'" class="text-[12px] text-rose-600" data-testid="config-update-signed-error">
            {{ updateStore.signedErrorMessage || "签名更新失败，请稍后重试。" }}
          </div>
        </div>
      </div>

      <!-- Wave43 D7 / ADR 0050：全局轨迹上报——裸字段行（无分区标题、无边框、无背景色），标签 + 最右端 Switch。 -->
      <div class="max-w-[680px] space-y-2" data-testid="config-diagnostics-section">
        <div class="flex items-center justify-between gap-2">
          <span class="text-[13px] font-medium text-stone-900">全局轨迹上报</span>
          <Switch
            :model-value="settings.agentTraceReporting"
            :disabled="saving"
            aria-label="全局轨迹上报"
            data-testid="config-agent-trace-reporting-switch"
            @update:model-value="toggleAgentTraceReporting($event)"
          />
        </div>

        <div
          v-if="traceReportingError"
          class="flex items-center gap-1.5 text-[12px] text-rose-600"
          data-testid="config-agent-trace-save-error"
        >
          <TriangleAlert class="h-3.5 w-3.5 shrink-0" />
          {{ traceReportingError }}
        </div>
      </div>

      <p v-if="settingsStore.error" class="text-[11px] leading-5 text-rose-600">{{ settingsStore.error }}</p>
    </div>
  </div>
</template>
