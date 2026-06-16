import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import path from "node:path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src")
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
