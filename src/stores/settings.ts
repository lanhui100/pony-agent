import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type { AppSettings, WorkspaceMode } from "@/types/settings";

/**
 * Wave43 D7：`agentTraceReporting` 缺省 `false`——诊断上报默认关闭（隐私 fail-closed），
 * 必须由用户在配置页显式开启后才产生任何 span 出口。
 */
const DEFAULT_SETTINGS: AppSettings = {
  workspaceMode: "coding",
  agentTraceReporting: false,
};

/**
 * B125：子代理并发上限的默认值与硬顶。
 *
 * 与 Rust 侧 `DEFAULT_SUBAGENT_MAX_CONCURRENT` / `SUBAGENT_MAX_CONCURRENT_CEILING`
 * 严格对齐（后端仍是权威：越界值保存时会被 clamp 并回显）。
 */
export const SUBAGENT_MAX_CONCURRENT_DEFAULT = 4;
export const SUBAGENT_MAX_CONCURRENT_CEILING = 8;

type SettingsState = {
  settings: AppSettings;
  loading: boolean;
  saving: boolean;
  error: string | null;
  notice: string | null;
  costBudgetUsd?: number | null;
  usageGovernancePolicy?: AppSettings["usageGovernancePolicy"];
};

export const useSettingsStore = defineStore("settings", {
  state: (): SettingsState => ({
    settings: { ...DEFAULT_SETTINGS },
    loading: false,
    saving: false,
    error: null,
    notice: null,
    costBudgetUsd: null,
    usageGovernancePolicy: undefined,
  }),
  getters: {
    workspaceMode: (state) => state.settings.workspaceMode,
    effectiveCostBudgetUsd: (state): number | null => {
      if (typeof state.costBudgetUsd === "number") return state.costBudgetUsd;
      if (typeof state.settings.costBudgetUsd === "number") return state.settings.costBudgetUsd;
      return null;
    },
    effectiveUsageGovernancePolicy: (state) => state.usageGovernancePolicy ?? state.settings.usageGovernancePolicy,
    // 向后兼容旧 getter
    usagePolicy: (state) => state.usageGovernancePolicy ?? state.settings.usageGovernancePolicy,
    /** B125：未配置时回落默认值，模板/展示统一从这里读。 */
    effectiveSubagentMaxConcurrent: (state): number =>
      state.settings.subagentMaxConcurrent ?? SUBAGENT_MAX_CONCURRENT_DEFAULT,
  },
  actions: {
    async loadSettings() {
      this.loading = true;
      this.error = null;
      this.notice = null;

      try {
        if (!isTauriAvailable()) {
          this.settings = { ...DEFAULT_SETTINGS };
          this.notice = "当前是浏览器预览模式，设置仅保留在当前页面会话中。";
          return;
        }

        const settings = await safeInvoke<AppSettings>("load_app_settings");
        this.settings = normalizeAppSettings(settings);
      } catch (error) {
        this.error = `加载应用设置失败：${String(error)}`;
      } finally {
        this.loading = false;
      }
    },
    async saveSettings() {
      this.saving = true;
      this.error = null;
      this.notice = null;

      try {
        if (!isTauriAvailable()) {
          this.notice = "当前是浏览器预览模式，设置仅保留在当前页面会话中。";
          return;
        }

        const settings = await safeInvoke<AppSettings>("save_app_settings", {
          settings: this.settings,
        });
        this.settings = normalizeAppSettings(settings);
        this.notice = "应用设置已保存。";
      } catch (error) {
        this.error = `保存应用设置失败：${String(error)}`;
      } finally {
        this.saving = false;
      }
    },
    async setWorkspaceMode(mode: WorkspaceMode) {
      if (this.settings.workspaceMode === mode) {
        return;
      }

      this.settings.workspaceMode = mode;
      await this.saveSettings();
    },
    async setAgentTraceReporting(enabled: boolean) {
      if (this.settings.agentTraceReporting === enabled) {
        return;
      }

      const previous = this.settings.agentTraceReporting;
      this.settings.agentTraceReporting = enabled;
      await this.saveSettings();

      if (this.error) {
        // 落盘失败即回滚：避免开关显示「已开启」而实际未持久化（隐私 fail-closed）。
        this.settings.agentTraceReporting = previous;
      }
    },
    /**
     * B125：保存子代理并发上限（越界值 clamp 到 1..=8，与后端同一口径）。
     * 失败回滚到保存前值，避免界面显示未真正生效的上限。
     */
    async setSubagentMaxConcurrent(limit: number) {
      const normalized = normalizeSubagentMaxConcurrent(limit);
      if (normalized === undefined || this.settings.subagentMaxConcurrent === normalized) {
        return;
      }

      const previous = this.settings.subagentMaxConcurrent;
      this.settings.subagentMaxConcurrent = normalized;
      await this.saveSettings();

      if (this.error) {
        this.settings.subagentMaxConcurrent = previous;
      }
    },
  },
});

function normalizeAppSettings(settings?: Partial<AppSettings> | null): AppSettings {
  // wave43-B7：透传未知字段以防设置回写丢字段。`load_app_settings` 返回的 Rust
  // `AppSettings` 含前端未建模的键（如 `toolRules`）；若只重建已知字段，
  // 任何一次 `save_app_settings` 都会把它们覆盖成缺省值（静默数据丢失）。
  // 故以原始对象为基线，仅覆盖需要规范化的已知字段。
  return {
    ...(settings ?? {}),
    workspaceMode:
      settings?.workspaceMode === "work" ? "work" : "coding",
    // 老 settings.json 无该键（legacy）时回落 false；仅显式 true 才开启上报。
    agentTraceReporting: settings?.agentTraceReporting === true,
    otelEndpoint: settings?.otelEndpoint ?? undefined,
    otelToken: settings?.otelToken ?? undefined,
    otelServiceName: settings?.otelServiceName ?? undefined,
    otelSamplingRatio: typeof settings?.otelSamplingRatio === "number" ? settings.otelSamplingRatio : undefined,
    costBudgetUsd: typeof settings?.costBudgetUsd === "number" ? settings.costBudgetUsd : undefined,
    usageGovernancePolicy: settings?.usageGovernancePolicy ? { ...settings.usageGovernancePolicy } : undefined,
    subagentMaxConcurrent: normalizeSubagentMaxConcurrent(settings?.subagentMaxConcurrent),
  } as AppSettings;
}

/** B125：并发上限合法化（整数、1..=硬顶）；非法值一律丢弃（回落默认值）。 */
function normalizeSubagentMaxConcurrent(value?: number | null): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }
  return Math.min(
    SUBAGENT_MAX_CONCURRENT_CEILING,
    Math.max(1, Math.round(value)),
  );
}
