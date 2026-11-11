# 0023 Windows 控制台子进程隐藏窗口与输出代码页解码回退

Status: implemented

## 背景

Run/Job/Terminal 在 Windows 上从 Tauri GUI 宿主派生 `cmd`/`powershell`/`git` 等控制台程序时未指定创建标志：父进程没有控制台，Windows 默认为每个控制台子进程分配一个新的可见控制台窗口——每次执行 Run 都会"弹 PowerShell 窗口"（长命令如 WMI 查询期间窗口持续可见）。

无窗口化会连带改变输出编码链：此前批处理靠 `chcp 65001` + 可见控制台让 PowerShell 5.1/cmd 以 UTF-8 向管道输出中文；去掉控制台后这些程序的中文输出退化为系统 ANSI/OEM 代码页（zh-CN 为 GBK），若仍按严格 UTF-8 解码会出现 U+FFFD 乱码。因此"隐藏弹窗"与"输出解码回退"必须作为同一修复落地，不能只做前者。

## 候选方案

- **方案 A：`CREATE_NO_WINDOW` 创建标志 + 输出解码回退（选定）**。在四个 spawn 点（ProcessManager/jobs/terminal/git diff）统一加 `creation_flags(CREATE_NO_WINDOW)`，并新增共享解码函数 `decode_process_output`：严格 UTF-8 直通（现有行为零改动），校验失败时 Windows 上按系统 ANSI→OEM 代码页转码（`MultiByteToWideChar`），非 Windows 维持 lossy。std 管道采集不受影响；编码链用解码回退兜底而非依赖可见控制台。
- **方案 B：宿主进程启动时 `AllocConsole` 一次并隐藏其窗口（不选）**。子进程继承该隐藏控制台，`chcp 65001` 继续生效，编码链原样保留。但要求改造应用启动路径（main.rs/WinMain），引入 WebView2 与隐藏控制台共存的未知交互，且每个应用实例持有一个隐藏控制台资源；相对收益（少一段回退解码代码）不抵启动路径的风险与侵入。
- **方案 C：保留可见控制台，仅依赖 STARTUPINFO wShowWindow 隐藏（不选）**。Rust `std::process` 不暴露 STARTUPINFO，需替换为原生 `CreateProcessW` 重写 spawn 路径（管道/Job 挂入/句柄继承全部重做），改动远超问题本身，且控制台窗口的创建/隐藏存在竞态窗口闪现。
- **方案 D：`chcp` 行维持不动（不选）**。`CREATE_NO_WINDOW` 下进程无控制台句柄，`chcp 65001` 会失败并向 stderr 写错误，污染工具输出；必须 `>nul 2>&1` 静默吞掉。

## 决策

Windows 上所有控制台子进程 spawn（`ProcessManager::start_inner`、`jobs::job_start`、`terminal::terminal_open`、`git_search::git_diff_remote`）统一调用 `crate::agent::process::hide_console_window` 设置 `CREATE_NO_WINDOW`，杜绝可见控制台窗口。批处理脚本 `chcp 65001` 行改为 `>nul 2>&1`，容忍无控制台场景。进程输出统一经 `decode_process_output` 解码：严格 UTF-8 优先，失败时 Windows 按系统 ANSI→OEM 代码页转码（GBK 等），非 Windows 保持 lossy。Job Object、管道采集、沙箱门禁与非 Windows 行为不变。

## 影响

- 用户可见：Run/Job/Terminal/git 不再弹控制台窗口；中文输出在无控制台下仍可读（GBK 正确转码），UTF-8 直通行为不变。
- 契约：`process` 模块新增 `pub(crate)` 的 `hide_console_window` 与 `decode_process_output`，供 jobs/terminal/git_search 复用；`windows-sys` 增加 `Win32_Globalization` 特性（非新依赖类别）。
- 验证：Linux 侧 `cargo check` + 单测（批处理逐字保留事故命令、chcp 2>&1、解码不 panic）通过；Windows 特有路径（`CREATE_NO_WINDOW` 效果、GBK 转码）需 Windows 构建/CI 实测确认。
