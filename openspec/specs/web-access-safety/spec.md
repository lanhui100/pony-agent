# web-access-safety Spec

## ADDED Requirements

### Requirement: Web access SHALL validate every network target

Pony Agent SHALL 在初始请求和每次 redirect 前验证 scheme、host、resolved addresses、port 与 credentials。

#### Scenario: A URL targets a restricted network
- **WHEN** URL 或解析后的任一目标属于 localhost、loopback、private、link-local、unspecified 或 host policy 禁止范围
- **THEN** WebFetch SHALL 在连接前拒绝请求
- **AND** SHALL 返回结构化 network_scope_denied

#### Scenario: A public URL redirects to a private target
- **WHEN** redirect location 指向受限网络
- **THEN** WebFetch SHALL 拒绝该 redirect
- **AND** SHALL NOT 依赖 HTTP 客户端自动跟随

### Requirement: Arbitrary URL WebFetch SHALL use a pinned connector

Pony Agent SHALL 在启用任意 URL WebFetch 前使用可注入 resolver 和 pinned connector，把已验证网络目标绑定到实际连接。

#### Scenario: DNS changes after policy validation
- **WHEN** hostname 在解析与连接之间发生 DNS rebinding
- **THEN** connector SHALL 连接到已验证的地址并校验实际 peer IP
- **AND** SHALL 保留原始 authority/TLS SNI
- **AND** 每次 redirect SHALL 重新解析、验证和 pin

#### Scenario: A pinned connector is unavailable
- **WHEN** host 没有提供满足 peer-IP 校验的 connector
- **THEN** arbitrary URL WebFetch SHALL 返回 `web_fetch_unavailable`
- **AND** SHALL NOT 使用默认 client 的二次 DNS 解析作为回退

### Requirement: Ambient proxies SHALL be disabled by default

Pony Agent SHALL 不得从环境变量自动采用 proxy。

#### Scenario: A request has an ambient HTTPS_PROXY or ALL_PROXY
- **WHEN** host 未显式注入受信代理策略
- **THEN** WebFetch SHALL 忽略 ambient proxy
- **AND** SHALL NOT 让代理绕过目标地址策略

### Requirement: Web responses SHALL be streamed under explicit budgets

Pony Agent SHALL 对响应时间、redirect 次数、正文大小和解码内容设置 hard limits。

#### Scenario: A response exceeds the body limit
- **WHEN** Content-Length 或 streaming bytes 超过配置上限
- **THEN** WebFetch SHALL 停止读取并返回 response_too_large
- **AND** trace SHALL 记录实际读取量与限制

#### Scenario: A response uses compression or slow delivery
- **WHEN** 响应压缩、headers、body 或传输速率违反预算
- **THEN** WebFetch SHALL 按压缩前字节、解压后字节、压缩比、headers 与总 deadline 停止读取
- **AND** SHALL 返回结构化预算失败

### Requirement: WebFetch SHALL distinguish text from unsupported content

Pony Agent SHALL 依据 content type 与安全探测决定是否解码正文。

#### Scenario: A URL returns unsupported binary content
- **WHEN** 响应不是允许的文本内容且不属于其他专用 artifact 工具
- **THEN** WebFetch SHALL 返回结构化 unsupported_content_type
- **AND** SHALL NOT 把任意二进制按 GBK 或 UTF-8 文本展开
