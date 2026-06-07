# Tasks: Add Session Control Surface And Feedback Loop

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-037` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-session-control-surface-and-feedback-loop` 的 proposal / design / spec 文档

## 2. Session Control UX Integration

- [x] 2.1 为 stop / resume / continue / replay 建立显式前端控制入口
- [x] 2.2 为 checkout / restore 的 degrade result 建立显式反馈
- [x] 2.3 为 historical / historical_dirty / paused / recovery-capable 建立统一状态文案、真相源映射与 disabled reason

## 3. Verification and Closeout

- [x] 3.1 为 `HomeSessionSidebar` 补 history control status / degrade feedback 验收测试
- [x] 3.2 为 `HomeWorkspace` 与 `runtime-store` 补 stop/resume/replay CTA 展示与派发回归
- [x] 3.3 回写任务卡、review 文档、日志与验收证据
