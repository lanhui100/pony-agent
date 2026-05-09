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

当前机器缺少 Rust 工具链，暂时无法直接运行 Tauri 构建。

根据 Tauri v2 官方文档，Windows 开发至少需要：

- Rust / rustup（MSVC toolchain）
- Microsoft C++ Build Tools
- WebView2 Runtime

参考文档：

- [Tauri Create Project](https://v2.tauri.app/start/create-project/)
- [Tauri Prerequisites](https://v2.tauri.app/start/prerequisites/)

## 安装建议

先安装 Rust 和 Tauri 前置依赖，再在本目录执行：

```powershell
npm install
npm run tauri dev
```
