# PA-076 Phase 6 Review — Web/SSRF 加固与 Search/Glob 加固（task 6.6）

## 审核对象

- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-and-expand-agent-tool-runtime/design.md)（Decision 8/9、Verification Strategy line 141 验收项）
- 阶段 6 产物（task 6.1-6.5 + 工作项 2：P1-5/P2-7 pinned connector）：
  - `crates/pony-agent-core/src/agent/web_access.rs`（`WebAccessPolicy`/`PinnedConnector`/`WebAccessDecision`/`WebAccessDenyReason`，纯 hermetic 决策面）
  - `crates/pony-agent-core/src/agent/tools.rs`（**重点**：`web_fetch` pinned 路径 :1532-1628、`build_pinned_web_client` :4841-4867、`ReqwestPinnedSender` :4890-4957、`PinnedHttpExchange` :4766-4793、`verify_pinned_peer` :4875-4888、`pinned_web_fetch` redirect 循环 :4964-5010、`FailClosedResolver` :5171-5180、`with_web_resolver` 注入点 :954-964；新增测试 :7937-8475）
  - `crates/pony-agent-core/src/agent/search.rs`（`SearchEngine`：regex/globset/ignore 语义、确定性排序、诚实截断）
  - `crates/pony-agent-core/src/agent/tool_runtime.rs`（`WebResolver`/`FakeResolver` ports）
- 相关：`Cargo.toml`（reqwest 0.12，features 无 gzip/brotli/zstd）、`Cargo.lock`（url 2.5.8、ignore 0.4.31、globset 0.4、regex 1）

审核为只读；未修改任何代码。测试证据独立复跑（见「测试证据」）。

## 审核阵容与结论

| 角色 | 结论 | 关键发现 | 处置 |
| --- | --- | --- | --- |
| `@security-reviewer` | **CONDITIONAL PASS** | redirect 全链无总 deadline（design Decision 8 / tasks 6.2/6.3 显式要求的验收项缺失，`retry_tool_timeout` 的 `total_budget_ms` 为死配置）；字面 IPv6 URL 的 resolve 覆盖注册无效键（无害）；IP 分类缺 6to4/CGNAT 等边缘类 | **P1-1 需本轮或记录承接**；P3 逐条记录 |
| `@performance-reviewer` | **CONDITIONAL PASS** | 单跳有界（timeout/2MiB 流式/redirect≤5/content-type 门禁）均核实成立；但全链聚合 deadline 缺失导致最坏 ~12 分钟阻塞；`glob_files` 无时间预算；Search 截断边界依赖 walker 顺序 | P1-1；P2-1/P2-2 记录承接 |

无 P0。两视角均确认：**pin 是结构性强制**——`build_pinned_web_client` 用 `resolve_to_addrs` 把 hostname 的 DNS 覆盖到已校验地址（reqwest 源码 async_impl/client.rs:2278-2283 确认 port 0 = 用 URL 端口），字面 IP 无 DNS 步骤天然 pin；空地址集拒绝构建 client（不回退环境 DNS）；peer-IP 校验为纵深防御；`no_proxy()` 关闭 ambient proxy；弱模式（「预校验后交给默认 client 重解析」）已整体移除，`build_web_client` 仅剩 `web_search`（固定可信 Exa 端点）使用。

## 已核实的健壮点

1. **pin 结构性成立，无 TOCTOU/rebinding 窗口**：`pinned_web_fetch` 对每个 URL（初始 + 每跳 redirect）先 `connector.validate_initial/validate_redirect`（经注入 `WebResolver` 解析并逐记录过 `classify_ip`），再把决策的 `resolved_addrs` 经 `resolve_to_addrs` 覆盖进每跳新建 client（tools.rs:4849-4863）。连接永不经过第二次 DNS 解析；即使解析器在连接前被 rebinding 改写，client 也只会连到校验时的地址。测试 `reqwest_pinned_sender_does_not_fall_back_to_ambient_dns`（tools.rs:8412-8443）证明不回落环境 DNS。
2. **redirect 每跳重解析、重校验、重 pin**：`is_redirect()` 仅在 3xx **且** 有非空 `Location` 时 follow（tools.rs:4790-4792；3xx 无 Location 视为终态）；`resolve_redirect_target` 用 `base.join`（相对 URL 正确解析，测试 tools.rs:8149-8176）；预算 ≤5 在 `validate_redirect`（web_access.rs:142-157）与 `validate_chain`（:162-188）双层强制，第 6 跳 deny `too_many_redirects`；redirect 到私网/不支持 scheme/受限端口均按跳号拒绝。redirect 跳的 body 永不读取（tools.rs:4925-4933），无未界定读。
3. **默认 resolver 下 hostname fail-closed**：`FailClosedResolver` 拒绝一切 host（tools.rs:5174-5180）→ `resolution_failed` → `web_access_denied`；测试 tools.rs:7341、8284。字面公网 IP 被 pin（无 DNS 步骤）且被允许（测试 tools.rs:8446-8475）；字面私网/回环/链路本地/unspecified/IPv4-mapped 均免 DNS 拒绝。
4. **无弱模式、无 ambient proxy**：`web_fetch` 唯一出口是 `pinned_web_fetch`；两处 client（`build_web_client` :4747、`build_pinned_web_client` :4849）均 `.no_proxy()` + `redirect(Policy::none())`。diff 确认「先校验后交默认 client」路径已删除。
5. **body 流式读取 + content-type 门禁**：终态跳先 `is_text_content_type` 门禁再 `read_bounded_body`（cap 2 MiB，tools.rs:5096-5116），截断诚实上报 `truncated`+原因。`buf.len().saturating_add(chunk.len())` 的切分无越界。
6. **错误码机器可读**：`web_access_denied`（带 `accessDecision` 结构化 deny reason）、`resolution_failed`、`connection_pin_violation`、`unsupported_content_type`、`timeout`、`request_failed`、`read_body_failed`、`client_build_failed` 均透出为 `error.code`（tools.rs:5185-5243）。
7. **Search/Glob 语义替换完整**：regex（literal 模式先 `regex::escape`）/globset（`literal_separator`+basename 回退）/ignore（`.gitignore`/`.ignore`、`follow_links(false)`、hidden 默认跳过、`.parents(false)` 保持 hermetic）；结果排序后输出；截断均带 `truncated=true`+原因。
8. **测试独立复跑全绿**：`agent::search::` 11/11、`agent::tools::tests::` 91/91、`agent::web_access::` 22/22，0 失败（含 redirect 链、peer 校验、字面 IP、默认 fail-closed 等）。

## 发现与分级

### P0

无。

### P1

#### P1-1 redirect 全链无总 deadline（design Decision 8 / tasks 6.2、6.3 验收项缺失）

- **位置**：`tools.rs:4964-5010`（`pinned_web_fetch` 循环，每跳各建带 `timeout_ms` 的 client）；`tools.rs:5059-5092`（`retry_tool_timeout` 的 `BackoffConfig { total_budget_ms: 30_000 }`（:5068）字段从未被检查，循环只按 `max_retries` 计数）；`tools.rs:1543-1548`（`timeoutMs` clamp 1..=60_000）。
- **设计/任务要求**：design.md Decision 8 明确「正文…限制压缩前字节、压缩比、headers、redirect、**总 deadline** 和慢速响应」；tasks.md 6.2「redirect re-resolution, **total deadline**」、6.3「redirect/time/body/compression/header budgets」。当前只有每跳独立 `client.timeout()`（覆盖单请求「连接+首字节+正文」），**没有跨跳聚合 deadline，也没有跨 2 次重试的 deadline**。
- **失败场景**：恶意/慢速服务器链把每个 redirect 跳拖到接近 timeout（< timeout_ms 的 trickle），6 跳可占满 6×timeout_ms；随后 `retry_tool_timeout` 检测到超时又整体重试一次。最坏墙钟 ≈ 2 次 ×（1 初始 + 5 跳）× timeout_ms + backoff：默认 15s 下 ≈ 3 分钟，clamp 60s 下 ≈ 12 分钟。`web_fetch` 经 `block_on` 同步阻塞调用线程（runtime_helper.rs:4-12），该窗口内代理 turn 被钉死；这是设计明确要求但完全缺失的资源上界。
- **建议处置**：在 `pinned_web_fetch`（或 `web_fetch` 层）引入全链聚合 deadline（例如 `total_budget = max(timeout_ms, K × timeout_ms)` 或独立参数），把已消耗时间传入每跳（用 `RuntimeClock`/`Instant`），超限返回结构化 `timeout`；并让 `retry_tool_timeout` 真正执行 `total_budget_ms`（或删除该死字段）。补一条链级总 deadline 测试（FakePinnedSender 各跳合计超预算即失败）。

### P2

#### P2-1 Search/Glob 截断边界依赖 walker 顺序，「确定性」仅对输出排序成立

- **位置**：`search.rs:358-371`（`build_walker` 未设置 `sort_by_file_path`）；`ignore` 0.4.31 `WalkBuilder` 默认 `sorter: None`（walk.rs:569），条目按 `fs::read_dir` 顺序（OS 相关、目录变更即变）；`search_text` 的排序（search.rs:270）与 `glob_files` 的排序+dedup（:321-327）只在**输出集合**上保证顺序。
- **失败场景**：`max_files`/`max_total_bytes`/`time_budget` 任一触发截断时，先扫到哪些文件、`scanned_files`/`scanned_bytes` 数值、命中哪些匹配、`truncation_reason` 都随 readdir 顺序变化。`results_are_deterministic_across_runs`（search.rs:599-626）只测了不截断的小树，未覆盖该场景。对同一输入、同一根目录，不同平台/不同时刻可能返回不同截断边界——与「结果确定性 + 诚实截断」的完整语义存在偏差。
- **建议处置**：`build_walker` 增加 `.sort_by_file_path(|a, b| a.cmp(b))`（保持 threads(1)），让截断边界也确定；补一条「max_files 截断下两次运行完全相等」的测试。

#### P2-2 `glob_files` 无时间预算

- **位置**：`search.rs:286-334`。`glob_files` 复用 `SearchOptions::default()`（含 `time_budget_ms: Some(5_000)`），但循环体从未调用 `budget_expired`，只有 `MAX_GLOB_WALK_FILES = 100_000` 文件数硬上限。
- **失败场景**：大工作区（尤其网络盘/冷存储）上 glob 可长时间占用工具线程，无时钟兜底。与 `search_text` 的时间预算行为不一致（`search_text` 在每次文件遍历前及每 256 行检查预算）。
- **建议处置**：在 `glob_files` 循环内加 `budget_expired` 检查（按文件粒度即可），截断置 `truncated=true` + `time_budget_ms` 原因。

#### P2-3 测试覆盖缺口：真实 socket 多跳 redirect、3xx 无 Location 终态、redirect 到字面私网 IP

- **位置**：redirect 重校验逻辑只在 `FakePinnedSender` 脚本驱动下测试（tools.rs:8024-8176）；`ReqwestPinnedSender` 的真实 socket 测试只有单跳（tools.rs:8351-8443）；`is_redirect()` 的「3xx 无 Location 视为终态」分支（tools.rs:4790-4792）无测试；redirect 到**字面私网 IP**（如 `Location: http://10.0.0.5/`）无链级测试（现有 redirect-to-private 用的是 hostname `internal.example`，经 DNS 拒绝）。
- **影响**：功能正确（代码路径清晰、web_access 策略级测试覆盖多 A/私网），但真实发送器在「每跳 peer 校验 + Host/authority 保留 + redirect 跳不读 body」的集成行为、以及终态判定分支，缺 socket 级证据。
- **建议处置**：用手工构造 `WebAccessDecision`（指向 127.0.0.1 本地测试服务器，同 tools.rs:8384-8390 手法）驱动 `ReqwestPinnedSender` 打一条 302→200 链，断言每跳 peer_ip ∈ resolved 集合、最终 body 为终态内容；补 3xx-无-Location 与 `Location: http://10.0.0.5/` 两个用例（后者走 `FakePinnedSender` 或策略级均可）。

### P3

- **P3-1** `web_access.rs:406-448` IP 分类未覆盖：`100.64.0.0/10`（CGNAT）、`198.18.0.0/15`（benchmark）、`192.0.0.0/24`、IPv6 `2002::/16`（6to4，内嵌 IPv4 私网可达 127.0.0.1）、Teredo。实际 SSRF 风险低（6to4 默认不路由、CGNAT 非回环语义），建议补分类作纵深。
- **P3-2** `PinnedHttpExchange.peer_ip` 标 `#[allow(dead_code)]`（tools.rs:4773-4775）：peer 校验发生但证据不落结果/telemetry，SSRF 取证价值丢失；建议在 `web_fetch` 输出透出 `peerIp`（或 trace 事件）。
- **P3-3** `web_fetch` 成功路径 `summary.text` 用原始 `url` 而非 `final_url`（tools.rs:1623），重定向后摘要指向误导。
- **P3-4** 字面 IPv6 URL：`url.host_str()` 返回带方括号串 → `hostname.parse::<IpAddr>()` 失败 → 走 hostname 分支注册一个**永不使用**的 `resolve_to_addrs` 覆盖键（tools.rs:4854；字面 IP 直连不经 DNS，键无效但无害）。web_access.rs:327 注释正确。建议在 `build_pinned_web_client` 内先剥括号再判字面 IP。
- **P3-5** `build_pinned_web_client_pins_domain_with_resolve_override`（tools.rs:8224-8239）只断言 client 可构建，无法检查 resolve map（reqwest 不暴露）；pin 保证是结构性推论而非直接断言。可注入自定义 `dns_resolver`（实现 `Resolve`）断言收到覆盖地址，或记录为已接受的证据缺口。
- **P3-6** `web_search` 的 `response.text()`（tools.rs:1727）无界读取（受 client timeout 与固定可信 Exa 端点约束；legacy 遗留）。建议后续给 `text()` 加 cap（复用 `read_bounded_body`）。
- **P3-7** `is_tool_timeout_message`（tools.rs:5052-5057）靠子串匹配触发重试，依赖 transport 错误消息含字面 `timeout:` 前缀（tools.rs:4908）；脆弱但当前可用，改结构化 `WebFetchTransportError` 判别更稳。
- **P3-8** redirect 未禁止 https→http 协议降级（策略允许双 scheme）。agent 不携带 cookie/凭据（policy 拒绝 credentials、无 cookie store），泄露风险低；如要加固可对 https 初始 URL 拒绝降级到 http 的后续跳。
- **P3-9** `search_text` 读文件用 `fs::read_to_string`（search.rs:236），与 `metadata.len()` 检查之间存在本地 TOCTOU（文件在检查后被换大为超大文件 → 无界读入内存）。低概率本地竞态；可改用 `fs::File` + 手动 `take(max_bytes_per_file)` 读。
- **P3-10** 其余测试缺口：redirect 到多 A/AAAA 的链级用例（策略级已覆盖 web_access.rs:614-642）、redirect 到受限端口链级、协议相对 `Location: //host`、DNS rebinding「时序」（结构性防住但无直接断言）、search 中途时间预算推进（现仅 `Some(0)` 立即截断）、symlink 不跟随、hidden 默认排除、`*.localhost` 通配拒绝链级——均建议后续补测。

## 测试覆盖矩阵

| 验收维度（design.md line 141） | 覆盖 | 缺口 |
| --- | --- | --- |
| literal/private DNS | web_access.rs 单测（loopback/私网/link-local/0.0.0.0/::1/::ffff）；tools.rs 端到端 fail-closed | — |
| 多 A/AAAA（任一 forbidden 即拒） | web_access.rs:614-642（初始 URL） | redirect 链级多 A 未测（P3-10） |
| DNS rebinding | `reqwest_pinned_sender_does_not_fall_back_to_ambient_dns`（tools.rs:8412）+ resolve 覆盖结构性防住 | 无直接断言「连接即已校验地址」（P3-5）；时序用例无（P3-10） |
| peer-IP | `verify_pinned_peer` 单测 ×3 + 真实 socket 断言 peer_ip（tools.rs:8404） | `peer_ip` 证据不透出（P3-2） |
| redirect-to-private | 链级 deny（tools.rs:8098；hostname 私网）；web_access.rs:846（chain） | redirect 到字面私网 IP 未测（P2-3） |
| oversize | `read_bounded_body` 2 MiB cap + 截断上报（tools.rs:5096） | 无 socket 级 2 MiB 截断用例 |
| compression | reqwest 未启用 gzip/brotli/zstd → 无解压路径，2 MiB cap 即 wire 字节 cap | 「压缩前字节/压缩比」要求因无压缩而自然满足；gzip 响应将按原始字节解码（legacy 行为，P3-6 相关） |
| binary/unsupported type | content-type 门禁（tools.rs:5120-5140）+ 结构化错误 | — |
| timeout | 单跳 client timeout；timeout 错误码（tools.rs:5218） | **全链总 deadline 缺失（P1-1）** |
| proxy policy | 两处 client `.no_proxy()` | — |
| Search regex/glob/ignore | search.rs 11 项（gitignore/glob/预算/captures/确定性/二进制跳过/非法 regex-glob/非目录根） | glob 时间预算（P2-2）；截断边界确定性（P2-1）；TOCTOU（P3-9） |

## 测试证据

- 独立复跑：`cargo test -p pony-agent-core --lib agent::web_access::` → **22/22 通过**；`agent::search::` → **11/11 通过**；`agent::tools::tests::` → **91/91 通过**（含 7937-8475 全部 pinned/peer/redirect/字面 IP/默认 fail-closed/socket 用例）；0 失败。
- 审查期间核对的第三方语义：reqwest `resolve_to_addrs` port 0 语义（async_impl/client.rs:2274-2283）；`ignore::WalkBuilder` 默认不排序（walk.rs:569）；`url::host_str()` IPv6 返回带括号串（lib.rs:1162-1168）。

## 结论

**CONDITIONAL PASS**。

- **P1-1（全链总 deadline）为硬条件**：design Decision 8 与 tasks 6.2/6.3 的验收项明确要求「总 deadline」，实现完全缺失（`retry_tool_timeout.total_budget_ms` 为死字段），最坏可阻塞单次工具调用 ~12 分钟。需在阶段 6 收口前实现（链级聚合 deadline + 让重试预算生效 + 链级总 deadline 测试），或在本卡结论显式记录承接点与到期时间。
- P2-1/P2-2/P2-3 记录并绑定承接：P2-1/P2-2（Search 确定性/时间预算）随 Search/Glob 加固收口；P2-3（测试补强）随 P1-1 的链级测试一并补齐。
- 其余 P3 逐条记录：IP 分类补全（6to4/CGNAT）、peer 证据透出、IPv6 字面 URL 覆盖键清理等。
- **SSRF/pin 主体判定为健壮**：pin 结构性成立（无 rebinding 窗口）、redirect 每跳重校验、默认 fail-closed、字面 IP 被 pin、无弱模式、无 ambient proxy、错误码机器可读；无 P0。
