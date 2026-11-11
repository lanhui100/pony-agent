# 0037 废除 PONYSENTRY_ENABLED：开关恒开，门禁由凭证承担

Status: implemented

## 背景

wave-18 将 PonySentry 改为 fail-closed（默认零 egress）：`PonySentryConfig::default().enabled == false`，
`from_env()` 仅在显式设置 `PONYSENTRY_ENABLED=true` 时启用。该契约在桌面端产生了实际故障：
用户在 Trace tab 手动上传 trace 时收到 `PonySentry is disabled`。

根因是发布链路的配置断层：

1. release.yml 只注入了 `PONYSENTRY_CLIENT_TOKEN`，没有注入 `PONYSENTRY_ENABLED`；
2. 更根本地，Tauri 桌面端运行时配置完全依赖 `PonySentryConfig::from_env()` 读取**最终用户机器**的环境变量，
   CI 构建机的 env 对已发布产物不可见。wave-18 禁止硬编码凭证是正确的，但没有为桌面端设计
   凭证分发通道，导致发布产物永远处于 disabled 状态。

用户决策（2026-10-18）：修复，且默认 `ponysentry_enabled` 为开，**不保留该开关变量**。

## 候选方案

1. **保留 `PONYSENTRY_ENABLED`，release.yml 注入 `PONYSENTRY_ENABLED=true`**：改动最小，
   但 CI 的 env 仍进不了用户机器，问题原样存在；要求用户手动设环境变量则把配置负担转嫁给终端用户，否决。
2. **桌面端增加用户可见设置项（设置文件/Tauri store 持久化 token 与开关）**：交互最完整，
   但超出本次最小闭包，token 分发仍需先解决；作为后续演进方向保留。
3. **废除开关变量 + 构建期烧录凭证（选定）**：`enabled` 恒 true，fail-closed 语义改由
   「client_token 是否配置」承担；token 优先级为运行时 env `PONYSENTRY_CLIENT_TOKEN` >
   构建期 `option_env!("PONYSENTRY_BUILD_CLIENT_TOKEN")`（CI 烧录）。零 egress 性质不变：
   无 token 时 worker 线程不 spawn、四处 `capture_*` 早退、`send_agent_trace_direct` 返回 Err。

## 决策

- `config.rs`：删除 `PONYSENTRY_ENABLED` 解析；`default().enabled == true`；token 链路
  env > `option_env!`，均空白则 `None`。
- `client.rs`：sender 构造与四处 capture 早退的门禁改为 `!enabled || client_token.is_none()`。
- `mod.rs`：`send_agent_trace_direct` 无 token 时返回 `Err("PonySentry client token is not configured")`。
- `release.yml`：新增 `PONYSENTRY_BUILD_CLIENT_TOKEN: ${{ secrets.PONYSENTRY_CLIENT_TOKEN }}`，
  并在 Verify secrets 步骤将其缺失列为 hard fail。
- `.env.example`：移除 `PONYSENTRY_ENABLED` 条目。
- `tests/env-example.spec.ts`（契约迁移）：C6-2 变量清单由 `PONYSENTRY_ENABLED` 改为
  `PONYSENTRY_BUILD_CLIENT_TOKEN`；C6-5 由「必须写明 `PONYSENTRY_ENABLED=true`」改为
  「模板**不得**出现任何 `PONYSENTRY_ENABLED=` 赋值 + fail-closed 原则声明 + 最小配置仅需
  token + 构建期烧录通道存在」。红相基线（wave-18b）随之失效并由本文件记录的契约取代。
- 测试：credential_gate / telemetry / manual_upload / adversarial 及 acceptance 镜像同步迁移为
  wave-18c 契约（开关恒开、变量取值不影响行为、无 token 时零 egress 的出口探针断言保留）。

## Alternatives considered

- 方案 1（仅补 CI env 注入）被否决：无法修复已发布产物的运行时配置断层，属假修复。
- 方案 2（桌面端设置 UI）被否决出本次闭包：token 烧录已解决分发问题，设置 UI 属体验增强，
  待后续 backlog 立项，不在本次引入额外交互面。
- 保留 enabled=false 作为「紧急止血开关」被否决：该场景可用 `PONYSENTRY_CLIENT_TOKEN=""`（空白
  ⇒ None ⇒ 零 egress）等价实现，无需第二个语义重叠的变量。

## 验证

- `cargo test -p pony-agent-core --test ponysentry_credential_gate --test ponysentry_telemetry_test
  --test ponysentry_manual_upload_test --test ponysentry_credential_gate_adversarial --test ponysentry_trace_test`
  → 5 个目标全绿（7+10+4+10+7 passed, 0 failed）。
- `cargo check -p pony-agent-core --bins --lib` → exit 0。
- `.github/workflows/release.yml` 经 YAML 解析校验通过。
- 已知与本次无关的预存问题：`--lib` 中 11 个测试失败与 `tool_surface_wave20_contract` 的
  `-D warnings` unused import 失败，在干净 HEAD（6af176e）上以 stash 对照法复现确认。
