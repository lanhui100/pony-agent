import { defineStore } from "pinia";

/**
 * 右侧栏面板共享状态（契约 C-8）。
 *
 * 面板开合原先由 `HomeSidebar` 组件内 `activePanel` ref 私有持有，导致斜杠命令
 * （`/plan`、`/tasks`）无法从提交流程驱动。现提升为 pinia store，由 `HomeSidebar`
 * 与斜杠命令处理器共同消费；交互语义与提升前完全等价（再次触发同一面板即收起）。
 *
 * 契约 C-8 写「切换面板」，因此斜杠命令与点击标题共用同一 toggle 语义，
 * store 不提供"幂等展开"入口，避免两套开合语义漂移。
 */

export type WorkspacePanel = "plan" | "roster" | "tasks";

type WorkspacePanelsState = {
  activePanel: WorkspacePanel | "";
};

export const useWorkspacePanelsStore = defineStore("workspacePanels", {
  state: (): WorkspacePanelsState => ({
    activePanel: ""
  }),
  actions: {
    togglePanel(panel: WorkspacePanel) {
      this.activePanel = this.activePanel === panel ? "" : panel;
    },
    togglePlan() {
      this.togglePanel("plan");
    },
    toggleTasks() {
      this.togglePanel("tasks");
    },
    toggleRoster() {
      this.togglePanel("roster");
    },
    closePanel() {
      this.activePanel = "";
    }
  }
});
