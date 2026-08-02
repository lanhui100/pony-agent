// 运行时常量：跨 domain 共享的数值、文案与 storage key。
// 只放纯常量，不依赖任何模块。

export const MAX_BG_TEXT_BUFFER_CHARS = 50000;
export const MAX_BG_REASONING_BUFFER_CHARS = 20000;

export const RUNTIME_STORAGE_KEY = "pony-agent.runtime-history.v1";
export const CACHED_STATE_VERSION = 2;
export const DEFAULT_SESSION_ID = "local-dev-session";

export const DEFAULT_BROWSER_SESSION_SUMMARY = "浏览器预览会话";
export const DEFAULT_FAILED_TURN_MESSAGE = "本轮执行失败，请查看右侧 trace。";
export const DEFAULT_FAILED_TURN_ERROR = "本轮执行失败。";
export const TIMEOUT_RETRY_PENDING_MESSAGE = "超时后错误重连中...";
export const HYDRATION_TIMEOUT_MS = 15000;

export const RETRIEVED_CONTEXT_FALLBACK_SUMMARY = "当前会话尚未从 Tauri 宿主加载结构化 retrieval 上下文。";

// 流式 flush 与 trace 节流参数
export const STREAM_FLUSH_INTERVAL_MS = 120;
export const STREAM_FLUSH_EAGER_CHARS = 120;
// delta 事件中 traceTimeline 的更新节流窗口：trace 是可观测性数据，允许轻微滞后，
// 避免每个模型输出 chunk 都全量克隆 timeline 而拖慢主对话流式渲染。
export const TRACE_TIMELINE_THROTTLE_MS = 200;

export const OUTPUT_END_PERSIST_DELAY_MS = 1200;

// 浏览器预览兜底常量
export const BROWSER_PREVIEW_PROVIDER_NAME = "browser-preview";
export const BROWSER_PREVIEW_MODEL_NAME = "mock-stream";
export const BROWSER_PREVIEW_FALLBACK_REASON =
  "当前通过 npm run dev 打开的是浏览器预览，而不是 Tauri 桌面窗口，因此不会连接 Rust 后端。";
export const BROWSER_PREVIEW_SESSION_SUMMARY = "浏览器预览模式已启用，当前轮次未连接 Rust 后端。";
export const BROWSER_PREVIEW_TRACE_TITLE = "浏览器预览";
export const BROWSER_PREVIEW_CHUNKS = [
  "当前看到的不是前端资源没加载，而是页面运行在普通浏览器里。\n\n",
  "此时 @tauri-apps/api 不会注入原生桥接能力，所以直接调用 invoke/listen 会失败。\n\n",
  "现在已切换到浏览器预览兜底模式：\n",
  "- 可以继续预览 UI 和输入交互\n",
  "- 不会连接 Rust agent core\n",
  "- 真正联调需要运行 tauri dev\n"
];
