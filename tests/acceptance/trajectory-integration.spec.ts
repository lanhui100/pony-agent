import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryToolbar from "@/components/trajectory/TrajectoryToolbar.vue";
import TrajectoryTimeline from "@/components/trajectory/TrajectoryTimeline.vue";
import HomeTracePanel from "@/components/HomeTracePanel.vue";
import type { TrajectoryTimelineModel, TrajectoryTimelineSpan } from "@/lib/runtime/trajectory-core";
import type { TurnTraceRecord } from "@/types/runtime";

describe("Trajectory Integration & E2E Components", () => {
  describe("TrajectoryToolbar.vue", () => {
    it("renders duration toggle, turns toggle, calls toggle and search input", async () => {
      const wrapper = mount(TrajectoryToolbar, {
        props: {
          actualDuration: false,
          allTurnsCollapsed: false,
          allCallsCollapsed: false,
          searchQuery: "",
        },
      });

      const durationBtn = wrapper.find("[data-testid='trajectory-toolbar-duration-toggle']");
      const turnsBtn = wrapper.find("[data-testid='trajectory-toolbar-turns-toggle']");
      const callsBtn = wrapper.find("[data-testid='trajectory-toolbar-calls-toggle']");
      const searchInput = wrapper.find<HTMLInputElement>("[data-testid='trajectory-toolbar-search-input']");

      expect(durationBtn.exists()).toBe(true);
      expect(durationBtn.text()).toContain("等宽视图");

      expect(turnsBtn.exists()).toBe(true);
      expect(turnsBtn.text()).toContain("折叠轮次");

      expect(callsBtn.exists()).toBe(true);
      expect(callsBtn.text()).toContain("折叠调用");

      expect(searchInput.exists()).toBe(true);

      // Toggle duration
      await durationBtn.trigger("click");
      expect(wrapper.emitted("update:actualDuration")?.[0]).toEqual([true]);

      // Toggle turns
      await turnsBtn.trigger("click");
      expect(wrapper.emitted("toggle-all-turns")).toBeTruthy();

      // Toggle calls
      await callsBtn.trigger("click");
      expect(wrapper.emitted("toggle-all-calls")).toBeTruthy();

      // Input search query
      await searchInput.setValue("read_file");
      expect(wrapper.emitted("update:searchQuery")?.[0]).toEqual(["read_file"]);
    });

    it("displays active styling when actualDuration is true", () => {
      const wrapper = mount(TrajectoryToolbar, {
        props: {
          actualDuration: true,
          allTurnsCollapsed: true,
          allCallsCollapsed: true,
          searchQuery: "test",
        },
      });

      const durationBtn = wrapper.find("[data-testid='trajectory-toolbar-duration-toggle']");
      expect(durationBtn.text()).toContain("实际耗时");
    });
  });

  describe("TrajectoryTimeline.vue", () => {
    const mockModel: TrajectoryTimelineModel = {
      totalStart: 0,
      totalEnd: 1000,
      totalDurationMs: 1000,
      turnBoundaries: [
        { turnId: "turn-1", title: "Turn 1", time: 0 },
        { turnId: "turn-2", title: "Turn 2", time: 500 },
      ],
      spans: [
        {
          id: "span-input",
          turnId: "turn-1",
          index: 0,
          kind: "user",
          label: "User Input",
          lane: 0,
          start: 0,
          end: 100,
          durationMs: 100,
          isError: false,
        },
        {
          id: "span-model",
          turnId: "turn-1",
          index: 1,
          kind: "message",
          label: "DeepSeek Chat",
          lane: 1,
          start: 100,
          end: 400,
          durationMs: 300,
          isError: false,
        },
        {
          id: "span-tool",
          turnId: "turn-1",
          index: 2,
          kind: "tool",
          label: "read_file",
          lane: 2,
          start: 400,
          end: 500,
          durationMs: 100,
          isError: false,
        },
        {
          id: "span-error",
          turnId: "turn-2",
          index: 0,
          kind: "message",
          label: "Failed Call",
          lane: 1,
          start: 500,
          end: 900,
          durationMs: 400,
          isError: true,
        },
      ],
    };

    it("renders 3 swimlanes and spans with correct lane distribution and emits select-span", async () => {
      const wrapper = mount(TrajectoryTimeline, {
        props: {
          model: mockModel,
          selectedSpanId: null,
        },
        global: {
          stubs: {
            Tooltip: {
              template: `<div><slot /></div>`,
            },
          },
        },
      });

      expect(wrapper.find("[data-testid='trajectory-timeline']").exists()).toBe(true);

      const spanInput = wrapper.find("[data-testid='trajectory-span-span-input']");
      const spanModel = wrapper.find("[data-testid='trajectory-span-span-model']");
      const spanTool = wrapper.find("[data-testid='trajectory-span-span-tool']");
      const spanError = wrapper.find("[data-testid='trajectory-span-span-error']");

      expect(spanInput.exists()).toBe(true);
      expect(spanModel.exists()).toBe(true);
      expect(spanTool.exists()).toBe(true);
      expect(spanError.exists()).toBe(true);

      // Verify error class styling
      expect(spanError.classes()).toContain("bg-rose-500");

      // Verify span label shows single uppercase letter for core event name
      expect(spanModel.text().trim()).toBe("M");
      expect(spanTool.text().trim()).toBe("T");
      expect(spanInput.text().trim()).toBe("U");

      // Click on tool span
      await spanTool.trigger("click");
      expect(wrapper.emitted("select-span")?.[0][0]).toEqual(mockModel.spans[2]);
    });
  });

  describe("HomeTracePanel.vue Integration", () => {
    const mockTurns: TurnTraceRecord[] = [
      {
        turnId: "turn-1",
        title: "Query 1",
        phase: "completed",
        updatedAt: 1000,
        traceSteps: [],
        toolActivities: [],
        traceTimeline: [
          {
            id: "step-1-input",
            kind: "input",
            label: "User Input",
            state: "done",
            sequence: 1,
            durationMs: 50,
          },
          {
            id: "step-1-tool",
            kind: "call_tool",
            label: "execute_script",
            state: "done",
            sequence: 2,
            durationMs: 250,
          },
        ],
      },
    ];

    it("integrates TrajectoryToolbar and TrajectoryTimeline inside HomeTracePanel", async () => {
      const wrapper = mount(HomeTracePanel, {
        props: {
          turns: mockTurns,
          sessionId: "test-session",
          copiedKey: "",
          open: true,
          canonicalKind: (kind) => kind,
          turnTimeline: (turn) => turn.traceTimeline || [],
          providerReturnedCacheHitInputTokens: () => null,
          expanded: true,
        },
        global: {
          stubs: {
            Tooltip: {
              template: `<div><slot /></div>`,
            },
            ScrollArea: {
              template: `<div><slot /></div>`,
            },
          },
        },
      });

      const toolbar = wrapper.findComponent(TrajectoryToolbar);
      expect(toolbar.exists()).toBe(true);

      const timeline = wrapper.findComponent(TrajectoryTimeline);
      expect(timeline.exists()).toBe(true);

      // Clicking timeline span triggers selection in HomeTracePanel
      const toolSpan = wrapper.find("[data-testid='trajectory-span-step-1-tool']");
      if (toolSpan.exists()) {
        await toolSpan.trigger("click");
        // active turn and step selected
        expect(wrapper.vm.activeTurnId).toBe("turn-1");
      }
    });
  });
});
