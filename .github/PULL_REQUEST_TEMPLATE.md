<!-- 提交前请确认：分支名 <类型>/<任务号或主题>；一个 PR 只做一件事。 -->

## 变更摘要

<!-- 一句话说清「改了什么、为什么改」。 -->

## 关联

- Backlog：B___
- ADR：`.agents/notes/...`（非平凡变更必填，含 `## Alternatives considered`）
- 关联 PR / issue：

## 验证证据（贴命令与退出码，不写「应该没问题」）

```text
$ npm run <门禁>
<exit code / 关键输出>
```

## 并发安全自查

- [ ] 本 PR 在**独立 worktree** 中完成，未在主工作区与其他会话共用一份索引
- [ ] 分支名符合 `<类型>/<任务号或主题>`，未直接推送 `main`
- [ ] 未提交构建产物（`target*/`、`dist/`、`coverage/`、`sessions/` 等）
- [ ] 压到共享文件的改动（`backlog/tasks.jsonl`、版本四处同步文件）已确认无并发写入
- [ ] 行为/契约变更已在同一次提交内更新对应文档与 ADR

## 合并方式

- 默认 **Squash and merge**（main 保持线性，一 PR 一提交）
- 发版 PR（`chore/release-v*`）用 **Merge commit**：squash 会让 tag 指向不在 main 上的提交
