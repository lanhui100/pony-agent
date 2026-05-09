import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  plugins: [vue()],
  server: {
    host: process.env.TAURI_DEV_HOST || "127.0.0.1",
    port: 5173,
    strictPort: true
  }
});
