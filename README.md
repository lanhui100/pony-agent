# Pony Agent

这是一个以 `Pony Agent` 为名的新项目根目录，内置 `Tauri + Rust` 智能体重构骨架，并保留 `hermes/` 作为参考实现。

## 目标

- 保留现有 Python Hermes 作为参考实现
- 用 Rust 重写智能体核心运行时
- 用 Tauri 提供桌面端壳层与前端桥接
- 参考 LangChain / LangGraph 的编排思路，以及 Claude 类产品的交互体验

## 目录

- `src/`：前端界面
- `src-tauri/`：Tauri 配置与 Rust 核心

## 开发前提

根据 Tauri v2 官方文档，Windows 开发至少需要：

- Rust / rustup（MSVC toolchain）
- Microsoft C++ Build Tools
- WebView2 Runtime

参考文档：

- [Tauri Create Project](https://v2.tauri.app/start/create-project/)
- [Tauri Prerequisites](https://v2.tauri.app/start/prerequisites/)

当前仓库已经在本机完成过 `cargo test` 与前端单测验证，但新环境仍需先确认以下依赖齐全。

## 安装建议

先安装 Rust 和 Tauri 前置依赖，再在本目录执行：

```powershell
npm install
npm run tauri dev
```

## 当前实现进度提示

当前项目已经完成 `turn lifecycle / recovery / hooks / session control` 这一批主线收口，尤其是：

- `PA-042`：history-control audit surface 与 summary-first explainability
- `PA-043`：run-control audit surface 与 summary-first explainability

如果要快速理解这批实现，建议优先阅读：

- [Session Control Plane 与 Audit Surface 架构基线](C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/session-control-plane-and-audit-surface.md)
- [PA-042 canonical spec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/session-control-audit-surface-and-history-evidence-summary/spec.md)
- [PA-043 canonical spec](C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/run-control-audit-surface-and-summary-first-explainability/spec.md)

## Rust 构建缓存约定

为避免 Windows 下 Rust/Tauri 编译缓存膨胀，不要再手工创建 `src-tauri/target-codex-*` 这类一次性目录。

统一使用项目内的固定缓存槽位：

```powershell
npm run cargo:check:shared
npm run cargo:test:shared
npm run cargo:test:exact -- --lib some_test_name -- --exact
npm run cargo:test:exact:b -- --lib some_other_test -- --exact
```

约定说明：

- `cargo:check:shared` 复用 `target-check`
- `cargo:test:shared` 复用 `target-test`
- `cargo:test:exact` / `:b` / `:c` 分别复用 `target-test-exact-a/b/c`
- 做完大轮验证后，可执行 `npm run clean:tauri:light`
- 如果磁盘再次吃紧，可执行 `npm run clean:tauri:deep`

## Git 对象库维护

如果 `.git` 目录异常膨胀，通常不是正常提交历史变大，而是临时构建产物或超大文件曾被 `git add` 进对象库，后来虽然撤回了工作区变更，但 Git 垃圾对象仍然留在 `.git/objects/`。

日常建议：

- 不要手工 `git add` `src-tauri/target*`、`target*`、`.tmp/`、`.codex-logs/`、`tmp-*.png`、`tmp-*.log`
- 先用 `npm run git:diagnose:objects` 看 loose objects 和 pack 体积
- 常规维护可执行 `npm run git:gc`
- 若确认只是历史遗留垃圾对象导致膨胀，可执行 `npm run git:gc:deep`
