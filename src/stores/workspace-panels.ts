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

export type WorkspacePanel = "plan" | "roster" | "tasks" | "goal";

type WorkspacePanelsState = {
  activePanel: WorkspacePanel | "";
};

export const useWorkspacePanelsStore = defineStore("workspacePanels", {
  state: (): WorkspacePanelsState => ({
    activePanel: ""
  }),
  actions: {
    /**
     * 幂等展开（斜杠命令进入 Plan 模式时用）。
     *
     * 与 `togglePanel` 的区别只在"已是该面板时不动"：模式切换每次都必须把方案正文
     * 摆到用户眼前，再点一次收起会让「进入模式」看起来没有反馈。
     */
    openPanel(panel: WorkspacePanel) {
      this.activePanel = panel;
    },
    togglePanel(panel: WorkspacePanel) {
      this.activePanel = this.activePanel === panel ? "" : panel;
    },
    togglePlan() {
      this.togglePanel("plan");
    },
    toggleGoal() {
      this.togglePanel("goal");
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
