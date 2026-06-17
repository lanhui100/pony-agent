import { defineStore } from "pinia";
import { isTauriAvailable, safeInvoke } from "@/lib/tauri";
import type { AppSettings, WorkspaceMode } from "@/types/settings";

const DEFAULT_SETTINGS: AppSettings = {
  workspaceMode: "coding",
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
  },
});

function normalizeAppSettings(settings?: Partial<AppSettings> | null): AppSettings {
  return {
    workspaceMode:
      settings?.workspaceMode === "work" ? "work" : "coding",
  };
}
