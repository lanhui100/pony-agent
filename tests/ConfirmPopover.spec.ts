// ConfirmPopover 双模式行为测试：非受控向后兼容 / 受控异步确认纪律。
// reka-ui Portal 把内容挂到 document.body —— 统一用 attachTo + document 查询。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import ConfirmPopover from "@/components/ui/ConfirmPopover.vue";

enableAutoUnmount(afterEach);

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as typeof ResizeObserver
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function q(selector: string): HTMLElement | null {
  return document.body.querySelector(selector);
}

async function flush(times = 4): Promise<void> {
  for (let i = 0; i < times; i++) {
    // eslint-disable-next-line no-await-in-loop
    await Promise.resolve();
  }
}

async function openAndMountConfirm(
  template: string,
  options: { methods?: Record<string, unknown>; data?: () => unknown } = {}
) {
  const wrapper = mount(
    {
      components: { ConfirmPopover },
      template,
      methods: options.methods,
      data: options.data
    },
    { attachTo: document.body }
  );
  const trigger = wrapper.get('[data-testid="trigger"]');
  await trigger.trigger("click");
  await flush();
  return wrapper;
}

describe("ConfirmPopover（非受控·兼容模式）", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("confirm 点击即关闭并回传事件（历史行为不变）", async () => {
    const onConfirm = vi.fn();
    const wrapper = await openAndMountConfirm(
      `<ConfirmPopover title="删除" confirm-text="删除工作区" @confirm="onConfirm">
         <button data-testid="trigger">打开</button>
       </ConfirmPopover>`,
      { methods: { onConfirm } }
    );
    const confirm = q('[data-testid="confirm-popover-confirm"]');
    expect(confirm?.textContent).toBe("删除工作区");
    confirm?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(onConfirm).toHaveBeenCalledTimes(1);
    // 非受控模式下 PopoverClose 立即收起内容。
    expect(q('[data-testid="confirm-popover-confirm"]')).toBeNull();
    wrapper.unmount();
  });

  it("cancel 关闭且不触发 confirm", async () => {
    const onConfirm = vi.fn();
    const wrapper = await openAndMountConfirm(
      `<ConfirmPopover title="删除" @confirm="onConfirm">
         <button data-testid="trigger">打开</button>
       </ConfirmPopover>`,
      { methods: { onConfirm } }
    );
    q('[data-testid="confirm-popover-cancel"]')?.dispatchEvent(
      new MouseEvent("click", { bubbles: true })
    );
    await flush();
    expect(onConfirm).not.toHaveBeenCalled();
    expect(q('[data-testid="confirm-popover-confirm"]')).toBeNull();
    wrapper.unmount();
  });
});

describe("ConfirmPopover（受控·异步模式）", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("loading 时禁用双按钮、显示处理中文案；失败展示错误槽且可重试；confirm 不关闭", async () => {
    const onConfirm = vi.fn();
    const wrapper = await openAndMountConfirm(
      `<ConfirmPopover
         title="归档对话"
         confirm-text="归档对话"
         :open="true"
         :loading="loading"
         :error="error"
         @confirm="onConfirm"
         @update:open="onOpen"
       >
         <button data-testid="trigger">打开</button>
       </ConfirmPopover>`,
      {
        methods: { onConfirm, onOpen: () => {} },
        data: () => ({ loading: true, error: "" })
      }
    );
    const confirm = q('[data-testid="confirm-popover-confirm"]');
    expect(confirm?.getAttribute("disabled")).toBeDefined();
    expect(confirm?.textContent).toContain("处理中…");
    expect(q('[data-testid="confirm-popover-cancel"]')?.getAttribute("disabled")).toBeDefined();

    confirm?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(onConfirm).toHaveBeenCalledTimes(1);

    // 失败态：弹层未关闭，错误槽展示，按钮恢复可点（可重试）。
    await wrapper.setData({ loading: false, error: "操作失败：boom" });
    await flush();
    expect(q('[data-testid="confirm-popover-error"]')?.textContent).toContain("boom");
    const retry = q('[data-testid="confirm-popover-confirm"]');
    expect(retry?.getAttribute("disabled")).toBeNull();
    retry?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(onConfirm).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
});
