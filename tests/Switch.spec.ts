import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import Switch from "@/components/ui/Switch.vue";

describe("Switch", () => {
  it("keeps the compact switch dimensions and checked accent color classes", () => {
    const wrapper = mount(Switch, {
      props: {
        modelValue: true
      }
    });

    const root = wrapper.get('[data-state="checked"]');
    const className = root.attributes("class") ?? "";

    expect(className).toContain("h-4");
    expect(className).toContain("w-7");
    expect(className).toContain("data-[state=checked]:border-[#8b5e34]/90");
    expect(className).toContain("data-[state=checked]:bg-[#8b5e34]");
    expect(className).toContain("focus-visible:ring-amber-300/70");
  });
});
