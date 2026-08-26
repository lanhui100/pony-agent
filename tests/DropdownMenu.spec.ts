// DropdownMenu 基件行为测试：项渲染 / 选择回传 / 禁用与 danger 语义。
// reka-ui Portal 把内容挂到 document.body —— 统一用 attachTo + document 查询。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { Trash2 } from "lucide-vue-next";
import DropdownMenu from "@/components/ui/DropdownMenu.vue";
import type { DropdownMenuItemSpec } from "@/components/ui/DropdownMenu.vue";

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

const items: DropdownMenuItemSpec[] = [
  { id: "rename", label: "重命名" },
  {
    id: "delete",
    label: "删除工作区",
    icon: Trash2,
    danger: true,
    disabled: true,
    disabledTitle: "对话运行中，暂不能执行该操作"
  }
];

function q(selector: string): HTMLElement | null {
  return document.body.querySelector(selector);
}

async function flush(times = 4): Promise<void> {
  for (let i = 0; i < times; i++) {
    // eslint-disable-next-line no-await-in-loop
    await Promise.resolve();
  }
}

describe("DropdownMenu", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("打开后渲染全部条目；danger 项带图标", async () => {
    const wrapper = mount(
      {
        components: { DropdownMenu },
        template: `
          <DropdownMenu :items="items">
            <button data-testid="trigger">⋯</button>
          </DropdownMenu>
        `,
        data: () => ({ items })
      },
      { attachTo: document.body }
    );
    await wrapper.get('[data-testid="trigger"]').trigger("click");
    await flush();

    const rename = q('[data-testid="dropdown-item-rename"]');
    expect(rename?.textContent).toBe("重命名");
    const del = q('[data-testid="dropdown-item-delete"]');
    expect(del?.textContent).toBe("删除工作区");
    expect(del?.querySelector("svg")).not.toBeNull();
    wrapper.unmount();
  });

  it("可用项点击回传 id", async () => {
    const onSelect = vi.fn();
    const wrapper = mount(
      {
        components: { DropdownMenu },
        template: `
          <DropdownMenu :items="items" @select="onSelect">
            <button data-testid="trigger">⋯</button>
          </DropdownMenu>
        `,
        data: () => ({ items }),
        methods: { onSelect }
      },
      { attachTo: document.body }
    );
    await wrapper.get('[data-testid="trigger"]').trigger("click");
    await flush();
    // reka 菜单项响应 click（keyboard select 同通道）。
    q('[data-testid="dropdown-item-rename"]')?.dispatchEvent(
      new MouseEvent("click", { bubbles: true })
    );
    await flush();
    expect(onSelect).toHaveBeenCalledWith("rename");
    wrapper.unmount();
  });

  it("禁用项不可点且带原因 tooltip", async () => {
    const onSelect = vi.fn();
    const wrapper = mount(
      {
        components: { DropdownMenu },
        template: `
          <DropdownMenu :items="items" @select="onSelect">
            <button data-testid="trigger">⋯</button>
          </DropdownMenu>
        `,
        data: () => ({ items }),
        methods: { onSelect }
      },
      { attachTo: document.body }
    );
    await wrapper.get('[data-testid="trigger"]').trigger("click");
    await flush();
    const del = q('[data-testid="dropdown-item-delete"]');
    expect(del?.getAttribute("aria-disabled")).toBe("true");
    expect(del?.getAttribute("title")).toBe("对话运行中，暂不能执行该操作");
    del?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flush();
    expect(onSelect).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("danger 组首项被禁用时其前分隔线仍然存在（两态视觉解剖一致）", async () => {
    // 回归钉板：分隔线条件不得依赖 !disabled——运行中会话菜单
    // [重命名(可用), 归档(禁用,danger), 删除(禁用,danger)] 恰是常见形态。
    const runtimeItems: DropdownMenuItemSpec[] = [
      { id: "rename", label: "重命名" },
      { id: "archive", label: "归档对话", danger: true, disabled: true },
      { id: "delete", label: "删除对话", danger: true, disabled: true }
    ];
    const wrapper = mount(
      {
        components: { DropdownMenu },
        template: `
          <DropdownMenu :items="runtimeItems">
            <button data-testid="trigger">⋯</button>
          </DropdownMenu>
        `,
        data: () => ({ runtimeItems })
      },
      { attachTo: document.body }
    );
    await wrapper.get('[data-testid="trigger"]').trigger("click");
    await flush();
    expect(document.body.querySelector(".h-px")).not.toBeNull();
    wrapper.unmount();
  });
});
