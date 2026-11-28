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
        "src/stores/runtime.ts",
        // B1795831357716（2026-11-28）：runtime.ts 拆成 12 个子模块后只剩 78 行装配壳，
        // 真正的状态机搬到了 src/stores/runtime/**（实测 7176 行）。上面那条 `runtime.ts`
        // 单文件 include 因此变成「只统计壳、放过主体」——覆盖率门禁在拆解当天就失效了。
        // 这里按目录纳管主体；__tests__ 与 fixtures 是测试自身，统计它们等于用被测代码给
        // 被测代码打分（自我抬分），故排除。
        "src/stores/runtime/**/*.ts"
      ],
      exclude: [
        "src/stores/runtime/__tests__/**",
        "src/stores/runtime/fixtures/**",
        "src/stores/runtime/types.ts"
      ],
      // W56-T⑤：阈值对齐**实际达成值**（`npm run test:coverage`），取向下取整作为门禁基线
      // （不设 flaky 边距、不通过删断言/skip 抬分）。提升覆盖率时应同步上调这里的数字。
      //
      // B1795831357716（2026-11-28）重锚：runtime.ts 拆解后 include 曾只统计 78 行装配壳，
      // 主体 src/stores/runtime/**（7176 行）整个放过。扩面后全量实测：
      //   statements 87.54 / branches 79.90 / functions 87.37 / lines 88.11
      // 旧阈值 82/73/82/82 是按「只算壳 + 三个 UI 文件」定的，对扩面后的口径已无约束力；
      // 这里按新口径重锚到 87/79/87/88。
      //
      // 注意：threshold只对**全量** run 成立。子集 run（如 test:ui-guard 只跑 3 个 spec）
      // 天然覆盖不到 runtime 主体，故它用自己的 include + 阈值（见 package.json 该脚本），
      // 两者口径不同、互不套用。
      thresholds: {
        lines: 88,
        functions: 87,
        statements: 87,
        branches: 79
      }
    }
  }
});
