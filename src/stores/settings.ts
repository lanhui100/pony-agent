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

type SettingsState = {
  settings: AppSettings;
  loading: boolean;
  saving: boolean;
  error: string | null;
  notice: string | null;
};

export const useSettingsStore = defineStore("settings", {
  state: (): SettingsState => ({
    settings: { ...DEFAULT_SETTINGS },
    loading: false,
    saving: false,
    error: null,
    notice: null,
  }),
  getters: {
    workspaceMode: (state) => state.settings.workspaceMode,
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
  } as AppSettings;
}
