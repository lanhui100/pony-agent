import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import path from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src")
    }
  },
  server: {
    host: process.env.TAURI_DEV_HOST || "127.0.0.1",
    port: 4174,
    strictPort: true,
    watch: {
      ignored: [
        "**/.codex/**",
        "**/.codex-logs/**",
        "**/.git/**",
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
        "**/target-test*/**",
        "**/target-check*/**",
        "**/target*/**"
      ]
    }
  }
});
