import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// PA-096：import.meta.url 推导目录，兼容 vite `--configLoader native`
// （进程内加载配置）与默认 bundle 两种 loader。
const configDirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  clearScreen: false,
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(configDirname, "./src")
    }
  },
  server: {
    host: process.env.TAURI_DEV_HOST || "127.0.0.1",
    port: 4176,
    strictPort: true,
    watch: {
      ignored: [
        "**/.codex/**",
        "**/.codex-logs/**",
        "**/.cargo-target*/**",
        "**/.git/**",
        "**/.omo/**",
        "**/.tmp/**",
        "**/.devlogs/**",
        "**/claude-code-sourcemap/**",
        "**/codex-openai/**",
        "**/coverage/**",
        "**/dist/**",
        "**/docs/**",
        "**/hermes/**",
        "**/management/**",
        "**/node_modules/**",
        "**/openspec/**",
        "**/reasonix-esengine/**",
        "**/sessions/**",
        "**/src-tauri/gen/**",
        "**/src-tauri/target/**",
        "**/src-tauri/target-check/**",
        "**/test-results/**",
        "**/target-test*/**",
        "**/target-check*/**",
        "**/target*/**",
        "**/*.log"
      ]
    }
  }
});
