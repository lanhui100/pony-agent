// 侧边栏三级树：全部用户可见文案单源（快照断言依据；design 文案基线落地）。
// 集中于此便于 vitest 快照与后续 i18n 迁移；运行时 wire 错误串按惯例不翻译直透。

export const SIDEBAR_COPY = {
  sectionTitle: "工作区",
  addWorkspaceTooltip: "添加工作区",

  newConversationHere: "在此工作区新建对话",
  workspaceActionsAria: "工作区「{name}」的操作",
  conversationActionsAria: "会话「{name}」的操作",

  menuRenameWorkspace: "重命名工作区",
  menuDeleteWorkspace: "删除工作区",
  menuRenameConversation: "重命名对话",
  menuArchiveConversation: "归档对话",
  menuDeleteConversation: "删除对话",

  deleteWorkspaceTitle: "删除工作区",
  deleteWorkspaceDescription: (name: string, count: number): string =>
    count === 0
      ? `将移除工作区「${name}」的注册，不影响磁盘上的目录与文件。\n这些对话后续的文件操作将在默认工作区目录进行。`
      : `将移除工作区「${name}」的注册，不影响磁盘上的目录与文件。名下的 ${count} 个对话会保留并移到顶部区域；这些对话后续的文件操作将在默认工作区目录进行。`,
  deleteWorkspaceConfirm: "删除工作区",

  archiveTitle: "归档对话",
  archiveDescription:
    "归档后该对话将从侧边栏消失，当前版本无法从界面恢复（磁盘历史仍在）。",
  archiveConfirm: "归档对话",

  deleteConversationTitle: "删除对话",
  deleteConversationConfirm: "删除对话",

  renameEmptyError: "名称不能为空",
  renameTooLongError: "名称不能超过 64 个字符",
  renameDuplicateWorkspaceError: "已存在同名工作区",

  disabledRunningTooltip: "对话运行中，暂不能执行该操作",
  disabledSubmittingTooltip: "正在提交，请稍候",

  confirmBusyLabel: "处理中…",
  confirmFailurePrefix: "操作失败：",

  emptyGroupHint: "该工作区暂无对话",
  emptyTreeHint: "暂无对话；发送第一条消息后会自动保存到当前工作区。"
} as const;

export function formatSidebarCopy(
  template: string,
  params: Record<string, string | number>
): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) =>
    key in params ? String(params[key]) : match
  );
}
