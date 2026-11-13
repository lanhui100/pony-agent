/**
 * 桌面端快捷键清单（真值源）。
 *
 * 设计约束：
 * - 本模块必须是**纯数据 + 纯函数**，不依赖 vue / pinia / DOM 全局，便于独立单测。
 * - key token 使用平台无关写法：`Mod`（macOS 为 ⌘，其它平台为 Ctrl）、`Shift`、`Alt`、
 *   `Esc`、`Enter`、`↑`、`↓` 与斜杠命令原文（`/goal` 等）；平台差异只在渲染时由
 *   `resolveShortcutKey` 解析，避免清单与平台耦合。
 * - 清单必须与 `HomeWorkspace.vue::handleWindowKeydown` 的真实绑定保持同源，改动键位时
 *   同步更新此处（`tests/shortcuts-help-registry.spec.ts` 提供机器门禁）。
 */

export interface ShortcutItem {
  keys: readonly string[];
  description: string;
}

export interface ShortcutSection {
  title: string;
  items: readonly ShortcutItem[];
}

/** 平台无关的修饰键 token：渲染时按平台解析。 */
export const SHORTCUT_MODIFIER_TOKEN = "Mod";

/**
 * handler 实际消费的「主修饰键 + 单字母」键集合（小写）。
 * 供测试断言「帮助面板列出的 Mod 组合键 ⊆ handler 实际处理的键」。
 */
export const SHORTCUT_HANDLED_MODIFIER_KEYS: ReadonlySet<string> = new Set(["h", "g", "t", "b", "z"]);

export const SHORTCUT_SECTIONS: ReadonlyArray<ShortcutSection> = [
  {
    title: "目标与计划",
    items: [
      { keys: [SHORTCUT_MODIFIER_TOKEN, "G"], description: "打开 / 切换目标 (Goal) 面板" },
      { keys: ["/goal"], description: "设置、管理或查看会话目标" },
      { keys: ["/plan"], description: "切换右侧计划面板" },
      { keys: ["/tasks"], description: "切换右侧 Team 任务看板" }
    ]
  },
  {
    title: "对话与发送",
    items: [
      { keys: ["Enter"], description: "发送消息（非中文输入法组合态）" },
      { keys: ["Shift", "Enter"], description: "换行输入" },
      { keys: ["↑"], description: "输入框为空时，回填上一条发送的消息" },
      { keys: ["Alt", "S"], description: "切换普通排队 (Queue) 与插队 (Steer) 发送模式" }
    ]
  },
  {
    title: "会话与导航",
    items: [
      { keys: [SHORTCUT_MODIFIER_TOKEN, "Z"], description: "撤销最后一轮对话" },
      { keys: ["Esc", "Esc"], description: "流式输出中双击停止本轮生成" },
      { keys: [SHORTCUT_MODIFIER_TOKEN, "T"], description: "平滑滚动到会话顶部" },
      { keys: [SHORTCUT_MODIFIER_TOKEN, "B"], description: "平滑滚动到最新消息（底部）" },
      { keys: ["Alt", "↑"], description: "跳转到上一轮对话" },
      { keys: ["Alt", "↓"], description: "跳转到下一轮对话" }
    ]
  },
  {
    title: "快捷命令与通用",
    items: [
      { keys: [SHORTCUT_MODIFIER_TOKEN, "H"], description: "打开 / 关闭快捷键帮助面板" },
      { keys: ["/compact"], description: "手动压缩当前会话上下文" },
      { keys: ["Esc"], description: "关闭弹窗或取消操作" }
    ]
  }
];

/** 单 token 平台解析：`Mod` → ⌘ / Ctrl，其余原样返回。 */
export function resolveShortcutKey(token: string, isMac: boolean): string {
  if (token === SHORTCUT_MODIFIER_TOKEN) {
    return isMac ? "⌘" : "Ctrl";
  }
  return token;
}

/** `navigator.platform` 的 Mac 判定（抽成纯函数以便测试，缺省视为非 Mac）。 */
export function isMacPlatform(platform: string | undefined): boolean {
  if (!platform) {
    return false;
  }
  return /(Mac|iPhone|iPod|iPad)/i.test(platform);
}
