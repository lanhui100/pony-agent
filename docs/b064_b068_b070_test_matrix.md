# 波次专项测试矩阵 (B064, B068, B070)

| 任务 | 级别 | 契约验证 (L2-T) | 对抗破坏注入 (L2-AT) |
| :--- | :--- | :--- | :--- |
| **B064 (Windows 首跑扫尾)** | L2 | 验证全平台运行基线，确保跨平台语义一致且无未声明的平台跳过。 | 模拟缺失的平台环境变量及畸形平台路径，断言 Fail-Closed 或降级保护。 |
| **B068 (control_plane 历史图)** | L2 | 验证 `history_commands_and_runtime_view_follow_persisted_history_graph` node id 确定性匹配契约。 | 乱序写入历史节点及注入重复历史记录，断言历史图拓扑解析保持一致。 |
| **B070 (TUI e2e 隔离)** | L2 | 验证 `ask_resume_e2e`, `approval_resume_e2e`, `stop_graph_run_wave64` 强制使用 `PONY_AGENT_DATA_DIR` 旁路隔离。 | 未配置 `PONY_AGENT_DATA_DIR` 时跨平台 fallback 检测，断言绝不穿透至真实操作系统数据目录。 |
