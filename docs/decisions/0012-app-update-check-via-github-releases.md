# 0012 应用内更新检测：前端直连 GitHub Releases 与构造式发布页跳转

Status: implemented

## 背景

应用此前没有任何更新感知能力，用户只能自行去仓库查看新发版。PA-099 要求在配置页提供软件更新区块，并在侧栏设置入口显示角标。GitHub 仓库为 `lanhui100/pony-agent`，以 `vX.Y.Z` tag 发版；桌面壳是 Tauri v2（WebView2），同时存在 `npm run dev` 浏览器预览模式。相关任务卡 `management/task-system/03_TASKS/PA-099-app-update-check.md`，实施 spec `openspec/changes/2026-08-24-add-app-update-check/`。

## 候选方案

**方案 A：前端直接 fetch GitHub Releases API（选定）**——`https://api.github.com/repos/lanhui100/pony-agent/releases/latest` 匿名 GET，纯逻辑层 + Pinia store + 薄组件。优点：零新依赖；浏览器预览模式与 Tauri 行为一致（该 API 返回 `Access-Control-Allow-Origin: *`，WebView2 与普通浏览器均放行）；jsdom + fetch stub 可完整测试；实现面小、无后端耦合。缺点：受 GitHub 匿名限流约束（60 req/h/IP，共享出口 IP 企业环境可能常态失效）；WebView2 若被企业代理 TLS 拦截，拿到的是内容级污染响应而非连接失败——因此响应必须按不可信输入校验（见决策 2）。

**方案 B：src-tauri 新增 reqwest 命令**（落选）——为 Tauri 壳引入 HTTP 客户端依赖树，编译时间与供应链审查面显著增加；浏览器预览模式需要另行降级路径；且本功能只需"只读 latest release"一个低频请求，为此加后端命令不成比例。落选核心原因：改动面与功能价值倒挂。

**方案 C：Tauri 官方 updater 插件（tauri-plugin-updater）**（落选）——提供签名校验的自动更新，但要求签名密钥体系与 manifest 托管流程，超出"有新版则提醒"的需求边界；接入属独立决策，未来若做应另立 ADR，不并入本特性。

## 决策

1. 检测链路取方案 A：`fetchLatestRelease`（8s 超时、`credentials:"omit"`、`cache:"no-store"`、错误六分类）+ `useUpdateStore`（缓存水合零网络启动、24h 节流、in-flight 幂等、后台静默失败保态）；无自动重试、无定时器。
2. **信任边界按"网络响应与 localStorage 缓存皆不可信"设计**：持久化只存原始快照，`hasUpdate` 由 store 用当前版本现算；API 返回的 `html_url` 全程不进入存储/状态/跳转链，发布页地址一律由通过严格格式验证的 tagName 构造（`buildReleasePageUrl`），使缓存投毒与前缀混淆攻击面对跳转无效。
3. **版本基线取 package.json `version`**（经 JSON import），与 bump-version.ps1 同步链一致；配套发布约定：**release tag 必须等于打 tag 时点的 package.json 版本（可带 `v` 前缀）**。`tauri.conf.json` 中失同步的 `0.1.0` 不作为比较基准，其修复登记为后续任务。
4. 角标只挂侧栏"设置"两个入口（amber-500 静态点）：更新卡片位于 general tab，rose 在该侧栏已是删除/失败语义。

## 影响

- 配置页新增"软件更新"卡片（手动检查、五态呈现、查看发布页、自动检查开关即隐私退出通道）；启动时若开启自动检查会向 api.github.com 发一次匿名 GET（文档级隐私披露已写入 spec 与卡片文案）。
- 后续任务（已在 PA-099 任务卡登记）：① Rust `open_url` 的 URL 白名单加固并替换 `cmd /c start`；② CSP 从 null 收紧与 markdown 消毒器评估；③ 发布流水线 owner 确认 tag↔package.json 约定并修复 tauri.conf.json 版本失同步。
