/** PA-096：配置页 tab 标识（App 会话内受控状态，不持久化 + ConfigPage 受控渲染共用）。 */
export type ConfigTab = "general" | "models" | "tools";

/** PA-096：左侧栏导航请求/高亮值集合（emit 与 currentPage prop 同集合）。 */
export type SidebarNavigationPage = "home" | "models" | "settings" | "telemetry";
