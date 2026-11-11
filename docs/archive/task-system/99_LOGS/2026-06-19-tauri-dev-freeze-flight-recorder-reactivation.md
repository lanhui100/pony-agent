# Session Log: 2026-06-19 — Tauri Dev Freeze Diagnosis & Flight Recorder Reactivation

## 背景

tauri dev 启动时前端卡死/白屏。

## 诊断路径

1. **SQLite 同步加载阻塞主线程 7.7s（已确认）**
   - `SessionStore::new()` 在主线程同步打开 11MB SQLite 并读取 28 个会话
   - 修复：`OnceLock` + 后台线程初始化，窗口立即出现
   - 提交: `f6aab80`

2. **启动链 yield 使用 requestAnimationFrame 导致节流（已确认）**
   - rAF 在 WebView2 特定 GPU/驱动条件下被严重节流，两帧间隔从 16ms 飙到数秒
   - 修复：替换为 `setTimeout(0)`（后续优化时已在组件还原中回退，因为原始代码无缝切换到 Vite HMR）

3. **flight recorder 集成导致事件循环冻结（已确认并修复）**
   - `tauri.ts` 导入 `frontend-flight-recorder.ts`（循环依赖）
   - `safeInvoke` 包装埋点 → 每个 IPC 调用生成 2-3 个事件
   - `flush()` → `safeInvoke("append_frontend_trace_events")` → 后端无此命令 → 失败重试 → 死循环
   - 修复：
     - 回退 `tauri.ts`、组件文件中的 flight recorder 引用
     - 模块级 `recorderState` 惰性初始化（不调用 `isTauriAvailable()`）
     - `flush()` 添加指数退避（500ms → 1s → 2s … 30s max）
     - `flush()` 添加 `initialized` 守卫
     - Rust 端 6 个命令全部 async + `spawn_blocking`

4. **git hook PowerShell 兼容性修复**
   - `.githooks/pre-commit` 使用 `powershell`（5.1）无法解析 Unicode 制表符
   - 修复：改为 `pwsh`（PowerShell 7+）

## 提交记录

- `f6aab80` — feat: integrate flight recorder + startup freeze fix
- `20f8a56` — fix: switch git hooks to pwsh

## 剩余工作

- `src/components/HomeWorkspace.vue` 中的 flight recorder instrumentation 尚未恢复
- 后续可通过分析 `frontend-diagnostics.db` 中的 stall 数据定位剩余卡顿

## 关联文档

- OpenSpec change: `openspec/changes/archive/2026-06-19-build-frontend-flight-recorder-and-stall-diagnostics/`
- Task card: `management/task-system/03_TASKS/PA-057-build-frontend-flight-recorder-and-stall-diagnostics.md`
- Review: `management/task-system/02_REVIEWS/2026-06-18-pa057-code-review.md`
