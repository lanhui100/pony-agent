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
    include: [
      "tests/**/*.spec.ts",
      "tests/**/*.test.ts",
      "src/**/__tests__/**/*.spec.ts"
    ],
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
        "src/components/HomeSessionSidebar.vue",
        // W56-T⑤：运行时 store 是本产品最大的状态机（会话隔离/流事件/授权透传都在这里），
        // 必须纳入覆盖率门禁，否则它的回归只能靠"用例自觉"。
        "src/stores/runtime.ts"
      ],
      // W56-T⑤：阈值对齐**实际达成值**（runtime.ts 纳入后实测，`npx vitest run --coverage`）：
      //   statements 82.09 / branches 73.06 / functions 82.34 / lines 82.54
      // 取向下取整作为门禁基线（不设 flaky 边距、不通过删断言/skip 抬分）。
      // 提升覆盖率时应同步上调这里的数字。
      thresholds: {
        lines: 82,
        functions: 82,
        statements: 82,
        branches: 73
      }
    }
  }
});
