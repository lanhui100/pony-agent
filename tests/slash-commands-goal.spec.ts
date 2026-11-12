/**
 * 桌面端 `/goal` 斜杠命令单测（契约 C-5，真值源 `.dev-team/contract-matrix-goal-command.md`）。
 *
 * 冻结资产说明：`tests/slash-commands.spec.ts` 由 Test Agent 冻结（旧契约 C-8/C-9 的断言不得就地改写），
 * 故本波次新增用例独立成文件，只测**纯函数 + 元数据**，不挂载组件。
 * 命令执行语义（状态机/文案）真值源在 Rust `agent::goal_command`，前端不重复实现。
 */

import { describe, expect, it } from "vitest";
import {
  DESKTOP_SLASH_COMMANDS,
  describeGoalOutcome,
  goalCommandArgs,
  parseSlashCommand,
  type GoalCommandOutcome
} from "@/lib/chat/slash-commands";

describe("DESKTOP_SLASH_COMMANDS: /goal 元数据", () => {
  it("包含 /goal 条目且 usage/description 与契约一致", () => {
    const spec = DESKTOP_SLASH_COMMANDS.find((command) => command.name === "goal");
    expect(spec).toBeDefined();
    expect(spec?.usage).toBe("/goal");
    expect(spec?.description).toBe("设置或查看当前会话目标");
  });

  it("不重复登记 /goal", () => {
    expect(DESKTOP_SLASH_COMMANDS.filter((command) => command.name === "goal")).toHaveLength(1);
  });
});

describe("parseSlashCommand: /goal 命中与近似串", () => {
  it("精确命中（大小写不敏感、允许带参）", () => {
    expect(parseSlashCommand("/goal")).toBe("goal");
    expect(parseSlashCommand("/GOAL")).toBe("goal");
    expect(parseSlashCommand("  /goal   ")).toBe("goal");
    expect(parseSlashCommand("/goal 建立目标")).toBe("goal");
    expect(parseSlashCommand("/goal clear")).toBe("goal");
  });

  it("近似串不命中（保持既有「按普通消息发送」行为）", () => {
    expect(parseSlashCommand("/goa")).toBeNull();
    expect(parseSlashCommand("/goalx")).toBeNull();
    expect(parseSlashCommand("/goalx 目标")).toBeNull();
    expect(parseSlashCommand("goal 目标")).toBeNull();
    expect(parseSlashCommand("/")).toBeNull();
  });
});

describe("goalCommandArgs: 参数提取口径", () => {
  it("无参数返回空串", () => {
    expect(goalCommandArgs("/goal")).toBe("");
    expect(goalCommandArgs("  /goal  ")).toBe("");
    expect(goalCommandArgs("/GOAL")).toBe("");
  });

  it("带参返回首个空白之后的原文（仅裁首尾空白）", () => {
    expect(goalCommandArgs("/goal 建立目标")).toBe("建立目标");
    expect(goalCommandArgs("/goal   多个   空格的   目标  ")).toBe("多个   空格的   目标");
    expect(goalCommandArgs("/goal edit 新目标")).toBe("edit 新目标");
    expect(goalCommandArgs("/GOAL PAUSE")).toBe("PAUSE");
  });

  it("未命中一律空串，绝不吞正文", () => {
    expect(goalCommandArgs("/goa 目标")).toBe("");
    expect(goalCommandArgs("/goalx 目标")).toBe("");
    expect(goalCommandArgs("目标")).toBe("");
    expect(goalCommandArgs("")).toBe("");
    expect(goalCommandArgs("/plan 目标")).toBe("");
  });
});

describe("describeGoalOutcome: 单行状态条渲染", () => {
  const base: GoalCommandOutcome = {
    action: "create",
    message: "目标已创建\n目标: 建立目标\n轮次: 0/30",
    goal: null
  };

  it("多行回执折叠为单行，内容不丢", () => {
    const text = describeGoalOutcome(base);
    expect(text).toContain("目标已创建");
    expect(text).toContain("建立目标");
    expect(text).toContain("轮次: 0/30");
    expect(text.includes("\n")).toBe(false);
  });

  it("未返回结果必须如实说明（不得谎报成功）", () => {
    expect(describeGoalOutcome(null)).toBe("目标命令已执行，但未返回结果");
    expect(describeGoalOutcome(undefined)).toBe("目标命令已执行，但未返回结果");
    expect(describeGoalOutcome({ action: "show", message: "   ", goal: null })).toBe(
      "目标命令已执行，但未返回结果"
    );
  });
});
