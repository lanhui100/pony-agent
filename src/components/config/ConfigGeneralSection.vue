<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
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
  Trash2,
  TriangleAlert
} from "lucide-vue-next";
import { SUBAGENT_MAX_CONCURRENT_CEILING, useSettingsStore } from "@/stores/settings";
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
/** 四态文案优先级：安装中 > 检查中 > 可安装 > 空闲（检查中禁用，文案必须随之，避免灰态「立即升级」）。 */
const primaryLabel = computed(() => {
  if (signedInstalling.value) return "正在升级…";
  if (updateChecking.value) return "检查中…";
  if (signedUpdateAvailable.value) return "立即升级";
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
const exaClearing = ref(false);
const exaKeyPresent = ref(false);
const exaEditing = ref(false);
/** 操作反馈（成功/失败一律显式提示，禁止静默无反应）。 */
const exaNotice = ref("");
/** F4c：宿主只回传掩码（`sk-…a1b2`），前端永不持有明文密钥。 */
const exaKeyMasked = ref("");

async function refreshExaKeyMask() {
  if (!isTauriAvailable()) return;
  try {
    const masked = await safeInvoke<string>("get_service_api_key", { service: "exa" });
    exaKeyMasked.value = masked ?? "";
    exaKeyPresent.value = !!exaKeyMasked.value;
  } catch {
    // browser mode
  }
}

onMounted(async () => {
  await refreshExaKeyMask();
});

function beginEditExa() {
  // 掩码不可逆：进入编辑即清空草稿，必须整串重输才能覆盖旧密钥。
  exaKey.value = "";
  exaNotice.value = "";
  exaEditing.value = true;
}

function cancelEditExa() {
  exaEditing.value = false;
  exaKey.value = "";
  exaNotice.value = "";
  void refreshExaKeyMask();
}

async function saveExaKey() {
  if (!isTauriAvailable()) return;
  const draft = exaKey.value.trim();
  // 空草稿不落盘（避免「未修改」被误写成清空密钥）；删除走显式 clearExaKey()。
  if (!draft) {
    exaNotice.value = "请输入新密钥；如需删除已保存的密钥，请点右侧「清除」。";
    return;
  }
  exaSaving.value = true;
  exaNotice.value = "";
  try {
    await safeInvoke("set_service_api_key", { service: "exa", key: draft });
    exaKey.value = "";
    exaEditing.value = false;
    await refreshExaKeyMask();
    exaNotice.value = "Exa 密钥已保存。";
  } catch (error) {
    exaNotice.value = `保存失败：${String(error)}`;
  } finally {
    exaSaving.value = false;
  }
}

/**
 * 显式清除已保存的密钥。
 *
 * 后端 `set_service_api_key(service, "")` 即删除语义（config.rs:328-335：空 key 走
 * `secret_store.delete`），故前端用空串提交即可完成清除，无需新增命令。
 */
async function clearExaKey() {
  if (!isTauriAvailable()) return;
  if (exaSaving.value || exaClearing.value) return;
  exaClearing.value = true;
  exaNotice.value = "";
  try {
    await safeInvoke("set_service_api_key", { service: "exa", key: "" });
    exaKey.value = "";
    exaEditing.value = false;
    await refreshExaKeyMask();
    exaNotice.value = exaKeyPresent.value ? "清除未生效，请重试。" : "已清除 Exa 密钥。";
  } catch (error) {
    exaNotice.value = `清除失败：${String(error)}`;
  } finally {
    exaClearing.value = false;
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

// ── B125：子代理并发上限（1..=8，默认 4；失焦/回车保存，越界由后端 clamp 回显）──
/** 输入草稿（字符串，避免 number input 的空值/半输入态被强转成 NaN）。 */
const subagentConcurrencyDraft = ref(String(settingsStore.effectiveSubagentMaxConcurrent));
const subagentConcurrencyNotice = ref("");
const subagentConcurrencyError = ref("");

// 保存回显/外部变更（如另一窗口改了设置）时同步草稿。
watch(
  () => settingsStore.effectiveSubagentMaxConcurrent,
  (value) => {
    subagentConcurrencyDraft.value = String(value);
  },
);

function onSubagentConcurrencyKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    void commitSubagentConcurrency();
  }
}

async function commitSubagentConcurrency() {
  const parsed = Number.parseInt(subagentConcurrencyDraft.value, 10);
  subagentConcurrencyNotice.value = "";
  subagentConcurrencyError.value = "";

  if (!Number.isFinite(parsed)) {
    // 空/非法输入：静默恢复当前生效值，不写盘。
    subagentConcurrencyDraft.value = String(settingsStore.effectiveSubagentMaxConcurrent);
    return;
  }

  await settingsStore.setSubagentMaxConcurrent(parsed);
  if (settingsStore.error) {
    subagentConcurrencyError.value = "并发上限保存失败，已恢复原值，请重试。";
  } else {
    subagentConcurrencyNotice.value = `已保存，当前生效：${settingsStore.effectiveSubagentMaxConcurrent}`;
  }
  // 后端 clamp 后的回显值才是真值（例如输入 99 → 8、0 → 1）。
  subagentConcurrencyDraft.value = String(settingsStore.effectiveSubagentMaxConcurrent);
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
              :model-value="exaEditing ? exaKey : ''"
              type="password"
              :disabled="!exaEditing || exaSaving"
              :placeholder="!exaEditing && exaKeyPresent ? exaKeyMasked : '输入 Exa API Key'"
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
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700 disabled:pointer-events-none disabled:opacity-50"
                  :disabled="exaClearing"
                  @click="beginEditExa()"
                >
                  <Pencil class="h-3.5 w-3.5" />
                </button>
                <button
                  v-if="exaKeyPresent"
                  type="button"
                  class="inline-flex h-7 w-7 items-center justify-center rounded-[0.35rem] text-stone-400 transition hover:bg-white hover:text-stone-700 disabled:pointer-events-none disabled:opacity-50"
                  :disabled="exaClearing"
                  aria-label="清除已保存的 Exa 密钥"
                  data-testid="config-exa-key-clear"
                  @click="clearExaKey()"
                >
                  <LoaderCircle v-if="exaClearing" class="h-3.5 w-3.5 animate-spin" />
                  <Trash2 v-else class="h-3.5 w-3.5" />
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
        <div
          v-if="exaNotice"
          class="max-w-[480px] text-[11px] leading-5"
          :class="exaNotice.startsWith('已') ? 'text-emerald-700' : 'text-amber-700'"
          role="status"
          aria-live="polite"
          data-testid="config-exa-key-notice"
        >
          {{ exaNotice }}
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

      <!-- B125：子代理并发上限——数字输入，失焦/回车保存，越界由后端 clamp 回显。 -->
      <div class="max-w-[680px] space-y-2" data-testid="config-subagent-concurrency-section">
        <div class="flex items-center justify-between gap-2">
          <span class="text-[13px] font-medium text-stone-900">子代理并发上限</span>
          <div class="flex items-center gap-2">
            <Input
              :model-value="subagentConcurrencyDraft"
              type="number"
              class="h-8 w-16 bg-stone-100/80 text-[12px]"
              :disabled="saving"
              aria-label="子代理并发上限"
              data-testid="config-subagent-concurrency-input"
              @update:model-value="subagentConcurrencyDraft = $event"
              @blur="commitSubagentConcurrency()"
              @keydown="onSubagentConcurrencyKeydown"
            />
            <span class="text-[11px] text-stone-500">1–{{ SUBAGENT_MAX_CONCURRENT_CEILING }}（默认 4）</span>
          </div>
        </div>
        <p
          v-if="subagentConcurrencyNotice"
          class="text-[11px] text-emerald-700"
          role="status"
          aria-live="polite"
          data-testid="config-subagent-concurrency-notice"
        >
          {{ subagentConcurrencyNotice }}
        </p>
        <p
          v-if="subagentConcurrencyError"
          class="flex items-center gap-1.5 text-[11px] text-rose-600"
          data-testid="config-subagent-concurrency-error"
        >
          <TriangleAlert class="h-3.5 w-3.5 shrink-0" />
          {{ subagentConcurrencyError }}
        </p>
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
