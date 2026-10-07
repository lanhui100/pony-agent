# Wave 1 T3：并发遗留未提交改动分诊（只读调查）

> 调查人：wave1-tester（Test Agent）
> 调查基线：`git status` 显示 4 项未提交改动（分支 hygiene-cleanup-1791356420，HEAD=805541e v0.1.108）
> 本调查只读，未修改任何文件；git 写操作由 Lead 唯一执行。

## 1. process.rs：tracked_count cfg 改动（#[cfg(test)] → #[cfg(all(test, windows))]）

### 改动事实
- 位置：`crates/pony-agent-core/src/agent/process.rs:516`，`pub(crate) fn tracked_count` 的属性从 `#[cfg(test)]` 改为 `#[cfg(all(test, windows))]`（git diff 已确认，无其他 process.rs 改动）。
- 该函数体仅 `self.inner.processes.lock().unwrap().len()`，不触碰任何 Windows API（非 windows-sys 依赖）。

### 调用点核查（全部在 #[cfg(windows)] 测试内）
process.rs 中 tracked_count 共 5 个调用点：1735 / 1749 / 1765 / 1787 / 1810 行，
全部位于同一个测试函数 `windows_job_failure_injection_cleans_up_and_publishes_no_handle`
（1717 行起，`#[cfg(windows)] #[test]`，函数体持续至 1869 行下一个测试函数之前）。
process.rs 1611-1900 区间全部测试函数均为 `#[cfg(windows)] #[test]`（windows_job_* 系列）。

### Linux 编译影响（只读验证）
- Linux 下 `#[cfg(windows)]` 测试函数整体不参与编译 → 上述 5 个调用点不编译 → tracked_count
  的 cfg 缩窄为 windows-test-only 与调用面完全一致，**Linux test 编译不受影响**。
- 实测：`cargo check --tests -p pony-agent-core --target-dir target-check` → **exit 0**
  （`Finished dev profile in 30.76s`，无 error/warning）。
- Windows 下 test 编译：`all(test, windows)` 命中，正常；产品代码不受 cfg(test) 影响。

### 来源推测
分支名 hygiene-cleanup-*（时间戳 1791356420）暗示为一次代码卫生清理会话的产物；
reflog 显示该分支 checkout 自 main（805541e），改动未提交。缩窄 cfg 与其唯一使用场景
（windows-only 测试）自洽，是**正确且无害**的清理，无行为影响。

### 处置建议
**提交（保留）**。改动正确、最小、自洽；Linux 与 Windows 编译均不受影响。
如倾向更保守，也可并入 Lead 的 hygiene 清理提交一并提交；不建议回退。

## 2. 三个被删 Vue 组件（TitleBar.vue / ui/Card.vue / ui/Separator.vue）

### 引用核查结果（全库，含 src/、tests/、src-tauri/，排除 node_modules / dist / target*）
| 组件 | 结果 |
|---|---|
| `src/components/TitleBar.vue` | 无任何引用（.vue/.ts/.js/.html 全 0 命中；无动态 import） |
| `src/components/ui/Card.vue` | 无任何引用；现存 `ui/` 目录仅 Badge/Button/ConfirmPopover/DropdownMenu/Input/ScrollArea/ScrollBar/Switch/Tooltip |
| `src/components/ui/Separator.vue` | 无任何引用；`DropdownMenuSeparator` 来自外部包 `reka-ui`（DropdownMenu.vue:8-20 import），与本组件无关 |

- 动态 import：src/ 下仅 `@tauri-apps/api/window`、`@tauri-apps/plugin-dialog` 等外部包，无组件动态导入。
- 组件桶文件：`src/components/` 与 `src/components/ui/` 均无 index.ts / 桶导出。
- 自动注册：`vite.config.ts` 仅 `vue()` + `tailwindcss()` 插件，无 unplugin-vue-components
  等自动导入 → 不存在"隐式引用"通道。
- 匹配到 `AskUserToolCallCard.vue` / `DropdownMenuSeparator` 的检索结果均为不同目标（ask/ 目录
  组件、reka-ui 包），非被删组件。

### 处置建议
**提交（保留）**。三个组件已无任何引用（含隐式通道），删除是安全的死代码清理，
与分支名 hygiene-cleanup 语义一致；HEAD 中最后一次修改 TitleBar.vue 的提交 c62a196
（feat(ui)）之前曾被 App 引用，现已全部迁移/移除。

## 3. 总结
- process.rs tracked_count cfg：保留提交（无害清理，Linux/Windows 编译均验证）。
- 三个 Vue 组件：保留提交（无任何引用，安全删除）。
- 两处均非本次弹窗修复引入，与 T1 测试分诊无因果关联，不阻塞 CI 测试修复。
