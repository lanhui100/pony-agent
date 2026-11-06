# wave-65 审批可恢复：tmux 真机复验证据

环境：`pony` v0.1.155（target/debug/pony，commit 25bd3a95 工作树），tmux 3.4，工作区 `/tmp/pa-approval-demo`
（与 `/home/dm/pony-agent` 不同 ⇒ 任何指向 pony-agent 的绝对路径 Read 都属“工作区外读取”）。

| 文件 | 时刻 | 内容 |
|---|---|---|
| `01-approval-panel.txt` / `.ansi.txt` | 23:54 | 输入「请用 Read 工具读取绝对路径 /home/dm/pony-agent/Cargo.toml 的前 3 行（不要用 Run）」后弹出**权限审批请求**面板：说明行含「工具 Read 需要读取工作区外路径 …」、目标行 `/home/dm/pony-agent/Cargo.toml`，三个选项 AllowOnce / AllowSession / Deny |
| `02-approved-continuation.txt` | 23:55 | 按 Enter（AllowOnce）后：`✓ 读取 (Read) · 读取 /home/dm/pony-agent/Cargo.toml 前 3 行` 成功，模型给出 Cargo.toml 前 3 行答案（`[workspace]` / `members = [` / `"crates/pony-agent-core",`） |
| `03-approval-panel-second.txt` | 23:58 | 第二次对 `/home/dm/pony-agent/package.json` 发起越界 Read，同样唤起审批面板（可复现） |
| `04-denied-continuation.txt` | 23:59 | 按 Esc 拒绝后：本轮不判死，宿主续轮带「用户拒绝了该工具调用」文案，模型改道并反问你如何继续 |

机器收据：
- 授权落盘：`store_metadata.path_authorizations.v1 = [{"path":"/home/dm/pony-agent/Cargo.toml","grantedAtMs":1793980454052,"scope":"read"}]`
- 续轮终态：`~/.pony-agent/tui-logs/tui-runtime.log` 中 `turn-approval-resume-e946f53d…` → `turn.completed`（seq 114, text_len=195），`decision:Continue run_phase=Ready`
- 修复前同场景：整轮 `turn:failed` + `approval_resume_unavailable`（会话 pa_01a1b122 的 turn-2e0eb0fe）
