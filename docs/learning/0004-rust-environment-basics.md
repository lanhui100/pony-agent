# 0004 Pony Agent 需要哪些 Rust 开发环境，它们分别做什么

## 问题

为了继续开发 Pony Agent 的 Rust 部分，需要安装哪些环境？这些环境分别是什么、有什么作用？

## 简短结论

当前 Windows 上继续开发 Pony Agent，至少需要：

- `rustup`
- `rustc`
- `cargo`
- Microsoft C++ Build Tools
- WebView2 Runtime

## 系统化梳理

### 1. `rustup`

Rust 官方安装器和版本管理器。

作用：

- 安装 Rust
- 管理 toolchain
- 提供 `rustc` 和 `cargo`

### 2. `rustc`

Rust 编译器。

作用：

- 把 `.rs` 代码编译成可执行程序或库

### 3. `cargo`

Rust 的构建和包管理工具。

作用：

- 下载依赖
- 构建项目
- 运行项目
- 跑测试

### 4. Microsoft C++ Build Tools

Windows 下的原生编译支持工具。

作用：

- 为 Rust 在 Windows MSVC 环境下构建提供底层支持

### 5. WebView2 Runtime

Tauri 在 Windows 上显示前端页面依赖的运行时。

作用：

- 承载桌面窗口中的 Web 前端

### 6. 在 Pony Agent 里它们分别用在哪里

- `rustup`：安装和管理 Rust
- `rustc`：编译 `src-tauri/` 的 Rust 代码
- `cargo`：构建和运行 Tauri 原生端
- Build Tools：让 Windows 原生编译链完整
- WebView2：让 Tauri 窗口能显示 Vue 前端

## 常见误区

- 误区 1：有 Node 环境就能跑 Tauri
- 误区 2：Build Tools 是给写 C++ 的人用的，Rust 不需要

## 后续值得继续学什么

- Rust toolchain 在 Windows 上的构成
- Tauri 的前后端构建链路

## 可延展内容选题

- 公众号：`Tauri + Rust 在 Windows 上到底要装什么环境？`
- 知乎：`初学 Rust 做桌面应用时，rustup、rustc、cargo 分别是什么？`
