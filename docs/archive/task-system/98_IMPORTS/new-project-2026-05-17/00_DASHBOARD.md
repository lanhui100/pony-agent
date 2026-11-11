# Pony Agent 项目总览

## 项目目标

逐步用 `Tauri + Rust` 重构一个学习式 agent 项目，让 agent core、前端工作台、provider 体系、工具调用与学习文档一起演进。

## 当前阶段

当前处于“主链路已打通，继续增强真实 provider 和 runtime 体验”的阶段。

## 当前状态摘要

- 状态：`In Progress`
- 当前重点：真实 provider 对话、Provider 配置体验、`run_turn()` 主线演进
- 最近完成：
  - Provider 配置页建立
  - 多 provider / 多模型基础切换
  - `API Key` 从长期普通配置中剥离
  - UI 可写入用户环境变量
  - 已有模型支持编辑
- 当前断点：
  - 主页仍需更清晰地区分真实 provider 与 mock fallback
  - `run_turn()` 的执行状态与 provider 来源展示还不够直观

## 下一步最小动作

在主页对话区或右侧状态栏中，明确显示当前回合的凭证来源与回退状态，例如：

- 当前 provider 是否命中运行时输入
- 是否使用环境变量
- 是否因为缺少凭证而回退到 mock

## 当前高优任务

- `TASK-001`：主页显示 provider 真实来源与 mock fallback
- `TASK-002`：增强 `run_turn()` 的运行状态可视化
- `TASK-003`：梳理 provider / env / runtime 的学习与使用说明

## 风险与注意事项

- 当前 docs 中部分旧文件仍有历史编码显示问题，后续可逐步清理
- `API Key` 当前已支持写入 env，但还不是最终的跨平台安全凭证方案
- 不要让凭证系统继续阻塞 agent core 主线
