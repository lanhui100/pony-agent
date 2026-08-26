import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { defineComponent } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import ProviderConfigPage from "@/components/ProviderConfigPage.vue";
import { createDefaultCapabilities, useProviderStore } from "@/stores/providers";
import type { ProviderConfig, ProviderModelConfig, ProviderRegistry } from "@/types/provider";

// ADR 0013：模型页双一级折叠区（提供商详情 / 模型列表）行为契约测试。
// fixture 直接 $patch registry（组件 watch immediate 初始化要求先于 mount 就绪）；
// tauri mock 关闭（isTauriAvailable→false）后保存走浏览器分支，不触网。
// 折叠体为平滑动画容器（grid-rows），内容常挂载——折叠态以 data-open 断言。

const tauriMocks = vi.hoisted(() => ({
  mockSafeInvoke: vi.fn(),
  mockSafeListen: vi.fn(),
  mockIsTauriAvailable: vi.fn()
}));

vi.mock("@/lib/tauri", () => ({
  safeInvoke: tauriMocks.mockSafeInvoke,
  safeListen: tauriMocks.mockSafeListen,
  isTauriAvailable: tauriMocks.mockIsTauriAvailable
}));

const ScrollAreaStub = defineComponent({
  template: '<div class="scroll-area-stub"><slot /></div>'
});

const TooltipStub = defineComponent({
  props: {
    text: {
      type: String,
      default: ""
    }
  },
  template: '<span class="tooltip-stub" :data-text="text"><slot /></span>'
});

const ConfirmPopoverStub = defineComponent({
  emits: ["confirm"],
  template:
    '<span class="confirm-popover-stub"><slot />' +
    '<button type="button" data-testid="confirm-popover-confirm" @click="$emit(\'confirm\')">确认删除</button></span>'
});

function createModel(partial: Partial<ProviderModelConfig> & Pick<ProviderModelConfig, "id" | "name" | "model">): ProviderModelConfig {
  return {
    protocol: "openai-completions",
    capabilityPreset: "custom",
    capabilities: { ...createDefaultCapabilities() },
    temperature: 0,
    maxOutputTokens: 64000,
    reasoningEffort: null,
    reasoningBudgetTokens: null,
    ...partial
  };
}

function createProviderFixture(partial: Partial<ProviderConfig> & Pick<ProviderConfig, "id" | "name">): ProviderConfig {
  return {
    protocol: "openai-completions",
    baseUrl: "https://example.invalid/v1",
    authType: "auto",
    supportedProtocols: ["openai-completions"],
    endpoints: [
      { protocol: "openai-completions", enabled: true, baseUrl: "https://example.invalid/v1", authType: "auto" }
    ],
    apiKeyEnvVar: "EXAMPLE_API_KEY",
    apiKeyValue: "",
    apiKeyPresent: false,
    models: [],
    selectedModelId: null,
    ...partial
  };
}

function seedRegistry() {
  const providerStore = useProviderStore();
  const alpha: ProviderConfig = createProviderFixture({
    id: "provider-alpha",
    name: "Alpha",
    models: [
      createModel({ id: "model-a1", name: "Alpha One", model: "alpha-1" }),
      createModel({ id: "model-a2", name: "Alpha Two", model: "alpha-2" })
    ],
    selectedModelId: "model-a1"
  });
  const beta: ProviderConfig = createProviderFixture({
    id: "provider-beta",
    name: "Beta",
    models: [createModel({ id: "model-b1", name: "Beta Solo", model: "beta-1" })]
  });
  const registry: ProviderRegistry = {
    providers: [alpha, beta],
    selectedProviderId: "provider-alpha"
  };
  providerStore.$patch({ registry });
  return { providerStore, registry };
}

async function mountProviderPage() {
  const wrapper = mount(ProviderConfigPage, {
    global: {
      stubs: {
        ScrollArea: ScrollAreaStub,
        ConfirmPopover: ConfirmPopoverStub,
        Tooltip: TooltipStub
      }
    }
  });
  await Promise.resolve();
  await Promise.resolve();
  return wrapper;
}

describe("ProviderConfigPage hierarchical layout (ADR 0013)", () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    setActivePinia(createPinia());
    tauriMocks.mockIsTauriAvailable.mockReturnValue(false);
    tauriMocks.mockSafeInvoke.mockResolvedValue(null);
    tauriMocks.mockSafeListen.mockResolvedValue(() => {});
    vi.spyOn(console, "info").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders a flat selectable provider list without accordion nesting", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    // 左列扁平行：全部提供商同时可见，无需展开
    expect(wrapper.find('[data-testid="provider-list-item-provider-alpha"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="provider-list-item-provider-beta"]').exists()).toBe(true);
    // 左列不再有手风琴的嵌套"提供商详情"入口（已上移为右侧一级折叠区标题）
    const aside = wrapper.get("aside");
    expect(aside.text()).not.toContain("提供商详情");
    expect(aside.find('[data-testid="provider-detail-toggle"]').exists()).toBe(false);

    // 点击即选中，右侧页头跟随
    await wrapper.get('[data-testid="provider-list-item-provider-beta"]').trigger("click");
    expect(wrapper.get("section h2").text()).toBe("Beta");
    expect(
      wrapper.get('[data-testid="provider-list-item-provider-beta"]').classes().join(" ")
    ).toContain("bg-white/78");
  });

  it("behaves as an accordion: at most one first-level section is open", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    const providerSection = wrapper.get('[data-testid="provider-detail-section"]');
    const modelSection = wrapper.get('[data-testid="model-list-section"]');
    expect(providerSection.text()).toContain("提供商详情");
    expect(modelSection.text()).toContain("模型列表");

    // 折叠开关为标题左侧纯图标（整行均为 trigger），默认手风琴展开提供商详情；
    // 迭代四：一级区头 hover 仅 pointer，不加背景色
    const providerHeader = wrapper.get('[data-testid="provider-detail-header"]');
    const providerToggle = wrapper.get('[data-testid="provider-detail-toggle"]');
    const modelToggle = wrapper.get('[data-testid="model-list-toggle"]');
    expect(providerHeader.classes().join(" ")).toContain("cursor-pointer");
    expect(providerHeader.classes().join(" ")).not.toContain("hover:bg");
    expect(providerToggle.attributes("aria-expanded")).toBe("true");
    expect(providerToggle.attributes("aria-controls")).toBe("provider-detail-body");

    // 迭代三：手风琴默认态——提供商详情展开、模型列表收起
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("false");

    // 展开模型列表 → 提供商详情收起
    await modelToggle.trigger("click");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("false");

    // 反向互斥
    await providerToggle.trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("false");

    // 点击已展开项允许双收起，再点恢复
    await providerToggle.trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("false");
    await providerToggle.trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
  });

  it("toggles a model row from anywhere in its row while tail actions keep their own handling", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    const row = wrapper.get('[data-testid="model-list-item-model-a1"]');
    // 整行 trigger：hover 背景类包裹整行（含行尾动作），行容器带 pointer 语义
    expect(row.classes().join(" ")).toContain("hover:bg-white/74");
    expect(row.classes().join(" ")).toContain("cursor-pointer");

    // 行尾动作区 @click.stop：编辑走编辑态而非折叠
    await wrapper.get('[data-testid="model-row-edit-model-a1"]').trigger("click");
    const detail = wrapper.get('[data-testid="model-detail-model-a1"]');
    expect(detail.attributes("data-open")).toBe("true");
    expect(detail.text()).toContain("模型 ID");

    // 取消编辑回落到该行的视图详情（既有语义：不收起）
    await wrapper.get('[data-testid="model-edit-cancel-model-a1"]').trigger("click");
    expect(detail.attributes("data-open")).toBe("true");
    expect(detail.text()).toContain("模型信息");

    // chevron 为纯指示器（非按钮），aria-expanded/aria-controls 挂在行内 disclosure 上
    expect(wrapper.find('[data-testid="model-row-chevron-model-a1"]').exists()).toBe(false);
    const disclosure = row.find("button");
    expect(disclosure.attributes("aria-expanded")).toBe("true");
    expect(disclosure.attributes("aria-controls")).toBe("model-detail-model-a1");

    await row.trigger("click");
    expect(wrapper.get('[data-testid="model-detail-model-a1"]').attributes("data-open")).toBe("false");
  });

  it("expands a model row into its config details and collapses back to provider view", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-list-item-model-a1"]').trigger("click");

    const detail = wrapper.get('[data-testid="model-detail-model-a1"]');
    expect(detail.attributes("data-open")).toBe("true");
    expect(detail.text()).toContain("模型信息");
    expect(detail.text()).toContain("Alpha One");
    expect(detail.text()).toContain("alpha-1");
    expect(detail.text()).toContain("模型能力");
    expect(detail.text()).toContain("模型参数");
    // ADR 0013 迭代：已在提供商上下文内，不重复"所属提供商"；参数以 K/M 单位呈现
    expect(detail.text()).not.toContain("所属提供商");
    expect(detail.text()).not.toContain("tokens");
    expect(detail.text()).toContain("256K");
    expect(detail.text()).toContain("64K");
    // 能力 chip 仅图标，名称经 tooltip（stub 的 data-text）呈现
    const chipTexts = detail.findAll(".tooltip-stub").map((tip) => tip.attributes("data-text"));
    expect(chipTexts).toContain("图片");
    expect(chipTexts).toContain("思考模型");
    // 行 disclosure（行内首个按钮）与详情区 aria-controls 配对
    const row = wrapper.get('[data-testid="model-list-item-model-a1"]');
    expect(row.find("button").attributes("aria-controls")).toBe("model-detail-model-a1");

    // 再次点击收起并回落提供商视图；其余行保持折叠
    await wrapper.get('[data-testid="model-list-item-model-a1"]').trigger("click");
    expect(wrapper.get('[data-testid="model-detail-model-a1"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="model-detail-model-a2"]').attributes("data-open")).toBe("false");
    expect(wrapper.get("section h2").text()).toBe("Alpha");
  });

  it("keeps the provider section in view mode while a model row is being edited (P1-1 regression)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-row-edit-model-a1"]').trigger("click");

    // 行尾编辑直接进入该行的编辑态（自动展开）
    expect(wrapper.get('[data-testid="model-detail-model-a1"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="model-detail-model-a1"]').text()).toContain("模型 ID");
    // 提供商 section 保持只读视图，不得泄漏 providerForm 编辑表单
    const providerSection = wrapper.get('[data-testid="provider-detail-section"]');
    expect(providerSection.text()).toContain("基础信息");
    expect(providerSection.find('input[placeholder="例如：OpenRouter"]').exists()).toBe(false);
  });

  it("re-expands the provider section when entering edit or create while folded (P1-2 regression)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("false");

    await wrapper.get('[data-testid="provider-edit-open"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
    expect(
      wrapper.get('[data-testid="provider-detail-body"]').find('input[placeholder="例如：OpenRouter"]').exists()
    ).toBe(true);
    await wrapper.get('[data-testid="provider-edit-cancel"]').trigger("click");

    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    await wrapper.get('[data-testid="provider-create-open"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
  });

  it("collapses away the create card and discards the draft instead of locking (D8)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    expect(wrapper.get('[data-testid="model-create-card"]').exists()).toBe(true);

    // D8：创建卡打开时收起模型列表区 = 放弃草稿，直接折叠
    await wrapper.get('[data-testid="model-list-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("false");
    expect(wrapper.find('[data-testid="model-create-card"]').exists()).toBe(false);

    // 手风琴连带：展开提供商区同样取消创建卡
    await wrapper.get('[data-testid="model-list-toggle"]').trigger("click");
    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    expect(wrapper.get('[data-testid="model-create-card"]').exists()).toBe(true);
    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("false");
    expect(wrapper.find('[data-testid="model-create-card"]').exists()).toBe(false);
  });

  it("keeps idle tail actions hover-revealed and trigger rows pointer-cursored (D9)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    // 一级区头与行内 trigger 均显式 cursor-pointer（原生按钮默认光标会覆盖容器）
    expect(wrapper.get('[data-testid="provider-detail-toggle"]').classes().join(" ")).toContain("cursor-pointer");
    expect(wrapper.get('[data-testid="model-list-toggle"]').classes().join(" ")).toContain("cursor-pointer");
    const row = wrapper.get('[data-testid="model-list-item-model-a1"]');
    expect(row.classes().join(" ")).toContain("group");
    expect(row.find("button").classes().join(" ")).toContain("cursor-pointer");

    // 空闲态动作簇默认隐藏、hover/focus-within 显现
    const rowActions = row.get(".group > div");
    const actionClasses = rowActions.classes().join(" ");
    expect(actionClasses).toContain("opacity-0");
    expect(actionClasses).toContain("invisible");
    expect(actionClasses).toContain("group-hover:opacity-100");
    expect(actionClasses).toContain("focus-within:visible");
  });

  it("cross-collapse cancels the model edit when the provider section opens (D8)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    // 进入模型 a1 的编辑态
    await wrapper.get('[data-testid="model-row-edit-model-a1"]').trigger("click");
    const detail = wrapper.get('[data-testid="model-detail-model-a1"]');
    expect(detail.attributes("data-open")).toBe("true");
    expect(detail.text()).toContain("模型 ID");

    // 手风琴连带收起模型区 → 先取消编辑（回视图态），不再有隐藏表单
    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("true");
    expect(wrapper.get('[data-testid="model-list-body"]').attributes("data-open")).toBe("false");
    expect(wrapper.find('[data-testid="model-edit-save-model-a1"]').exists()).toBe(false);
  });

  it("collapsing a create-provider form discards and resets the draft (D8)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="provider-create-open"]').trigger("click");
    await wrapper.get('input[placeholder="例如：OpenRouter"]').setValue("Draft Provider");

    // 收起 = 放弃草稿
    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("false");

    // 重新进入创建态为空白表单
    await wrapper.get('[data-testid="provider-create-open"]').trigger("click");
    const nameInput = wrapper.get('input[placeholder="例如：OpenRouter"]');
    expect((nameInput.element as HTMLInputElement).value).toBe("");
  });

  it("renders header and row actions as weakened icon-only buttons with tooltips", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    // 头部：编辑 / 新增模型 —— 纯图标、缩小弱化（h-7 w-7、stone-400、透明底）
    for (const testid of ["provider-edit-open", "model-create-open"]) {
      const button = wrapper.get(`[data-testid="${testid}"]`);
      expect(button.text().trim()).toBe("");
      const classes = button.classes().join(" ");
      expect(classes).toContain("h-7");
      expect(classes).toContain("w-7");
      expect(classes).toContain("text-stone-400");
      expect(classes).toContain("hover:bg-[#f7e3bf]");
      const tip = button.element.closest(".tooltip-stub") as HTMLElement;
      expect(tip.getAttribute("data-text")).toBeTruthy();
    }

    // 行尾：编辑 / 删除同样纯图标弱化，且不再独占一行（位于行头 flex 内）
    const rowEdit = wrapper.get('[data-testid="model-row-edit-model-a1"]');
    expect(rowEdit.text().trim()).toBe("");
    expect(rowEdit.classes().join(" ")).toContain("text-stone-400");
    const rowDeleteTip = (rowEdit.element.closest(".flex") as HTMLElement).querySelector(
      '[data-testid="model-row-delete-model-a1"]'
    );
    expect(rowDeleteTip).not.toBeNull();
  });

  it("deletes a model directly from its row tail and falls back to provider view when it was active", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-list-item-model-a1"]').trigger("click");
    await wrapper.get('[data-testid="model-row-delete-model-a1"]').trigger("click");

    // 行尾删除的确认弹层（stub）在模型列表 section 内，与提供商删除互不混淆
    const confirmButton = wrapper
      .get('[data-testid="model-list-section"]')
      .get('[data-testid="confirm-popover-confirm"]');
    await confirmButton.trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    expect(store.notice).toBe("模型已删除。");
    expect(store.registry?.providers[0].models.map((model) => model.id)).toEqual(["model-a2"]);
    // 被删的是展开中的活跃模型 → 回落提供商视图
    expect(wrapper.get("section h2").text()).toBe("Alpha");
  });

  it("opens the create-model card inside the list section and saves through the browser branch", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    expect(wrapper.get('[data-testid="model-create-card"]').exists()).toBe(true);

    const card = wrapper.get('[data-testid="model-create-card"]');
    await card.get('input[placeholder="例如：Claude Sonnet 4"]').setValue("New Model");
    await card.get('input[placeholder="手动填入，或点刷新从目录选择"]').setValue("new-model-id");

    await card.get('[data-testid="model-create-save"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    expect(store.notice).toBe("模型已新增。");
    expect(store.registry?.providers[0].models.some((model) => model.model === "new-model-id")).toBe(true);
  });

  it("hides the model list section while creating a provider", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="provider-create-open"]').trigger("click");

    expect(wrapper.get("section h2").text()).toBe("新增提供商");
    expect(wrapper.find('[data-testid="model-list-section"]').exists()).toBe(false);
    // 提供商创建表单在提供商详情折叠区内渲染
    expect(
      wrapper.get('[data-testid="provider-detail-body"]').find('input[placeholder="例如：OpenRouter"]').exists()
    ).toBe(true);
  });

  it("edits the provider inline; collapsing the section cancels the edit back to view (D8)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="provider-edit-open"]').trigger("click");

    const body = wrapper.get('[data-testid="provider-detail-body"]');
    const nameInput = body.get('input[placeholder="例如：OpenRouter"]');
    expect((nameInput.element as HTMLInputElement).value).toBe("Alpha");
    await nameInput.setValue("Changed");

    // D8：编辑态收起 = 放弃未保存修改并回落视图
    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').attributes("data-open")).toBe("false");
    expect(wrapper.get('[data-testid="provider-detail-body"]').text()).toContain("基础信息");

    // 原始数据未被污染
    await wrapper.get('[data-testid="provider-detail-toggle"]').trigger("click");
    expect(wrapper.get('[data-testid="provider-detail-body"]').text()).toContain("Alpha");
    expect(
      wrapper.get('[data-testid="provider-detail-body"]').find('input[placeholder="例如：OpenRouter"]').exists()
    ).toBe(false);
  });

  it("renders protocol badges on one row with name and API key, and saves canonical endpoints (D5)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="provider-edit-open"]').trigger("click");
    const body = wrapper.get('[data-testid="provider-detail-body"]');

    // 三徽章默认态：仅 openai-completions 启用（fixture 的旧值已规范化）
    for (const protocol of ["openai-responses", "openai-completions", "anthropic-messages"]) {
      const badge = body.get(`[data-testid="provider-protocol-badge-${protocol}"]`);
      expect(badge.attributes("aria-pressed")).toBe(protocol === "openai-completions" ? "true" : "false");
      expect(badge.text().trim()).toBe(protocol);
    }

    // 关闭唯一启用的 completions 被拒绝（至少保留一个）
    await body.get('[data-testid="provider-protocol-badge-openai-completions"]').trigger("click");
    expect(body.get('[data-testid="provider-protocol-badge-openai-completions"]').attributes("aria-pressed")).toBe("true");

    // 开启 anthropic-messages 后可关闭 completions → 出现模型引用警示（Alpha 两模型都在 completions 上）
    await body.get('[data-testid="provider-protocol-badge-anthropic-messages"]').trigger("click");
    await body.get('[data-testid="provider-advanced-toggle"]').trigger("click");
    expect(body.get('[data-testid="provider-baseurl-anthropic-messages"]').exists()).toBe(true);
    await body.get('[data-testid="provider-protocol-badge-openai-completions"]').trigger("click");
    expect(body.get('[data-testid="provider-badge-off-warning"]').text()).toContain("2 个模型");

    // 保存按钮位于区头动作簇（编辑态常显），不在 body 内
    await wrapper.get('[data-testid="provider-save"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    const alpha = store.registry?.providers.find((provider) => provider.id === "provider-alpha");
    expect(alpha?.supportedProtocols).toContain("anthropic-messages");
    expect(alpha?.endpoints.find((endpoint) => endpoint.protocol === "anthropic-messages")?.enabled).toBe(true);
  });

  it("adds models in batch from the fetched catalog and reports skipped duplicates (D6)", async () => {
    seedRegistry();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "fetch_provider_models") {
        return ["m1", "m2", "m1"];
      }
      if (command === "save_provider_registry") {
        return (args?.registry as unknown) ?? null;
      }
      return null;
    });

    const wrapper = await mountProviderPage();
    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    const card = wrapper.get('[data-testid="model-create-card"]');

    await card.get('[data-testid="model-catalog-fetch"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const panel = card.get('[data-testid="model-catalog-panel"]');
    await panel.get('[data-testid="model-catalog-option-m1"]').setValue(true);
    await panel.get('[data-testid="model-catalog-option-m2"]').setValue(true);

    await panel.get('[data-testid="model-catalog-add-selected"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    expect(store.notice).toBe("已添加 2 个模型。");
    const modelValues = store.registry?.providers[0].models.map((model) => model.model) ?? [];
    expect(modelValues).toContain("m1");
    expect(modelValues).toContain("m2");
    // 批量添加使用高级设置默认协议 openai-completions
    const added = store.registry?.providers[0].models.find((model) => model.model === "m1");
    expect(added?.protocol).toBe("openai-completions");
  });

  it("shows info tooltip on the selector and swaps to failure semantics when fetch errors (迭代五)", async () => {
    seedRegistry();
    tauriMocks.mockIsTauriAvailable.mockReturnValue(true);
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    const card = wrapper.get('[data-testid="model-create-card"]');

    // 初始：info 图标 + 说明 tooltip（提示右端刷新按钮与批量用法）
    const info = card.get('[data-testid="model-catalog-info"]');
    expect((info.element.closest(".tooltip-stub") as HTMLElement).getAttribute("data-text")).toContain(
      "刷新按钮"
    );

    // 成功：面板自动展开，info 保持说明语义
    tauriMocks.mockSafeInvoke.mockImplementation(async (command: string) =>
      command === "fetch_provider_models" ? ["m1", "m2"] : null,
    );
    await card.get('[data-testid="model-catalog-fetch"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();
    expect(card.get('[data-testid="model-catalog-panel"]').exists()).toBe(true);
    expect(card.get('[data-testid="model-catalog-info"]').exists()).toBe(true);

    // 失败：info 换为失败语义图标，tooltip 变为错误信息，面板收起
    tauriMocks.mockSafeInvoke.mockRejectedValueOnce(new Error("401 unauthorized"));
    await card.get('[data-testid="model-catalog-fetch"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const failureInfo = card.get('[data-testid="model-catalog-info-error"]');
    expect(failureInfo.classes().join(" ")).toContain("text-rose-500");
    expect(
      (failureInfo.element.closest(".tooltip-stub") as HTMLElement).getAttribute("data-text")
    ).toContain("拉取模型列表失败");
    expect(card.find('[data-testid="model-catalog-panel"]').exists()).toBe(false);
  });

  it("limits advanced protocol options to enabled provider protocols and binds base-url override (D6/P0-1)", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-create-open"]').trigger("click");
    const card = wrapper.get('[data-testid="model-create-card"]');

    await card.get('[data-testid="model-advanced-toggle-create"]').trigger("click");
    const select = card.get('[data-testid="model-advanced-protocol-create"]');
    const options = select.findAll("option");
    // fixture Alpha 仅启用 openai（规范化为 openai-completions）→ 下拉只含该值
    expect(options.map((option) => option.element.value)).toEqual(["openai-completions"]);

    const override = card.get('[data-testid="model-advanced-baseurl-create"]');
    expect((override.element as HTMLInputElement).placeholder).toBe("https://example.invalid/v1");
    await override.setValue("https://mirror.example.invalid/v1");

    await card.get('input[placeholder="手动填入，或点刷新从目录选择"]').setValue("override-model");
    await card.get('input[placeholder="例如：Claude Sonnet 4"]').setValue("Override Model");
    await card.get('[data-testid="model-create-save"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    const created = store.registry?.providers[0].models.find((model) => model.model === "override-model");
    expect(created?.baseUrl).toBe("https://mirror.example.invalid/v1");
  });

  it("fills token params from K/M preset badges inside the model form", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    await wrapper.get('[data-testid="model-row-edit-model-a1"]').trigger("click");

    const region = wrapper.get('[data-testid="model-detail-model-a1"]');
    await region.get('[data-testid="context-preset-1M"]').trigger("click");
    const inputs = region.findAll('input[type="number"]');
    expect((inputs[0]!.element as HTMLInputElement).value).toBe("1000000");

    await region.get('[data-testid="output-preset-128K"]').trigger("click");
    expect((inputs[1]!.element as HTMLInputElement).value).toBe("128000");
  });

  it("deletes the selected provider and falls back to the next one", async () => {
    seedRegistry();
    const wrapper = await mountProviderPage();

    // 当前选中 Alpha；section 头部的删除经 ConfirmPopover stub 确认
    const providerSection = wrapper.get('[data-testid="provider-detail-section"]');
    await providerSection.get('[data-testid="confirm-popover-confirm"]').trigger("click");
    await Promise.resolve();
    await Promise.resolve();

    const store = useProviderStore();
    expect(store.notice).toBe("提供商已删除。");
    expect(store.registry?.providers.map((provider) => provider.id)).toEqual(["provider-beta"]);
    // 回落到下一个提供商的视图
    expect(wrapper.get("section h2").text()).toBe("Beta");
  });
});
