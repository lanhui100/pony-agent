# 项目工程宪章（Constitution）

本项目为 **Pony Agent**（Tauri v2 + Rust 桌面级智能体应用）。

## 核心不变量（Core Invariants）

1. **Rust 核心与前端渲染职责分离**：
   - 智能体状态流、调度引擎、工具执行沙箱、会话持久化与模型网络交互归属 `crates/pony-agent-core` 及 `src-tauri`。
   - 前端（Vue 3 / TypeScript）仅作为人机交互视图投影，模型流量一律不走前端 `fetch`。
2. **外部安全边界（Fail-Closed）**：
   - 桌面打开外部链接统一走 `src/lib/open-external-url.ts` 并在 Rust 端实施严格协议（HTTPS）与域名白名单，Windows 下强制使用 `ShellExecuteW`，禁止使用命令拼接或回退。
3. **版本四处同步与发版契约**：
   - `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 保持严格一致，由 CI 与本地脚本双重锁定。
4. **决策与规范先导**：
   - 非平凡功能改动必须有 OpenSpec 规范与对应 ADR 架构决策支持。
   - 弃用或被替代的设计必须移入 `superseded/` 或 `archive/`。
