# 证据包 session-e17642a6-b180-4ab4-abae-ee3260f00c36（pack_hash=41fdc7dceaa8aa57）

- 任务: wave-59 桌面端 UI/UX 四项优化（等待动画去重/计数下移/流式闪跳/状态准确性） / 分级 B / 源码 L=3749 文件 27
- 区间: d3745a7..2b5e6f20c2866f05ab4b7aeed596cd0b4e0b8357

## 提交序列
- 1b42053a04a4 index on main: d3745a7 chore(release): bump version to 0.1.168  
  files: 
- 2b5e6f20c286 WIP on main: d3745a7 chore(release): bump version to 0.1.168  200 files changed, 5498 insertions(+), 2400 deletions(-)
  files: .dev-team/evidence/tui-polish/after/add-provider/assertions.json, .dev-team/evidence/tui-polish/after/add-provider/main.ansi, .dev-team/evidence/tui-polish/after/add-provider/main.png, .dev-team/evidence/tui-polish/after/add-provider/main.txt, .dev-team/evidence/tui-polish/after/add-provider/meta.json, .dev-team/evidence/tui-polish/after/add-provider/mock.out, .dev-team/evidence/tui-polish/after/bin-info.json, .dev-team/evidence/tui-polish/after/cli-modes/assertions.json

## 交付后 7 天相关提交
- f3979e49bfd2 feat(streaming): 接入流式视觉层（唯一 inline 尾巴 + 单行遮罩浮现） files: .dev-team/contracts/wave-pds-3-streaming-visual.md, docs/decisions/0059-streaming-visual-layer-tail-node.md, docs/decisions/README.md, src/components/HomeWorkspace.vue, src/components/MarkdownRenderer.vue, src/components/chat/AgentRuntimeStatusBar.vue
- b0493690e7ca fix(core): 恢复请求保留完整上下文并补齐轮次止损与请求形状可观测 files: crates/pony-agent-core/src/agent/context.rs, crates/pony-agent-core/src/agent/jobs.rs, crates/pony-agent-core/src/agent/otel/mod.rs, crates/pony-agent-core/src/agent/runtime/limits.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/agent/runtime/stream_support.rs
- 9bcb68da9c81 fix(core): 消除 -D warnings 下的三处编译阻断（CI 与发版门禁） files: crates/pony-agent-core/src/agent/tools.rs, crates/pony-agent-core/src/agent/workspace_hotloader.rs
- b9b2538c672a test(tui-audit): tmux 真终端审计扩展至 22 场景 / 420 断言 files: .dev-team/evidence/tui-polish/after/add-provider/assertions.json, .dev-team/evidence/tui-polish/after/add-provider/extra.json, .dev-team/evidence/tui-polish/after/add-provider/main.ansi, .dev-team/evidence/tui-polish/after/add-provider/main.png, .dev-team/evidence/tui-polish/after/add-provider/main.txt, .dev-team/evidence/tui-polish/after/add-provider/meta.json
- d2b69cb5029a fix(tui): 消除崩溃面并加固生命周期、状态一致性与渲染性能 files: crates/pony-agent-core/src/bin/tui.rs, crates/pony-agent-core/src/bin/tui/protocol.rs, crates/pony-agent-core/tests/tui_acceptance.rs, crates/pony-agent-core/tests/tui_batch1_regression.rs, crates/pony-agent-core/tests/tui_batch2_regression.rs, crates/pony-agent-core/tests/tui_batch3_regression.rs
- 255a427084cd feat(desktop-web): 流式渲染稳定性优化与斜杠命令 (/plan /tasks /compact) files: .dev-team/contracts/wave-59-desktop-uiux.md, .dev-team/regression-baseline-wave59.json, .dev-team/review-join-wave59-r2.json, .dev-team/review-join-wave59.json, .dev-team/review-plan-wave59.json, .dev-team/wave59-delivery.md
- a879a168c4cc feat(core): 手动上下文压缩内核与 compact_session 命令 files: crates/pony-agent-core/src/agent/control_plane/compaction_commands.rs, crates/pony-agent-core/src/agent/control_plane/mod.rs, crates/pony-agent-core/src/agent/runtime/compaction_ops.rs, crates/pony-agent-core/src/agent/runtime/mod.rs, crates/pony-agent-core/src/bin/cli.rs, crates/pony-agent-core/tests/cli_acceptance.rs
- a0d6232e1af7 fix(observability): 修正 usage total/ttft/轮次序号/错误语义四处口径缺陷 files: crates/pony-agent-core/src/agent/otel/mod.rs, crates/pony-agent-core/src/agent/provider/mod.rs, crates/pony-agent-core/src/agent/provider/openai_sse.rs, crates/pony-agent-core/src/agent/runtime/session_ops.rs, crates/pony-agent-core/src/agent/runtime/tool_recovery.rs, crates/pony-agent-core/src/agent/runtime/trace_timeline.rs

## 机械红旗（判定必须逐条回应）
- impl_commit_modified_frozen_test:2b5e6f20c286
- self_signed_review:review-join-a.json
- self_signed_review:review-join-a2.json
- self_signed_review:review-join-b.json
- self_signed_review:review-join-b2.json
- self_signed_review:review-join-pds3.json
- self_signed_review:review-join-wave59-r2.json
- self_signed_review:review-join-wave59.json
- self_signed_review:review-join.json
- self_signed_review:review-l3-concurrency.json
- self_signed_review:review-l3-security.json
- self_signed_review:review-plan-pds3.json
- self_signed_review:review-plan-wave58.json
- self_signed_review:review-plan-wave59.json
- self_signed_review:review-plan.json

## 无法从机器判定的未知项
- 每个门禁实际抓到多少缺陷（gate_caught）需从 review JSON/测试日志判定
- 用户是否口头授权了自扩范围（scope）需读会话原文
