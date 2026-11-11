# 0010 CI rust-cache 对齐 target 槽位与最小令牌权限

Status: implemented

## 背景

2f8b0c4 落地最小 CI（决策校验→npm ci→typecheck→vitest→cargo check），但 `Swatinem/rust-cache@v2` 未配置 `workspaces`，默认缓存根工作区的 `./target`；而按 AGENT.md 的 target 槽位硬性规则，CI 实际执行的 `cargo:check:shared` 写入 `target-check/`——缓存目录与构建目录错位，恢复/保存均落空，每次 CI 全量冷编译 Rust 依赖。同时 workflow 未声明 `permissions`，GITHUB_TOKEN 以仓库默认宽权限运行。

## 候选方案

**rust 缓存目录**

- CI 步骤改跑裸 `cargo check` 写默认 `./target`：落选——违反 AGENT.md"所有 cargo 操作必须走 npm script 固定槽位"硬性规则，且污染本地 dev 主构建槽位的语义约定。
- 移除 rust-cache，接受全量冷编译：落选——Rust 依赖冷编译分钟级耗时，随 crates 增长持续恶化，纯属浪费。
- **`workspaces: . -> target-check`（采纳）**：缓存键仍取根 Cargo.lock（workspace 根事实正确），保存/恢复对齐真实写入的 `target-check/` 槽位，与 AGENT.md 约定一致。

**GITHUB_TOKEN 权限**

- 保持仓库默认权限：落选——本 workflow 只读代码、不发布产物，宽权限违背最小权限原则。
- **顶层声明 `permissions: contents: read`（采纳）**：一行收敛令牌上限，后续新增 job 默认继承只读基线。

## 决策

- CI 中 `Swatinem/rust-cache@v2` 必须以 `workspaces: . -> target-check` 声明，使缓存目录对齐 AGENT.md target 槽位约定；未来若 CI 引入其他 cargo 槽位（如 test），须为对应槽位单独配置或显式评估。
- 本 workflow 顶层固定 `permissions: contents: read`；需要写权限的发布类 workflow 须单独声明并说明理由。
