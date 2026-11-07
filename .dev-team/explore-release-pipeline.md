1. 版本号真值链：`.version.json` 包含各组件（core/tauri）的 `{major, minor, patch}` 结构；`scripts/release.mjs` 读取该文件并 bump `core` 和 `tauri` 组件，随后通过 `updateCargoVersion` 函数原地修改 `crates/pony-agent-core/Cargo.toml`（Line 77）及其他四个文件。`Cargo.toml` 中的 `version` 参与 bump，`pony --version` 不一致可能是由于构建缓存或未完整执行完整同步导致的偏差。
2. 产物发布到哪里：`release.mjs` 仅负责 Bump 版本、Git 提交和打 Tag（Line 115-122），不具备构建或上传产物的逻辑。仓库内不存在发布分发步骤。
3. `s3.local.ponyjob.top`：出现于 CSP（dist/index.html:31）、Fallback 检查链接（`crates/pony-agent-core/src/bin/tui.rs:2083`）及更新检查（`src/lib/update-check.ts:20`）。推断为生产环境的 release 版本检查和分发点，在当前测试环境中被桩处理以通过 E2E 门禁。
4. 安装器 / 下载器：仓库内无 `install.sh` 或自举下载脚本。存在 `npm run cli:build` 命令（`package.json:7`）用于本地手动构建，安装器相关 UI 代码已实现（PA-103），但在生产环境处于禁用态。
5. GitHub 仓库：存在 `.github/` 目录；`scripts/release.mjs` 推送 `main` 和 `tags`（Line 126），触发 GitHub CI。
6. 跨平台打包：`scripts/` 下无交叉编译脚本；tauri 打包依赖 `src-tauri/tauri.conf.json` 配置与 `npm run build`。
7. 版本处理：仓库内部实现简单 semver 解析（`crates/pony-agent-core/src/bin/tui.rs:2031` `parse_semver_triple` 及 `:2044` `is_newer_semver`），未引入外部 semver 库。

覆盖盲区：未实际验证 GitHub CI 中是否存在对应的构建发布流，且仅凭仓库内脚本排除发布逻辑。
