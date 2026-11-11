# 0049 OTel 准入 token 改为构建期烧录（用户零配置）

Status: implemented

## 背景

桌面端此前**没有任何 OTel token 的分发通道**：`otel::resolve_config` 的凭证来源只有
settings（用户在「配置 → 通用」手填）与进程 env（`PONY_OTEL_TOKEN`）。
`release.yml` 只烧录了 PonySentry 的 token，**从未注入 OTel token**，
GitHub 仓库也长期没有 `PONY_OTEL_TOKEN` secret。

后果（2026-10-26 实测）：

- Tempo 里 `service.name=desktop-client` 最近 24h **0 条**，而同期探针（`pony.probe=true`）仍有数据
  ⇒ 公网 edge 与 collector 正常，断在客户端导出；
- PonySentry 的 `by_release` 停在 `0.1.136`，与「用户手填过 token、后来丢失/过期」一致；
- `otel::init()` 允许无鉴权 header 静默导出，公网 edge 401 后丢弃且**零告警**
  ⇒ 用户完全无法察觉 trace 已经停了（该可诊断性缺口已由 wave-48 的 `ExportDiagnostics` 补齐）。

## 候选方案

| 方案 | 结果 |
| --- | --- |
| A. 构建期烧录共享 token（`option_env!("PONY_OTEL_BUILD_TOKEN")`），优先级最低 | **采纳** |
| B. 首次运行向注册服务换取 per-device token | 落选（当前基础设施只有手动 `provision-otel-token.sh`，无注册端点；作为演进方向保留） |
| C. 边缘放开鉴权 | 落选（公网写入面裸奔，不可接受） |
| D. 继续要求用户手填 | 落选（实测就是这条路径失效了，且失败完全静默） |

## 决策

- 新增构建期环境变量 `PONY_OTEL_BUILD_TOKEN`，由 `release.yml` 从 GitHub secret
  `PONY_OTEL_TOKEN` 注入；core 用 `option_env!` 读取。
- **优先级：settings > env(`PONY_OTEL_TOKEN`) > 构建期 > 缺省**。
  构建期值只在「settings 与 env 都没给凭证」时生效 ⇒ 运维/用户仍可覆盖，也能立刻吊销为本地值。
- 空白/纯空格视为未配置（fail-closed，不得凭空产生凭证）。
- 实现拆成 `resolve_config_with_build_token(settings, env, build_token)` 纯函数内核，
  `resolve_config` 只是把 `option_env!` 传进去；否则 `option_env!` 在测试构建里恒为 `None`，
  「最低优先级兜底」这条契约无法被单测锁定。

## 安全取舍（必须显式承认）

- **token 内嵌在安装包里，可被逆向提取**。这与 ADR 0037 为 PonySentry token 做出的取舍同款。
- 当前公网 edge 是**单 token** 模型（`otel-public-ingest-tokens` 只含一枚 `otl_…`），
  所以「按设备吊销」本来就不成立；烧录并不会让现状更差。
- 缓解面：该 token 只能**写入**（不能读任何数据）；edge 侧有 120 req/min/IP（burst 240）
  与 16 MiB/请求上限；trace 侧靠 `ingest.client.kind=device` + `pony.app.name` 做归因，
  不依赖 token 身份。
- **吊销代价 = 重新发版**。若将来需要按设备吊销，必须走方案 B（注册端点换 per-device token）。

## 影响

- `crates/pony-agent-core/src/agent/otel/mod.rs`：新增 `ENV_PONY_OTEL_BUILD_TOKEN`、
  `build_time_otel_token()`、`resolve_config_with_build_token()` 与其单测。
- `.github/workflows/release.yml`：注入 `PONY_OTEL_BUILD_TOKEN`。
- GitHub secret：新增 `PONY_OTEL_TOKEN`（值取自集群 `observability/otel-public-ingest-tokens`）。
- 本地 `cargo build` 不受影响（`option_env!` 为 `None`，仍可走 settings/env 调试）。
