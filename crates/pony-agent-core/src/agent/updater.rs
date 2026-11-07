//! `pony update` CLI 自更新协议面（wave-67）。
//!
//! 契约真值源：`.dev-team/contract-matrix-cli-self-update.md`（C1 = API、C2 = 命令面、
//! C4 = 发现与重定向、C5 = NFR、C6 = 安全）。
//!
//! 本文件当前是**协议空桩**：只冻结常量、数据结构与函数签名，函数体一律 `unimplemented!()`；
//! 下载 / 校验 / 原子替换 / Release 发现等实现逻辑由 Executor 按契约填充，测试由 Test Agent
//! 在本文件末尾的 `#[cfg(test)] mod` 内冻结。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// CLI 更新清单在 release 中的资产名（发布侧与客户端唯一约定）。
pub const CLI_MANIFEST_ASSET: &str = "pony-cli-latest.json";

/// 默认更新源仓库（`owner/repo`）；可由 `PONY_UPDATE_REPO` 覆盖。
pub const DEFAULT_UPDATE_REPO: &str = "lanhui100/pony-agent";

/// 默认 GitHub API 基址；可由 `PONY_UPDATE_API_BASE` 覆盖（必须 `https://` 开头）。
pub const DEFAULT_UPDATE_API_BASE: &str = "https://api.github.com";

/// 制品下载总预算默认值（秒）：300s。
pub const DEFAULT_DOWNLOAD_BUDGET_SECS: u64 = 300;

/// 制品下载总预算硬上限（秒）：1800s；`PONY_UPDATE_DOWNLOAD_BUDGET_SECS` 越界一律回退默认。
pub const MAX_ALLOWED_DOWNLOAD_BUDGET_SECS: u64 = 1800;

/// 制品体积硬上限：128MiB。超限即 `TooLarge` 并删除临时文件（禁止整文件读入内存）。
pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

/// 当前平台的发布标识。
///
/// 取值集合与发布侧一致：`linux-x86_64` / `linux-aarch64` / `macos-x86_64` /
/// `macos-aarch64` / `windows-x86_64`。
///
/// 失败条件：目标三元组不在上述集合内（不支持的 OS/架构）⇒ 返回 `None`，调用方须映射为
/// `UpdateError::UnsupportedPlatform`，不得猜测近似平台。
pub fn current_platform() -> Option<&'static str> {
    unimplemented!()
}

/// CLI 更新清单（release 内 `pony-cli-latest.json` 的解析结果）。
///
/// 不变量：`schema` 必须等于 `"pony-cli-manifest"`；`version` 必须是可解析的语义化版本；
/// `platforms` 的每个条目都必须带 64 位小写 hex 的 `sha256`。
#[derive(Deserialize)]
pub struct CliManifest {
    pub schema: String,
    pub version: String,
    #[serde(default)]
    pub generated_at: Option<String>,
    pub platforms: BTreeMap<String, PlatformAsset>,
}

/// 单个平台的制品声明：资产文件名、sha256（64 位小写 hex）、字节数。
#[derive(Deserialize)]
pub struct PlatformAsset {
    pub asset: String,
    pub sha256: String,
    pub size: u64,
}

/// 一次更新的执行计划（`plan_update` 的产物，供下载与安装阶段使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub version: String,
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    pub is_downgrade: bool,
    pub is_same: bool,
}

/// 原子安装的收据（`install_atomically` 的产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReceipt {
    pub target: PathBuf,
    pub bytes: u64,
}

/// 自更新全链路的错误面（退出码口径见契约 C2：1 校验 / 2 用法 / 3 网络清单 / 4 安装）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Network(String),
    HttpStatus(u16),
    Manifest(String),
    Signature(String),
    NotWritable(String),
    Downgrade(String),
    UnsupportedPlatform(String),
    MissingAsset(String),
    Io(String),
    TooLarge(u64),
}

impl UpdateError {
    /// 面向用户的单行/多行中文说明：必须给出可执行的出路（换 `--pin`、改
    /// `PONY_UPDATE_INSTALL_PATH`、用 sudo、改用桌面安装包等），不得只回显内部错误。
    pub fn user_message(&self) -> String {
        unimplemented!()
    }

    /// 该错误对应的进程退出码（契约 C2：1 校验失败 / 2 用法错误 / 3 网络·HTTP·清单失败 /
    /// 4 安装失败）。调用方据此 `process::exit`，不得另行发明码值。
    pub fn exit_code(&self) -> i32 {
        unimplemented!()
    }
}

/// 解析清单原文。
///
/// 失败条件（均映射为 `UpdateError::Manifest`）：JSON 非法；`schema != "pony-cli-manifest"`；
/// `version` 非可解析语义化版本；任一 `sha256` 非 64 位 hex（大写视为合法并规范化为小写）；
/// `platforms` 为空。
pub fn parse_cli_manifest(raw: &str) -> Result<CliManifest, UpdateError> {
    unimplemented!()
}

/// 按平台标识取清单中的制品条目。
///
/// 失败条件：清单无该平台条目 ⇒ `None`（由调用方转 `UnsupportedPlatform` 并列出已有平台）。
pub fn select_platform_asset<'a>(
    manifest: &'a CliManifest,
    platform: &str,
) -> Option<&'a PlatformAsset> {
    unimplemented!()
}

/// 结合当前版本与 `force` 语义生成更新计划。
///
/// 失败条件：清单无该平台条目 ⇒ `UnsupportedPlatform`（错误串必须列出清单已有平台）；
/// 目标版本低于当前且非 `force` ⇒ `Downgrade`。
///
/// 「已是最新」不走错误面：与当前版本相同且非 `force` ⇒ 返回 `Ok(UpdatePlan { is_same: true,
/// .. })`，由调用方映射为成功态（「当前已是最新版本（v<ver>）」、退出码 0）；`force` 时允许重装。
pub fn plan_update(
    manifest: &CliManifest,
    platform: &str,
    current_version: &str,
    force: bool,
) -> Result<UpdatePlan, UpdateError> {
    unimplemented!()
}

/// 流式校验文件 sha256（64KiB 块读，禁止整文件进内存）。
///
/// 失败条件：文件不存在/不可读 ⇒ `Io`；摘要不符 ⇒ `Signature`（此时调用方**不得**触碰原二进制）。
pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), UpdateError> {
    unimplemented!()
}

/// 原子安装：同目录临时名 `<exe>.new-<pid>` → 赋可执行位（unix `0o755`）→ POSIX rename 覆盖，
/// 目标在此过程中永不缺失；失败路径必须删除临时文件。
///
/// 失败条件：目标目录不可写/rename 失败 ⇒ `NotWritable`。
pub fn install_atomically(
    downloaded: &Path,
    target_exe: &Path,
) -> Result<InstallReceipt, UpdateError> {
    unimplemented!()
}

/// 解析本次更新的目标可执行文件路径。
///
/// 语义：默认 `std::env::current_exe()`；测试/CI 可由 `PONY_UPDATE_INSTALL_PATH` 覆盖。
/// 失败条件：取不到当前可执行文件路径或覆盖值非法 ⇒ `NotWritable` 或 `Io`。
pub fn resolve_install_path() -> Result<PathBuf, UpdateError> {
    unimplemented!()
}

/// 解析语义化三元组版本号（`vX.Y.Z` 或 `X.Y.Z`）。
///
/// 失败条件：段数不是 3、任一段非十进制整数、空串 ⇒ `None`（如 `1.2`、`1.2.3.4`、`abc`）。
pub fn parse_semver_triple(v: &str) -> Option<(u64, u64, u64)> {
    unimplemented!()
}

/// 候选版本是否严格高于当前版本；任一侧不可解析 ⇒ `false`（绝不因解析失败而放行更新）。
pub fn is_newer_semver(candidate: &str, current: &str) -> bool {
    unimplemented!()
}

// ═════════════════════════════════════════════════════════════════════════════
// 验收测试（wave-67，Test Agent 冻结；契约真值源 = 契约矩阵 C1/C5/C6/C7）
//
// 冻结纪律：本 mod 只使用上面已冻结的公开 API，不引用任何未冻结的内部实现
// （发现/下载编排面不在本文件 API 内，其集成测试在 bin `pony` 的 `cli_self_update_tests` 里
//  通过 `run_update_action` + `PONY_UPDATE_*` 逃生舱驱动）。
// 红相事实：此刻实现全是 `unimplemented!()`，A 组用例必须真实失败。
// ═════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};

    // 每个用例独享临时目录（进程 id + 自增序号），Drop 时整目录删除（谁创建谁清理）。
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static SEQ: AtomicUsize = AtomicUsize::new(0);
            let seq = SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "pony-update-test-{}-{seq}-{label}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("必须能创建测试临时目录");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn manifest_json(version: &str, platform: &str, asset: &str, sha256: &str, size: u64) -> String {
        json!({
            "schema": "pony-cli-manifest",
            "version": version,
            "generated_at": "2026-11-07T00:00:00Z",
            "platforms": {
                platform: { "asset": asset, "sha256": sha256, "size": size }
            }
        })
        .to_string()
    }

    fn asset_of<'a>(manifest: &'a CliManifest, platform: &str) -> &'a PlatformAsset {
        manifest
            .platforms
            .get(platform)
            .unwrap_or_else(|| panic!("清单必须含平台 {platform}"))
    }

    // ── C7-1：semver 解析与比较边界 ──────────────────────────────────────────

    /// 合法写法：`X.Y.Z` / `vX.Y.Z` / `VX.Y.Z` 三者同解；非法写法一律 `None`。
    #[test]
    fn parse_semver_triple_accepts_v_prefix_and_rejects_malformed() {
        // Arrange / Act / Assert：三种合法写法。
        assert_eq!(
            parse_semver_triple("1.2.3"),
            Some((1, 2, 3)),
            "裸三元组必须解析成功"
        );
        assert_eq!(
            parse_semver_triple("v1.2.3"),
            Some((1, 2, 3)),
            "小写 v 前缀必须被剥离（历史 tui.rs 语义）"
        );
        assert_eq!(
            parse_semver_triple("V1.2.3"),
            Some((1, 2, 3)),
            "大写 V 前缀必须被剥离"
        );

        // 非法：段数不符、空串、非数字、负数段。
        for bad in ["1.2", "1.2.3.4", "", "a.b.c", "1.2.-3"] {
            assert_eq!(
                parse_semver_triple(bad),
                None,
                "非法版本 {bad:?} 必须解析为 None（不得猜测/补零）"
            );
        }
    }

    /// 严格大于判定：相等为 false、跨段比较正确、任一侧不可解析一律 false（fail-closed）。
    #[test]
    fn is_newer_semver_compares_triples_strictly() {
        assert!(
            is_newer_semver("0.1.184", "0.1.183"),
            "补丁号更高的版本必须判为新"
        );
        assert!(
            !is_newer_semver("0.1.183", "0.1.183"),
            "同版本必须判为不新"
        );
        assert!(
            is_newer_semver("0.2.0", "0.1.999"),
            "次版本更高时即使补丁号更低也必须判为新"
        );
        assert!(
            !is_newer_semver("bad", "0.1.0"),
            "候选版本不可解析时必须 fail-closed（判为不新，绝不放行更新）"
        );
        assert!(
            !is_newer_semver("0.1.0", "not-a-version"),
            "当前版本不可解析时必须 fail-closed"
        );
    }

    // ── C7-1：清单解析（含大写 hex 规范化） ─────────────────────────────────

    /// 合法清单：schema/version/generated_at/platforms 逐字段落地；**大写 hex 规范化为小写**。
    #[test]
    fn parse_cli_manifest_accepts_valid_manifest_and_normalizes_uppercase_hex() {
        // Arrange：64 位 hex 的大写形式（契约 C1：大写视为合法并规范化为小写）。
        let lowercase_sha = sha256_hex(b"pony-cli-binary");
        let uppercase_sha = lowercase_sha.to_uppercase();
        assert_eq!(uppercase_sha.len(), 64, "测试输入必须是 64 位 hex");
        let raw = manifest_json(
            "0.1.184",
            "linux-x86_64",
            "pony-tui_0.1.184_linux-x86_64",
            &uppercase_sha,
            28_000_000,
        );

        // Act
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        // Assert
        assert_eq!(manifest.schema, "pony-cli-manifest");
        assert_eq!(manifest.version, "0.1.184");
        assert_eq!(
            manifest.generated_at.as_deref(),
            Some("2026-11-07T00:00:00Z")
        );
        assert_eq!(manifest.platforms.len(), 1);
        let asset = asset_of(&manifest, "linux-x86_64");
        assert_eq!(asset.asset, "pony-tui_0.1.184_linux-x86_64");
        assert_eq!(
            asset.sha256, lowercase_sha,
            "大写 hex 必须在解析期规范化为小写（下游比对与展示依赖此不变量）"
        );
        assert_eq!(asset.size, 28_000_000);
    }

    /// 负向：schema 错 / sha 长度非 64 / sha 含非 hex / version 非 semver / platforms 为空
    /// 一律 `Manifest` 错误（安全边界 C6-1：sha256 强制，无「跳过校验」开关）。
    #[test]
    fn parse_cli_manifest_rejects_bad_schema_sha_version_and_empty_platforms() {
        let good_sha = sha256_hex(b"payload");
        let base_version = "0.1.184";

        // ① schema 非 "pony-cli-manifest"
        let bad_schema = json!({
            "schema": "pony-desktop-manifest",
            "version": base_version,
            "platforms": { "linux-x86_64": {
                "asset": "a", "sha256": good_sha, "size": 1
            }}
        })
        .to_string();
        assert!(
            matches!(
                parse_cli_manifest(&bad_schema),
                Err(UpdateError::Manifest(_))
            ),
            "schema 不符必须为 Manifest 错误"
        );

        // ② sha256 长度 63（非 64）
        let short_sha = "a".repeat(63);
        assert!(
            matches!(
                parse_cli_manifest(&manifest_json(
                    base_version,
                    "linux-x86_64",
                    "a",
                    &short_sha,
                    1
                )),
                Err(UpdateError::Manifest(_))
            ),
            "sha256 长度非 64 必须为 Manifest 错误"
        );

        // ③ sha256 含非 hex 字符（长度仍是 64）
        let non_hex_sha = format!("g{}", "a".repeat(63));
        assert_eq!(non_hex_sha.len(), 64);
        assert!(
            matches!(
                parse_cli_manifest(&manifest_json(
                    base_version,
                    "linux-x86_64",
                    "a",
                    &non_hex_sha,
                    1
                )),
                Err(UpdateError::Manifest(_))
            ),
            "sha256 含非 hex 字符必须为 Manifest 错误"
        );

        // ④ version 非 semver
        assert!(
            matches!(
                parse_cli_manifest(&manifest_json(
                    "0.1",
                    "linux-x86_64",
                    "a",
                    &good_sha,
                    1
                )),
                Err(UpdateError::Manifest(_))
            ),
            "version 非三段 semver 必须为 Manifest 错误"
        );

        // ⑤ platforms 为空
        let empty_platforms = json!({
            "schema": "pony-cli-manifest",
            "version": base_version,
            "platforms": {}
        })
        .to_string();
        assert!(
            matches!(
                parse_cli_manifest(&empty_platforms),
                Err(UpdateError::Manifest(_))
            ),
            "空 platforms 必须为 Manifest 错误（否则后续必然退化为平台缺失）"
        );
    }

    // ── C7-1：资产选择与更新计划 ────────────────────────────────────────────

    /// 命中返回条目本身（asset/sha256/size 一致）；未命中返回 None（不猜测近似平台）。
    #[test]
    fn select_platform_asset_hits_and_misses() {
        let sha = sha256_hex(b"bin");
        let raw = manifest_json("0.1.184", "linux-x86_64", "pony-tui-linux", &sha, 123);
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        let hit = select_platform_asset(&manifest, "linux-x86_64")
            .expect("已有平台必须命中");
        assert_eq!(hit.asset, "pony-tui-linux");
        assert_eq!(hit.sha256, sha);
        assert_eq!(hit.size, 123);

        assert!(
            select_platform_asset(&manifest, "macos-aarch64").is_none(),
            "清单里没有的平台必须返回 None（由调用方转 UnsupportedPlatform）"
        );
    }

    /// 有更新 ⇒ Ok 且 is_downgrade=false、is_same=false，计划字段与清单一致。
    #[test]
    fn plan_update_plans_upgrade() {
        let sha = sha256_hex(b"bin");
        let raw = manifest_json("0.1.184", "linux-x86_64", "pony-tui-linux", &sha, 456);
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        let plan = plan_update(&manifest, "linux-x86_64", "0.1.183", false)
            .expect("更高版本必须给出计划");

        assert_eq!(plan.version, "0.1.184");
        assert_eq!(plan.asset, "pony-tui-linux");
        assert_eq!(plan.sha256, sha);
        assert_eq!(plan.size, 456);
        assert!(!plan.is_downgrade, "升级计划不得标记为降级");
        assert!(!plan.is_same, "版本不同时不得标记为「已最新」");
    }

    /// 版本相同且非 force ⇒ **`Ok` 且 `is_same == true`**（契约 C1：不走错误面，调用方映射为
    /// 「当前已是最新版本」成功态、退出码 0）；force 时允许重装。
    #[test]
    fn plan_update_marks_same_version_and_allows_reinstall_only_with_force() {
        let sha = sha256_hex(b"bin");
        let raw = manifest_json("0.1.183", "linux-x86_64", "pony-tui-linux", &sha, 789);
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        let same = plan_update(&manifest, "linux-x86_64", "0.1.183", false)
            .expect("同版本非 force 不得走错误面（契约 C1：返回 is_same 计划）");
        assert!(same.is_same, "同版本必须标记 is_same=true");
        assert!(!same.is_downgrade, "同版本不得标记为降级");
        assert_eq!(same.version, "0.1.183");

        let forced = plan_update(&manifest, "linux-x86_64", "0.1.183", true)
            .expect("force 必须允许同版本重装");
        assert_eq!(forced.version, "0.1.183");
        assert!(!forced.is_downgrade, "同版本重装不得标记为降级");
    }

    /// 降级：非 force ⇒ `Err(Downgrade)`（错误串含「降级」或版本号）；force ⇒ Ok 且 is_downgrade=true。
    #[test]
    fn plan_update_rejects_downgrade_without_force_but_allows_with_force() {
        let sha = sha256_hex(b"bin");
        let raw = manifest_json("0.1.150", "linux-x86_64", "pony-tui-linux", &sha, 11);
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        // 非 force：必须拒绝，且错误串对用户有信息量。
        let error = plan_update(&manifest, "linux-x86_64", "0.1.183", false)
            .expect_err("降级且非 force 必须拒绝");
        let message = match error {
            UpdateError::Downgrade(message) => message,
            other => panic!("降级必须映射为 UpdateError::Downgrade，实际={other:?}"),
        };
        assert!(
            message.contains("降级") || message.contains("0.1.150") || message.contains("0.1.183"),
            "降级错误串必须说明是降级并给出版本号，实际={message}"
        );

        // force：允许，且必须显式标记 is_downgrade。
        let plan = plan_update(&manifest, "linux-x86_64", "0.1.183", true)
            .expect("force 必须允许降级");
        assert!(plan.is_downgrade, "force 降级必须标记 is_downgrade=true");
        assert!(!plan.is_same, "降级不得标记为「已最新」");
        assert_eq!(plan.version, "0.1.150");
    }

    /// 平台缺失 ⇒ `Err(UnsupportedPlatform)`，且**错误串必须列出清单已有平台**（可执行的出路）。
    #[test]
    fn plan_update_unsupported_platform_lists_known_platforms() {
        // Arrange：清单只有 macos-aarch64 + windows-x86_64 两个平台。
        let sha = sha256_hex(b"bin");
        let raw = json!({
            "schema": "pony-cli-manifest",
            "version": "0.1.184",
            "platforms": {
                "macos-aarch64": { "asset": "pony-macos", "sha256": sha, "size": 1 },
                "windows-x86_64": { "asset": "pony-win.exe", "sha256": sha, "size": 2 }
            }
        })
        .to_string();
        let manifest = parse_cli_manifest(&raw).expect("合法清单必须解析成功");

        // Act
        let error = plan_update(&manifest, "linux-x86_64", "0.1.183", false)
            .expect_err("清单没有当前平台必须报 UnsupportedPlatform");

        // Assert
        let message = match error {
            UpdateError::UnsupportedPlatform(message) => message,
            other => panic!("平台缺失必须映射为 UpdateError::UnsupportedPlatform，实际={other:?}"),
        };
        assert!(
            message.contains("macos-aarch64"),
            "错误串必须列出清单已有平台（macos-aarch64），实际={message}"
        );
        assert!(
            message.contains("windows-x86_64"),
            "错误串必须列出清单已有平台（windows-x86_64），实际={message}"
        );
    }

    // ── C7-1：流式 sha256 校验 ─────────────────────────────────────────────

    /// sha256 正确 ⇒ Ok；改 1 字节 ⇒ Err(Signature)；文件不存在 ⇒ Err(Io)。
    #[test]
    fn verify_sha256_accepts_match_and_rejects_tampered_or_missing_file() {
        // Arrange：1 KiB 内容 + 正确摘要。
        let dir = TempDir::new("verify");
        let path = dir.join("artifact.bin");
        let content: Vec<u8> = (0..1024u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &content).expect("必须能写入测试制品");
        let expected = sha256_hex(&content);

        // ① 正确摘要 ⇒ Ok
        verify_sha256(&path, &expected).expect("摘要一致必须通过校验");

        // ② 改 1 字节 ⇒ Signature
        let mut tampered = content.clone();
        tampered[0] ^= 0xff;
        std::fs::write(&path, &tampered).expect("必须能改写测试制品");
        let error = verify_sha256(&path, &expected).expect_err("内容被改动后必须校验失败");
        assert!(
            matches!(error, UpdateError::Signature(_)),
            "摘要不符必须映射为 Signature，实际={error:?}"
        );

        // ③ 文件不存在 ⇒ Io
        let missing = dir.join("no-such-artifact.bin");
        let error = verify_sha256(&missing, &expected).expect_err("缺失文件必须报错");
        assert!(
            matches!(error, UpdateError::Io(_)),
            "文件不存在必须映射为 Io，实际={error:?}"
        );
    }

    /// 不可逆面保护（C6-1/C6-5）：摘要不符时**绝不触碰**既有二进制，且不残留任何
    /// `.new*` / `.part` 临时文件。
    #[test]
    fn sha_mismatch_leaves_existing_binary_untouched_without_residue() {
        // Arrange：既有二进制 + 一份摘要不符的下载产物。
        let dir = TempDir::new("sha-mismatch");
        let target = dir.join("pony");
        std::fs::write(&target, b"OLD-INSTALLED-BINARY").expect("必须能写入哨兵二进制");
        let downloaded = dir.join("pony.part-download");
        std::fs::write(&downloaded, b"NEW-BINARY").expect("必须能写入下载产物");
        let expected_for_other_payload = sha256_hex(b"something-else");

        // Act：校验失败（调用方在 install 之前必须停下）。
        let error = verify_sha256(&downloaded, &expected_for_other_payload)
            .expect_err("摘要不符必须阻止安装");

        // Assert
        assert!(matches!(error, UpdateError::Signature(_)));
        assert_eq!(
            std::fs::read(&target).expect("目标必须仍可读"),
            b"OLD-INSTALLED-BINARY".to_vec(),
            "校验失败不得触碰既有二进制（逐字节不变）"
        );
        let residues: Vec<String> = std::fs::read_dir(&dir.path)
            .expect("必须能列目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".new") || name.contains(".part") || name.ends_with(".update.lock"))
            .collect();
        assert!(
            residues.is_empty(),
            "校验失败路径不得残留临时/锁文件，实际残留={residues:?}"
        );
    }

    // ── C1：平台标识映射 ────────────────────────────────────────────────────

    /// 平台标识必须与发布侧一致（5 项白名单）；未知 OS/ARCH 一律 `None`（不猜测近似平台）。
    #[test]
    fn current_platform_matches_os_arch_whitelist() {
        let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some("linux-x86_64"),
            ("linux", "aarch64") => Some("linux-aarch64"),
            ("macos", "x86_64") => Some("macos-x86_64"),
            ("macos", "aarch64") => Some("macos-aarch64"),
            ("windows", "x86_64") => Some("windows-x86_64"),
            _ => None,
        };
        assert_eq!(
            current_platform(),
            expected,
            "平台标识必须与本机 os/arch 的发布侧白名单严格一致（未知组合返回 None）"
        );
    }

    // ── C1/C7-1：原子安装（unix） ───────────────────────────────────────────

    /// 正常安装：目标**逐字节等于**下载产物、具备可执行位、收据字段正确、无 `.new-*` 残留。
    #[cfg(unix)]
    #[test]
    fn install_atomically_replaces_target_with_identical_executable_bytes() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let dir = TempDir::new("install-ok");
        let downloaded = dir.join("downloaded.bin");
        let payload = b"#!/bin/sh\nexit 0\n".to_vec();
        std::fs::write(&downloaded, &payload).expect("必须能写入下载产物");
        let target = dir.join("pony");
        std::fs::write(&target, b"OLD-BINARY").expect("必须能写入旧二进制");

        // Act
        let receipt = install_atomically(&downloaded, &target).expect("安装必须成功");

        // Assert
        assert_eq!(receipt.target, target, "收据必须回报真实安装目标");
        assert_eq!(
            receipt.bytes,
            payload.len() as u64,
            "收据字节数必须等于制品长度"
        );
        assert_eq!(
            std::fs::read(&target).expect("目标必须可读"),
            payload,
            "安装后目标必须逐字节等于下载产物"
        );
        let mode = std::fs::metadata(&target)
            .expect("目标元数据必须可读")
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "安装后必须带可执行位，实际 mode={mode:o}"
        );
        let residues: Vec<String> = std::fs::read_dir(&dir.path)
            .expect("必须能列目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".new") || name.contains(".part"))
            .collect();
        assert!(
            residues.is_empty(),
            "成功路径不得残留临时文件，实际残留={residues:?}"
        );
    }

    /// 目标目录只读 ⇒ `NotWritable`，且目录内**不产生**任何新文件。
    #[cfg(unix)]
    #[test]
    fn install_atomically_reports_not_writable_for_read_only_directory() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange：制品放在可写目录，目标落在只读目录。
        let dir = TempDir::new("install-readonly");
        let downloaded = dir.join("downloaded.bin");
        std::fs::write(&downloaded, b"NEW-BINARY").expect("必须能写入下载产物");
        let readonly_dir = dir.join("readonly");
        std::fs::create_dir_all(&readonly_dir).expect("必须能创建只读目录");
        let target = readonly_dir.join("pony");
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o555))
            .expect("必须能把目录设为只读");

        // 收尾保证：无论断言是否失败，Drop 前都必须恢复 0o755（否则污染 temp 清理）。
        struct RestorePerms(PathBuf);
        impl Drop for RestorePerms {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(
                    &self.0,
                    std::fs::Permissions::from_mode(0o755),
                );
            }
        }
        let _restore = RestorePerms(readonly_dir.clone());

        // Act
        let error = install_atomically(&downloaded, &target)
            .expect_err("只读目标目录必须安装失败");

        // Assert
        assert!(
            matches!(error, UpdateError::NotWritable(_)),
            "只读目录必须映射为 NotWritable，实际={error:?}"
        );
        let entries: Vec<String> = std::fs::read_dir(&readonly_dir)
            .expect("必须能列只读目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            entries.is_empty(),
            "只读目录内不得产生任何新文件（含临时文件），实际={entries:?}"
        );
    }
}
