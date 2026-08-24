/** PA-096：配置页 tab 标识（App 会话内受控状态，不持久化 + ConfigPage 受控渲染共用）。 */
export type ConfigTab = "general" | "models" | "tools";

/**
 * 左侧栏导航请求/高亮值集合（emit 与 currentPage prop 同集合）。
 *
 * ADR 0013：左栏一级键收敛为 home/settings 两个——观测入口移至对话页右栏
 * 浮动图标按钮（不再经左栏），模型配置直达键并入配置页"模型" tab。
 */
export type SidebarNavigationPage = "home" | "settings";
