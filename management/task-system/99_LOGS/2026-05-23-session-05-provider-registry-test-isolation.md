# 2026-05-23 Session 05 - provider registry 测试隔离修复

## 背景

- 本轮发现 `src-tauri/tests/provider_registry_regression.rs` 使用 `ProviderRegistryStore::new()`。
- 在当前 Windows 环境下，测试里临时覆盖 `APPDATA/XDG_CONFIG_HOME` 并没有可靠隔离真实配置目录。
- 结果是集成测试把真实 `C:\\Users\\HUAWEI\\AppData\\Roaming\\pony-agent\\providers.json` 写成了测试数据。

## 本次修复

1. 在 `src-tauri/src/agent/config.rs` 为 `ProviderRegistryStore` 增加 `with_path(...)`，允许显式注入存储路径。
2. 将 `provider_registry_regression` 全部改为使用临时目录下的 `providers.json`，不再依赖环境变量重定向。
3. 新增静态回归测试：扫描 `src-tauri/tests/*.rs`，禁止测试代码再次出现 `ProviderRegistryStore::new()`。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml --test provider_registry_regression -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target-codex-provider-guard --lib --test provider_registry_regression --test session_regression --test tool_router_regression`
- `npm run verify`

以上验证均已通过。

## 备注

- 默认 `cargo test` 仍可能因为宿主进程占用 `src-tauri/target/debug/pony-agent.exe` 在 Windows 上失败；这不是本次功能回归，而是默认 target 目录文件锁问题。
- 真实 `providers.json` 已被此前测试污染，后续应在确认用户期望的 provider/model 后再恢复，不应擅自覆盖。
