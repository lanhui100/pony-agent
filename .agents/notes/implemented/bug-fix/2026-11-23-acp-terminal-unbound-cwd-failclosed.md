# Agent Note: 未绑定 workspace 的 terminal handler 拒绝自定义 cwd（B177 ⑨）

Status: implemented

## Problem

`ManagedTerminalHandler` 有两个构造口：

- `with_workspace_root(pm, root)` —— **生产路径**（`session.rs`、`orchestration.rs` 都走它），
  cwd 必须在 workspace 内；
- `new(pm)` —— `workspace_root: None`，公开可见，测试在用。

`resolve_cwd` 的边界校验原本写成：

```rust
if let Some(root) = self.workspace_root.as_ref() {
    if !canonical.starts_with(root) { return Err(...); }
}
```

`None` 时**整段跳过** ⇒ 只要显式传 `cwd`，任何绝对路径都能通过（复审实测 `cwd: "/etc"`
能真的把终端开起来）。生产不会走到这里，但 `new()` 是公开构造口——"只有测试会用"不是一条护栏。

## Decision

把校验改成 `match`，`None` 分支**fail-closed**：

- `Some(root)`：保持原有 workspace 边界判定；
- `None`：**只要调用方给了显式 cwd 就拒绝**（`Denied`，理由写明「未绑定 workspace」），
  只允许"不指定 cwd、沿用进程默认目录"。

生产路径全部走 `with_workspace_root`，因此该兜底对正常使用零影响。

## Verification

- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_capabilities_contract`
  → **12 passed / 0 failed**
- 新增 `g7c_unbound_handler_rejects_explicit_cwd`：`new()` + `cwd: "/etc"` 必须被判 `Denied`
  且文案含「未绑定 workspace」。**红相**：去掉兜底后 `create` 会真的返回 `terminalId`，用例立即 panic。
- `g7_terminal_handler_full_chain_and_no_residue`（用 `new()` 但**不带 cwd**）仍然通过
  ⇒ 兜底没有误伤"未绑定 + 默认目录"的既有用法。

## Alternatives considered

- **删除 `new()`，只留 `with_workspace_root`**：最彻底，但 `new()` 被两个集成测试使用，
  且"无 workspace 的纯逻辑测试"是合理需求；删了会把测试逼去构造假 workspace。
- **让 `new()` 从 `std::env::current_dir()` 推断 root**：隐式绑定一个"看起来像边界"的目录，
  比无边界的危害更大——调用方会以为有边界。
- **在文档里写"生产请用 with_workspace_root"**：这正是现状（文档早就这么写），
  约束靠自觉就等于没有约束。
