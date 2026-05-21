# 2026-05-21 Session 03 - PA-008 tool runtime first pass

## 背景
用户把当前并行开发中的 `PA-008` 指派出来，要求优先补强工具层的多工具、并发、权限和错误恢复语义，写入范围以 `tools.rs / planner.rs` 为主。

## 本轮完成
- 整体重写 `src-tauri/src/agent/tools.rs`，清理旧乱码字符串并保留已有工具接口
- 新增 `workspace_batch`
  - 单次工具调用内可执行多个受限子调用
  - 支持 `parallel`
  - 支持 `continueOnError`
  - 禁止递归 batch
- 新增 `workspace_gather_context`
  - 文件模式：`path_info + read_file_segment`
  - 目录模式：`path_info + list_files`
  - 搜索模式：`path_info + search_text`
  - 聚合结果保留 partial success 语义，但对 runtime 仍返回可 follow-up 的 `ok`
- 为 `workspace_read_file` 增加整文件读取预算保护
- 为 `workspace_search_text` 增加目录预算保护，默认跳过典型大目录
- 重写 `src-tauri/src/agent/planner.rs`
  - 本地 planner 现在能区分目录列举、显式路径概览、基于历史路径的继续追问、带引号的本地搜索语句
- 为 `tools.rs / planner.rs` 补了最小单元测试

## 已验证
- `cargo check --target-dir ../target-check` 通过

## 未完全完成的验证
- `cargo test --lib` 已开始，但独立测试 target dir 首轮编译较慢，尚未等到最终结束

## 风险与后续
- 当前多工具仍是“组合工具”语义，而非 runtime 级显式 `ToolPlan`
- trace / tool activity 仍只把 batch / gather 视为一个外层工具，后续需要决定是否把内部子调用展开到 UI
