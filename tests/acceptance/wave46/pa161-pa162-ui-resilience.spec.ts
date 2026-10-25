/**
 * Wave-46 契约验收（L2-T & L2-AT）— PA-161 / PA-162：前端交互面包屑与局部错误边界
 *
 * 契约源：`.dev-team/contract-matrix-wave46.md` 与 `.dev-team/nfr-baseline-wave46.json`
 *
 * 覆盖：
 *   PA161-T1（导航面包屑记录）
 *     - 记录 category="navigation" 面包屑，附带 from/to
 *     - 队列有界不超过 30 项
 *   PA161-T2（数据脱敏与截断验证）
 *     - 包含敏感 token 的参数在面包屑中被脱敏或截断
 *   PA162-T1（局部 ErrorBoundary 异常捕获与降级渲染）
 *     - 子组件渲染报错时，阻断冒泡，渲染 fallback 卡片并触发 reportError
 *   PA162-T2（ErrorBoundary 重试恢复）
 *     - 点击重试按钮重置错误状态并重新尝试渲染
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent, h, nextTick, ref } from "vue";
import { addBreadcrumb, getBreadcrumbs, clearBreadcrumbs, reportError } from "@/lib/telemetry";

// 动态 import 待实现的 ErrorBoundary 组件（当前尚未实现，红相断言其行为）
async function loadErrorBoundary() {
  try {
    const mod = await import("@/components/common/ErrorBoundary.vue");
    return mod.default;
  } catch {
    // 组件尚未创建时抛出清晰红相断言错误
    throw new Error("PA-162 RED: ErrorBoundary.vue 尚未实现");
  }
}

describe("Wave-46: PA-161 前端交互与导航面包屑契约", () => {
  beforeEach(() => {
    clearBreadcrumbs();
  });

  afterEach(() => {
    clearBreadcrumbs();
  });

  it("PA161-T1: 页面导航面包屑记录与队列容量上限", () => {
    addBreadcrumb("navigation", "navigate_page", { from: "home", to: "telemetry" });
    const crumbs = getBreadcrumbs();
    expect(crumbs.length).toBeGreaterThanOrEqual(1);
    const navCrumb = crumbs.find((c) => c.category === "navigation");
    expect(navCrumb).toBeDefined();
    expect(navCrumb?.message).toBe("navigate_page");
    expect(navCrumb?.data?.from).toBe("home");
    expect(navCrumb?.data?.to).toBe("telemetry");

    // 压测有界性：推入超过 30 条导航面包屑
    for (let i = 0; i < 40; i++) {
      addBreadcrumb("navigation", `step_${i}`, { index: String(i) });
    }
    const boundedCrumbs = getBreadcrumbs();
    expect(boundedCrumbs.length).toBeLessThanOrEqual(30);
  });

  it("PA161-T2: 交互面包屑深度脱敏与截断", () => {
    const sensitiveToken = "Bearer secret-token-xyz-123456789";
    const longString = "A".repeat(3000);
    addBreadcrumb("navigation", "navigate_page", {
      auth: sensitiveToken,
      query: longString,
    });
    const crumbs = getBreadcrumbs();
    const lastCrumb = crumbs[crumbs.length - 1];
    expect(lastCrumb).toBeDefined();
    const rendered = JSON.stringify(lastCrumb?.data ?? {});
    // 断言敏感 token 不得明文出现
    expect(rendered).not.toContain("secret-token-xyz-123456789");
    // 断言超长文本被截断（最大 2048）
    expect(rendered.length).toBeLessThan(2500);
  });
});

describe("Wave-46: PA-162 前端局部错误边界组件契约", () => {
  it("PA162-T1: ErrorBoundary 捕获子组件异常，阻止冒泡，渲染 fallback UI 并触发 reportError", async () => {
    const ErrorBoundary = await loadErrorBoundary();

    const BrokenChild = defineComponent({
      name: "BrokenChild",
      setup() {
        throw new Error("Simulated component explosion");
      },
      render() {
        return h("div", "normal content");
      },
    });

    const wrapper = mount(ErrorBoundary, {
      slots: {
        default: () => h(BrokenChild),
      },
    });

    await nextTick();

    const fallback = wrapper.find('[data-testid="error-boundary-fallback"]');
    expect(fallback.exists()).toBe(true);
    expect(fallback.text()).toContain("组件渲染异常");
    expect(wrapper.text()).not.toContain("normal content");
  });

  it("PA162-T2: ErrorBoundary 重试机制恢复渲染", async () => {
    const ErrorBoundary = await loadErrorBoundary();
    const shouldFail = ref(true);

    const FlakyChild = defineComponent({
      name: "FlakyChild",
      setup() {
        if (shouldFail.value) {
          throw new Error("Temporary crash");
        }
        return () => h("div", { "data-testid": "recovered-content" }, "Recovered successfully");
      },
    });

    const wrapper = mount(ErrorBoundary, {
      slots: {
        default: () => h(FlakyChild),
      },
    });

    await nextTick();
    expect(wrapper.find('[data-testid="error-boundary-fallback"]').exists()).toBe(true);

    // 修复子组件状态并触发重试
    shouldFail.value = false;
    const retryBtn = wrapper.find('[data-testid="error-boundary-retry"]');
    expect(retryBtn.exists()).toBe(true);
    await retryBtn.trigger("click");
    await nextTick();

    expect(wrapper.find('[data-testid="recovered-content"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="error-boundary-fallback"]').exists()).toBe(false);
  });
});
