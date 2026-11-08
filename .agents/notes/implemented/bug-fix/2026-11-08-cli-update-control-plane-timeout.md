# Agent Note: `pony update` 控制面超时与 body 读取缺陷修复

Status: implemented

## Context

`pony update` 在开发服务器（`HTTPS_PROXY=http://127.0.0.1:8899`）间歇报

```
[pony-update] phase=discover status=fail code=network
pony update 失败：网络请求失败：读取发布列表失败：error decoding response body
```

同一网络下 `curl` 拉同一 API 稳定 200（1.7~3.4s / 175KB），`pony update --check` 连跑 8 次复现 1 次（≈5.1s）。
定位到三处叠加缺陷：控制面 5s 总预算贴着实测耗时；reqwest 的 body 流错误被统一包成
`error decoding response body`（真实原因在 `source`）；body 读取在重试闭环之外。

决策正文见 ADR 0095：[`docs/decisions/0095-cli-update-control-plane-timeout.md`](../../../../docs/decisions/0095-cli-update-control-plane-timeout.md)。

## Decision

1. `crates/pony-agent-core/src/agent/updater.rs`：控制面时限 `connect 5s / 空闲读 10s / 总预算 30s`
   （`CONTROL_PLANE_READ_TIMEOUT` 新增，`CONTROL_PLANE_TOTAL_TIMEOUT` 5s→30s），客户端同步挂
   `.read_timeout(...)`。
2. 删除 `get_with_retry`（返回 `Response`、body 交给调用方），改为 `get_control_plane_body`：
   单次尝试内完成「发请求 + 读完 body」，失败分类为 `Retryable`（网络 / body 读失败 / 5xx）与
   `Fatal`（4xx），可重试项最多 2 次尝试、退避 500ms 不变。
3. 新增 `describe_error_chain`，把 `source` 链拍平成一行；`读取<标签>失败` / `请求 <url> 失败` /
   `下载流中断` 三处文案都改用它，不再只回显 reqwest 外层摘要。
4. 测试：`updater::tests` 增两条（预算契约、错误链拍平），`cli_self_update_tests` 增一条
   （截断 body ⇒ 请求 ≥2 次且文案带 `source` 链）。
5. 文档同提交同步：契约矩阵 C5、`docs/observability.yaml` 登记、`nfr-baseline-wave67.json` 的
   `download_budget`、ADR 0087 的事实指针。

## Alternatives considered

- **只调大总预算（20s 级）**：解决「慢」，解决不了「body 中断绕过重试」与不可诊断文案 —— 落选。
- **控制面套用下载的 300s 预算**：真断网时 `pony update` 会卡到不可接受 —— 落选。
- **缩小 `per_page` 压 body**：`--pin` 依赖最近 release 列表扫清单版本，缩小窗口会让旧版本 pin 静默失效 —— 落选。
- 完整比较见 ADR 0095。
