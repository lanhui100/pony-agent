// Workspace 共享常量（PA-079/PA-081 跨端契约）。
// 与后端 `DEFAULT_WORKSPACE_ID = "default"`（agent/workspace.rs）保持一致；
// 侧边栏分组 / 会话构造的 None→default 投影均以此为单一真相源。
export const DEFAULT_WORKSPACE_ID = "default";

// PA-081：激活 Workspace 的 localStorage 单一真相源 key。
export const ACTIVE_WORKSPACE_STORAGE_KEY = "pony-agent.active-workspace.v1";
