import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// PA-096：改用 import.meta.url 推导目录，兼容 vite `--configLoader native`
// （进程内加载，沙箱环境无需 spawn 子进程）与默认 bundle 两种 loader。
const configDirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(configDirname, "./src")
    }
  },
  test: {
    environment: "jsdom",
    include: ["tests/**/*.spec.ts"],
    exclude: ["tests/e2e/**"],
    outputFile: {
      json: "test-results/vitest/results.json"
    },
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary"],
      reportsDirectory: "coverage/vitest",
      include: [
        "src/App.vue",
        "src/components/HomeWorkspace.vue",
        "src/components/HomeSessionSidebar.vue"
      ],
      thresholds: {
        lines: 80,
        functions: 80,
        statements: 80,
        branches: 80
      }
    }
  }
});
