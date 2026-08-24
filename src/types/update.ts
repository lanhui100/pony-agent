/**
 * PA-099：应用更新检测类型（GitHub Releases latest 对齐的原始快照与状态机）。
 * 设计约束见 openspec/changes/2026-08-24-add-app-update-check/proposal.md：
 * - 持久化只存原始数据，`hasUpdate` 一律由 store 用当前版本现算；
 * - API 的 html_url 不进入任何存储/状态/跳转链（构造式跳转 URL）。
 */

/** 更新检测状态机。 */
export type UpdateStatus =
  | "idle" // 尚未检查过且无缓存
  | "checking" // 请求进行中
  | "up-to-date" // 已检查，无比当前版本更新的 release
  | "available" // 发现比当前版本新的 release
  | "unpublished" // 仓库暂无发布版本（404 负缓存）
  | "error"; // 手动检查失败（后台静默失败不进入此态）

/** 一次成功响应中与 UI 相关的原始字段（不含 html_url）。 */
export interface AppReleaseInfo {
  tagName: string;
  /** GitHub 返回的发布名；缺失时为 null（仅展示用）。 */
  name: string | null;
  /** published_at 解析结果；缺失或非法时为 null。 */
  publishedAtMs: number | null;
}

/** localStorage 缓存文件结构（key: pony-agent.update-check.v1）。release=null 即 404 负缓存。 */
export interface UpdateCacheFile {
  checkedAtMs: number;
  release: AppReleaseInfo | null;
}

/** 用户偏好（key: pony-agent.update-prefs.v1）。 */
export interface UpdatePrefs {
  autoCheck: boolean;
}
