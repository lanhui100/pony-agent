# PA-075 清理会话日志中的过期路径引用

## 基本信息
- 编号: PA-075
- 名称: 清理会话日志中的过期路径引用
- 状态: Ready
- 优先级: P3
- 创建日期: 2026-06-27
- 更新日期: 2026-06-27

## 目标
`management/task-system/99_LOGS/` 和 `02_REVIEWS/` 中大量 session log 仍引用 `src-tauri/src/agent/` 路径。统一替换为 `crates/pony-agent-core/src/agent/`。

## 输出
- `management/task-system/99_LOGS/*.md` 中的路径引用更新
- `management/task-system/02_REVIEWS/*.md` 中的路径引用更新

## 范围
- 搜索所有 `src-tauri/src/agent/` 引用并替换
- 只替换路径引用，不修改内容语义

## 验收标准
1. 搜索 `src-tauri/src/agent/` 在 `management/task-system/` 下不再有匹配

## 断点续跑
- 当前状态: Ready
- 下一步: 使用全局搜索替换工具执行
