# TUI E2E 数据目录隔离调查总结

## 现状分析
当前 TUI 工具主要依赖 `XDG_*` 环境变量和 `dirs-rs` 库（如 `dirs::config_dir()` 和 `dirs::data_local_dir()`）来定位配置文件和数据存储目录。

在 Windows 平台上，`dirs-rs` 库使用原生 `SHGetKnownFolderPath` API（即 Known Folder API）来获取 `%LOCALAPPDATA%` 和其他路径。此 API **不直接响应** 进程环境变量（`LOCALAPPDATA`、`APPDATA` 等）的修改，因此在测试中仅通过 `std::env::set_var("LOCALAPPDATA", ...)` 来隔离数据目录是无效的。这导致部分 E2E 测试在 Windows 上运行时，会意外读写真实的用户数据目录。

## 主要依赖点
1. **持久化存储**：`sessions.db` 等文件通常存储在 `dirs::data_local_dir()` 下，Windows 上为 `AppData\Local\PonyAgent\sessions.db`。
2. **配置存储**：`tui-preferences.json` 等文件存储在 `dirs::config_dir()` 下，Windows 上为 `AppData\Roaming\PonyAgent\`。
3. **日志存储**：Tauri 解析 `app_data_dir()` 得到 `%LOCALAPPDATA%\PonyAgent\logs\pony-agent.log`。

## 稳健隔离建议
为了彻底解决 E2E 测试中的数据泄露与交叉污染问题，建议放弃「尝试重定向系统原生 API 返回值」的思路，转而使用显式的依赖注入：

### 1. 引入持久化根路径注入 (Path Injection)
在 `crates/pony-agent-core` 中定义一个全局或作用域范围的配置结构，允许通过环境变量直接注入特定目录路径，例如 `PONY_AGENT_DATA_DIR` 和 `PONY_AGENT_CONFIG_DIR`。

### 2. 运行时配置优先顺序
修改所有目录解析逻辑，强制遵守以下优先级：
1. 显式环境变量（如 `PONY_AGENT_DATA_DIR`）
2. 其他逻辑（XDG 环境变量或默认系统路径）

**示例逻辑：**
```rust
pub fn resolve_data_dir() -> PathBuf {
    std::env::var("PONY_AGENT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::data_local_dir().join("PonyAgent"))
}
```

### 3. 测试适配
E2E 测试在启动被测二进制文件时，只需注入 `PONY_AGENT_DATA_DIR=<tmp_sandbox>` 即可彻底隔离，不再依赖对 `LOCALAPPDATA` 或 `XDG_*` 的修改，也无需处理 Windows Known Folder API 的硬限制。
