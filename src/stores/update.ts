import { defineStore } from "pinia";
import { version as APP_VERSION } from "../../package.json";
import {
  buildReleasePageUrl,
  describeUpdateCheckError,
  fetchLatestRelease,
  isNewerVersion,
  loadUpdateCache,
  loadUpdatePrefs,
  saveUpdateCache,
  saveUpdatePrefs,
  UpdateCheckError,
  UPDATE_CHECK_MIN_INTERVAL_MS
} from "@/lib/update-check";
import type { AppReleaseInfo, SignedUpdateStatus, UpdateStatus } from "@/types/update";
import { getTauriUpdaterAdapter, isSignedUpdaterAvailable } from "@/lib/tauri-updater";
import type { SignedUpdateCandidate, UpdaterDownloadProgress } from "@/lib/tauri-updater";

/**
 * PA-099：应用更新检测 store。
 * spec：openspec/changes/2026-08-24-add-app-update-check/proposal.md
 *
 * 关键语义（审核定稿，勿回退）：
 * - 缓存只存原始快照；hasUpdate 由本 store 用当前版本现算——应用升级后旧角标自动消失；
 * - initialize() 只做缓存恢复与过期判定，网络检查以 void 后台触发，绝不阻塞启动链；
 * - checkForUpdates 带 in-flight 幂等守卫；手动调用恒绕过 24h 节流；
 * - 失败分路径：手动 → error 态（保角标）；后台 → console.warn 保态；
 * - 无自动重试、无 setInterval 定时器。
 */

type UpdateState = {
  status: UpdateStatus;
  /** 比较基线：package.json version（与 bump-version.ps1 同步链一致）。 */
  currentVersion: string;
  latest: AppReleaseInfo | null;
  errorMessage: string | null;
  lastCheckedAtMs: number | null;
  autoCheck: boolean;
  signedStatus: SignedUpdateStatus;
  signedCandidate: SignedUpdateCandidate | null;
  signedProgress: number | null;
  signedErrorMessage: string | null;
};

export const useUpdateStore = defineStore("update", {
  state: (): UpdateState => ({
    status: "idle",
    currentVersion: APP_VERSION,
    latest: null,
    errorMessage: null,
    lastCheckedAtMs: null,
    autoCheck: true,
    signedStatus: isSignedUpdaterAvailable() ? "idle" : "disabled",
    signedCandidate: null,
    signedProgress: null,
    signedErrorMessage: null
  }),
  getters: {
    /** 唯一驱动侧栏角标的派生值；latest 为不可信输入的现算结果。 */
    hasUpdate(state): boolean {
      return state.latest !== null && isNewerVersion(state.latest.tagName, state.currentVersion);
    },
    /** 构造式跳转 URL（tagName 未通过格式验证时为 null）。 */
    releasePageUrl(state): string | null {
      return state.latest === null ? null : buildReleasePageUrl(state.latest.tagName);
    }
  },
  actions: {
    /** 由 latest 派生非 checking 态。 */
    deriveStatusFromLatest(): UpdateStatus {
      if (this.latest !== null) {
        return this.hasUpdate ? "available" : "up-to-date";
      }
      return this.lastCheckedAtMs !== null ? "unpublished" : "idle";
    },

    /** 距上次检查是否已超自动检查间隔（未来时间戳按当前时间钳制）。 */
    shouldAutoCheck(nowMs: number): boolean {
      if (this.lastCheckedAtMs === null) {
        return true;
      }
      const clamped = Math.min(this.lastCheckedAtMs, nowMs);
      return nowMs - clamped >= UPDATE_CHECK_MIN_INTERVAL_MS;
    },

    /**
     * 启动入口：恢复偏好与缓存（零网络），必要时后台静默补一次检查。
     * 同步完成水合后立即返回，网络结果后续到达。
     */
    async initialize(): Promise<void> {
      this.autoCheck = loadUpdatePrefs().autoCheck;

      const cache = loadUpdateCache();
      if (cache !== null) {
        // 投毒的未来时间戳在水合处一次钳制：同时约束节流判定与"上次检查"展示。
        this.lastCheckedAtMs = Math.min(cache.checkedAtMs, Date.now());
        this.latest = cache.release;
        this.errorMessage = null;
        this.status = this.deriveStatusFromLatest();
      }

      if (!this.autoCheck || !this.shouldAutoCheck(Date.now())) {
        return;
      }

      // 后台检查不进入启动任务等待链（runStartupTask 的 Promise.all 不能被网络拖住）。
      // 重入安全：重复调用 initialize 不会重复拉网——首次调用在同步段即置 checking，
      // 后续调用的后台检查被 checkForUpdates 的 in-flight 守卫吞掉（有测试钉住）。
      void this.checkForUpdates(false);
    },

    /**
     * 执行一次检查。manual=true 时绕过 24h 节流并允许进入 error 态；
     * manual=false 仅由 initialize 在节流判定通过后调用，失败保持原状。
     */
    async checkForUpdates(manual = true): Promise<void> {
      // in-flight 幂等守卫：init 与手点并发、双挂载重入时只保留一个请求（防乱序覆盖）。
      if (this.status === "checking") {
        return;
      }

      const priorStatus = this.status;
      const priorErrorMessage = this.errorMessage;
      this.status = "checking";
      this.errorMessage = null;

      try {
        const release = await fetchLatestRelease();
        const checkedAtMs = Date.now();
        this.latest = release;
        this.lastCheckedAtMs = checkedAtMs;
        this.status = isNewerVersion(release.tagName, this.currentVersion) ? "available" : "up-to-date";
        saveUpdateCache({ checkedAtMs, release });
      } catch (error) {
        // 404 是确定性结论而非故障：写负缓存并落到 unpublished 态（手动亦同）。
        if (error instanceof UpdateCheckError && error.code === "unpublished") {
          const checkedAtMs = Date.now();
          this.latest = null;
          this.lastCheckedAtMs = checkedAtMs;
          this.status = "unpublished";
          saveUpdateCache({ checkedAtMs, release: null });
          return;
        }

        if (manual) {
          this.errorMessage = describeUpdateCheckError(error);
          this.status = "error";
          console.warn("[pony-agent][update] manual check failed", error);
          return;
        }

        // 后台静默失败：保留既有状态与角标；若恢复回 error 态需连同文案一起还原，
        // 否则 UI 会退化为通用兜底文案，与状态不一致。
        this.status = priorStatus;
        this.errorMessage = priorErrorMessage;
        console.warn("[pony-agent][update] background check failed", error);
      }
    },

    async checkSignedUpdate(): Promise<void> {
      if (!isSignedUpdaterAvailable() || this.signedStatus === "disabled") {
        this.signedStatus = "disabled";
        this.signedCandidate = null;
        return;
      }
      if (["checking", "downloading", "installing"].includes(this.signedStatus)) return;

      this.signedStatus = "checking";
      this.signedErrorMessage = null;
      try {
        const candidate = await getTauriUpdaterAdapter().check();
        this.signedCandidate = candidate;
        this.signedStatus = candidate === null ? "idle" : "available";
      } catch (error) {
        this.signedErrorMessage = error instanceof Error ? error.message : "签名更新检查失败。";
        this.signedStatus = "error";
      }
    },

    async installSignedUpdate(): Promise<void> {
      const candidate = this.signedCandidate;
      if (
        candidate === null ||
        this.signedStatus !== "available" ||
        !isSignedUpdaterAvailable()
      ) {
        return;
      }

      this.signedStatus = "downloading";
      this.signedProgress = null;
      this.signedErrorMessage = null;
      try {
        await getTauriUpdaterAdapter().downloadAndInstall(candidate, (progress: UpdaterDownloadProgress) => {
          if (this.signedStatus !== "downloading") return;
          if (progress.contentLength === null) {
            this.signedProgress = null;
            return;
          }
          this.signedProgress = Math.min(100, Math.max(0, Math.round((progress.downloaded / progress.contentLength) * 100)));
        });
        this.signedStatus = "installing";
        // The updater plugin commits before this call. A relaunch error must not be reported as
        // an ordinary install failure; the candidate is kept for startup reconciliation.
        try {
          await getTauriUpdaterAdapter().relaunch();
          this.signedStatus = "pending-restart";
        } catch (error) {
          this.signedErrorMessage = error instanceof Error ? error.message : "更新已提交，但重启失败。";
          this.signedStatus = "relaunch-failed";
        }
      } catch (error) {
        this.signedErrorMessage = error instanceof Error ? error.message : "签名更新安装失败。";
        this.signedStatus = "error";
      }
    },
    setAutoCheck(enabled: boolean) {
      this.autoCheck = enabled;
      saveUpdatePrefs({ autoCheck: enabled });
    }
  }
});
