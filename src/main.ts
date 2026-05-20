import { createPinia } from "pinia";
import { createApp } from "vue";
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

app.use(createPinia());
app.mount("#app");
