import { createPinia } from "pinia";
import { createApp } from "vue";
import { MotionPlugin } from "motion-v";
import App from "./App.vue";
import "./styles.css";

const mountCount = Number(window.sessionStorage.getItem("pony-agent.mount-count") || "0") + 1;
window.sessionStorage.setItem("pony-agent.mount-count", String(mountCount));
console.info("[pony-agent][boot] mount app", {
  mountCount,
  href: window.location.href,
  ts: new Date().toISOString()
});

const app = createApp(App);

const pinia = createPinia();
app.use(pinia);
app.use(MotionPlugin);
app.mount("#app");

// 优雅淡出 index.html 预加载启动屏
if (typeof window !== "undefined") {
  requestAnimationFrame(() => {
    const splash = document.getElementById("app-loading-screen");
    if (splash) {
      splash.classList.add("fade-out");
      setTimeout(() => {
        splash.remove();
      }, 400);
    }
  });
}

if (import.meta.env.DEV && typeof window !== "undefined") {
  (window as unknown as Record<string, unknown>).__ponyaPinia = pinia;
}
