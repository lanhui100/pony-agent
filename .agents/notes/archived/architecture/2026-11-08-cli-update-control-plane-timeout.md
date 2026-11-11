# Agent Note: `pony update` 控制面改用「空闲读超时 + 30s 总预算」，body 读取纳入重试

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0095-cli-update-control-plane-timeout.md` 之归档快照，仅供溯源。

## 背景

2026-11-08 现场（本机开发服务器，`HTTPS_PROXY=http://127.0.0.1:8899`）：`pony update` 间歇失败，输出为

```
[pony-update] phase=discover status=fail code=network
pony update 失败：网络请求失败：读取发布列表失败：error decoding response body
```

同一台机、同一代理下 `curl https://api.github.com/repos/lanhui100/pony-agent/releases?per_page=20`
稳定 200（`time_total` 1.7~3.4s，body 175KB）；`pony update --check` 连跑 8 次，第 5 次在约 5.1s 处
复现同一文案，其余 3.4~6.0s 成功。三条事实叠加：

1. ADR 0087 给控制面定的是 **connect 5s / total 5s**。`releases?per_page=20` 的 body 有 ~175KB，
   经代理链路的累计耗时本就贴着 5s 线抖动 ⇒ 总预算先于任何真实故障触发。
2. reqwest 把总超时同时挂在 body 流上（`TotalTimeoutBody`），body 流错误统一由 `error::decode` 包装，
   `Display` 只给外层摘要 `error decoding response body`；真正的 `operation timed out` 只在 `source` 里。
   旧实现用 `{error}` 单层渲染 ⇒ 用户得到一句与网络/代理毫无字面关联的结论。
3. 旧实现「先取响应头、再单独 `.text()`」，body 读取落在重试闭环**之外** ⇒ 这段失败连那 1 次重试都拿不到。

## 候选方案

- **A（采纳）：按空闲读超时治理 + 留余量的总预算，body 读取挪进重试闭环，reqwest 错误一律展开 `source` 链。**
  connect 5s 保留、新增 read/idle 10s、总预算 30s；列表/清单的「网络错误 / body 读失败 / 5xx」各重试 1 次
  （退避 500ms），4xx 直接判 HTTP 状态。理由：这段链路的变量是 body 体积与代理抖动，「多久没数据」比
  「总共花了多久」更接近真实故障判据；30s 只在上限处兜底，重试次数与退出码口径（契约 C2）都不变。
- **B：只把 5s 调大（如 20s），不动重试与错误文案。** 落选：治不了「body 中断绕过重试」这条，
  也治不了 `error decoding response body` 这种不可诊断文案；下次真正的对端中断仍然只剩一句乱码。
- **C：控制面也套下载那套 300s 预算 + `tokio::time::timeout`。** 落选：控制面要的是「快速失败 + 一次重试」，
  30s 级预算已覆盖代理抖动；300s 会让真断网时 `pony update` 卡到不可接受。
- **D：减小 `per_page`（如 5）来压缩 body。** 落选：`--pin` 依赖在最近的 release 列表里扫清单版本，
  缩小窗口会让旧版本 pin 静默失效（功能回退），且完全不解决失败分类问题。

## 决策

`pony update` 控制面（releases 列表 / 清单）的时限改为 **connect 5s + 空闲读 10s + 总预算 30s**；
body 读取进入重试闭环（网络错误 / body 读失败 / 5xx 各重试 1 次，退避 500ms，4xx 直接判状态）；
所有 reqwest 错误按 `source` 链展开后再写入 `UpdateError` 文案。

## Alternatives considered

见「候选方案」B / C / D 及各自落选原因（本节按规范显式命名，内容不重复）。

## 影响

- 契约矩阵 `.dev-team/contract-matrix-cli-self-update.md` 的 C5「控制面超时」行随之改写（C5 其余行不变）；
  `.dev-team/nfr-baseline-wave67.json` 的 `download_budget` 里 `control_plane_timeout_ms` 改为总预算 30000，
  并新增 `control_plane_connect_timeout_ms` / `control_plane_read_timeout_ms` 两项（5s / 10s）。
- 机械收口：`agent::updater::tests::control_plane_budget_is_read_idle_based_not_five_second_total`
  与 `error_chain_flattens_nested_sources_into_one_line`。
- 行为收口：`cli_self_update_tests::update_retries_when_manifest_body_read_fails_and_surfaces_the_cause`
  （截断 body ⇒ 必须 ≥2 次请求，且文案必须带 `source` 链而非只回显摘要）。
- ADR 0087 的制品下载预算（300s / 上限 1800s / 不重试）与全部安全边界不变。

