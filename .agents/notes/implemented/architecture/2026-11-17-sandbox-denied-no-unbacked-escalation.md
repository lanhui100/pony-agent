# Agent Note: 沙箱拒绝不再承诺「可升级」——只给可执行替代路径

Status: implemented

## Problem

`outside_workspace_write_denied` 的文案硬编码 `[sandbox: escalation available — retry with explicit authorization]`（`agent/path_permission.rs`）。现场核对（会话 `pa_01a1df2f-8315`）：

- `Write /tmp/err_scan.py` 返回 `{kind: outside_workspace_write_denied, message: "…已拒绝写入。 [sandbox: escalation available …]"}`；
- 同会话 `pending_asks` 查询为 **0 条**：运行时根本不产生可被用户批准的升级请求；
- 于是模型把这条当作普通 tool_result 原样重试，再次被拒，最终被上游超时吞掉。

后果不是「提示不友好」，而是**承诺了不存在的能力**：模型据此选择重试而不是改写路径，用户也没有任何可点的审批入口。B223 曾在该承诺之外补了可写目录提示，但承诺本身没被移除——留着就是误导源。

## Decision

在「真的发起审批」与「删掉承诺」之间选后者（前者要把 HostControlPlane/Ask 生命周期接进工具执行路径，收益远小于成本，且 Ask 面板已有独立入口）：

1. `outside_workspace_write_denied(message)`（无 root 上下文）：追加**通用可执行指引** `请改用工作区内的路径（工作区根下的 .tmp 受控可写目录）后重试。`，不再追加 escalation 提示。
2. `outside_workspace_write_denied_with_workspace(message, root)`：保留 B223 的受控可写目录 `<root>/.tmp` 具体路径 + 改写建议，同样移除 escalation 提示。
3. 删除 `ESCALATION_HINT` / `ESCALATION_MARKER` / `with_escalation_hint`（无消费者的残留常量不留）。
4. 结构化错误码 `outside_workspace_write_denied` **不变**（前端与 ACP 依赖该码做分支）。
5. 同步修订 B223 验收测试：原「escalation 必须出现一次 / 必须结尾」的断言改为「**不得出现** + 必须有可执行指引」，并新增幂等断言（已带可写提示的文案二次经过构造器不得叠加）。

## Alternatives considered

- **真的发起 HostControlPlane 审批，approve 后按显式授权重试**：语义最完整，但需要把 Ask 的发起/等待/超时/答复回填接进工具执行的内层路径（工具层目前只有同步返回的 `ToolResult`），改动面覆盖 runtime、control plane 与 TUI/桌面两端；而「改写路径」在绝大多数场景已足够（受控 `.tmp` 就在工作区内）。作为后续独立需求（如确实需要跨边界写）再立法。否决。
- **保留承诺但在文案里注明「当前版本无审批入口」**：等于自己声明是假承诺，噪音更大，且模型仍会尝试升级。否决。
- **只改文案不动测试**：B223 的验收断言会锁死旧行为，改文案必须同步改断言，否则门禁红 —— 契约变更必须同提交更新验收口径。否决。

## Consequences

- 模型拿到的拒绝理由变成「往哪写 + 下一步做什么」，不再有「也许能升级」的错误暗示；工具重试路径不再浪费一轮上游请求。
- 若将来真的实现跨边界写审批，可复用 `PermissionErrorCode::RequiresAuthorization` 这条**已有**码（它与 `outside_workspace_write_denied` 本就分离），不必改动本条契约。
- 回归口径：`cargo test -p pony-agent-core --test b223_write_denied_hint_acceptance`（含 `b204_legacy_constructor_never_promises_escalation`）、`cargo test -p pony-agent-core --lib path_permission`。

## Related

- B223（受控可写目录提示，本条保留其可执行部分、移除其无落点承诺）
- `2026-11-17-turn-end-reason-interrupted.md`（同一次事故的终态三分）
