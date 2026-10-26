import { createPinia } from "pinia";
import { createApp } from "vue";
import { MotionPlugin } from "motion-v";
import App from "./App.vue";
import "./styles.css";
import { reportError, addBreadcrumb, isNetworkNoise } from "./lib/telemetry";
import { useRuntimeStore } from "@/stores/runtime";

const mountCount = Number(window.sessionStorage.getItem("pony-agent.mount-count") || "0") + 1;
window.sessionStorage.setItem("pony-agent.mount-count", String(mountCount));
console.info("[pony-agent][boot] mount app", {
  mountCount,
  href: window.location.href,
  ts: new Date().toISOString()
});

addBreadcrumb("app", "App boot start", {
  mountCount,
  href: window.location.href,
});

const app = createApp(App);

const pinia = createPinia();
app.use(pinia);
app.use(MotionPlugin);

function getActiveTraceId(): string | undefined {
  try {
    return useRuntimeStore().activeTraceId;
  } catch {
    return undefined;
  }
}

// 全局 Vue 组件渲染及生命周期错误捕获 (PonySentry A1)
app.config.errorHandler = (err: unknown, instance, info: string) => {
  console.error("[pony-agent][vue-error]", err, info);
  const errorObj = err instanceof Error ? err : new Error(String(err));
  const message = errorObj.message || String(err);
  if (isNetworkNoise(message)) return;
  reportError({
    errorType: errorObj.name || "VueComponentError",
    message,
    stack: errorObj.stack,
    trace_id: getActiveTraceId(),
    extra: {
      lifecycleHook: info,
      componentTag: instance?.$options?.name || "AnonymousComponent",
    },
    tags: {
      source: "vue_error_handler",
    },
  });
};

// 全局未处理 Promise 拒绝捕获 (PonySentry A1)
if (typeof window !== "undefined") {
  window.addEventListener("unhandledrejection", (event) => {
    console.error("[pony-agent][unhandled-rejection]", event.reason);
    const reason = event.reason;
    const errorObj = reason instanceof Error ? reason : new Error(String(reason));
    const message = errorObj.message || String(reason);
    if (isNetworkNoise(message)) return;
    reportError({
      errorType: errorObj.name || "UnhandledRejection",
      message,
      stack: errorObj.stack,
      trace_id: getActiveTraceId(),
      extra: {
        reasonString: String(reason),
      },
      tags: {
        source: "window_unhandled_rejection",
      },
    });
  });

  // 全局普通 JS 运行时错误捕获 (PonySentry A1)
  window.addEventListener("error", (event) => {
    // 忽略加载资源失败引发的无 error 对象的事件
    if (!event.error && !event.message) return;
    const errorObj =
      event.error instanceof Error ? event.error : new Error(String(event.message));
    const message = errorObj.message || event.message || "Unknown window error";
    if (isNetworkNoise(message, event.filename)) return;
    console.error("[pony-agent][window-error]", event.error || event.message);
    reportError({
      errorType: errorObj.name || "WindowRuntimeError",
      message,
      stack: errorObj.stack,
      trace_id: getActiveTraceId(),
      extra: {
        filename: event.filename,
        lineno: event.lineno,
        colno: event.colno,
      },
      tags: {
        source: "window_error",
      },
    });
  });
}

app.mount("#app");

// 优雅淡出 index.html 预加载启动屏（保留 3s 品牌展示，结束后立即解绑事件拦截防遮挡）
if (typeof window !== "undefined") {
  setTimeout(() => {
    const splash = document.getElementById("app-loading-screen");
    if (splash) {
      splash.style.pointerEvents = "none";
      splash.classList.add("fade-out");
      setTimeout(() => {
        splash.remove();
      }, 350);
    }
  }, 3000);
}

if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as Record<string, unknown>).__ponyaPinia = pinia;
}
