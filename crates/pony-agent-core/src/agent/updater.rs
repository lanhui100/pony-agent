//! `pony update` CLI 自更新协议面（wave-67）。
//!
//! 契约真值源：`.dev-team/contract-matrix-cli-self-update.md`（C1 = API、C2 = 命令面、
//! C4 = 发现与重定向、C5 = NFR、C6 = 安全）。
//!
//! 分工：本文件既是被冻结的**协议面**（常量 / 数据结构 / 纯函数，见 C1），也承载
//! 真正的编排实现（发现 → 清单 → 下载 → 校验 → 原子安装 → 回验），由 bin 侧
//! `run_update_action` 单点调用。理由：bin 目标只能触达 lib 的 `pub` 面，若把发现/下载
//! 留在 lib 私有层会变成死代码 —— 故对外暴露 `run_self_update` 这一个编排入口。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

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

// ── 环境变量名（契约 C1 表；全部「非法值回退默认」，禁止静默取 0/空） ──────────
const API_BASE_ENV: &str = "PONY_UPDATE_API_BASE";
const REPO_ENV: &str = "PONY_UPDATE_REPO";
const MANIFEST_URL_ENV: &str = "PONY_UPDATE_MANIFEST_URL";
const DOWNLOAD_BUDGET_ENV: &str = "PONY_UPDATE_DOWNLOAD_BUDGET_SECS";
const INSTALL_PATH_ENV: &str = "PONY_UPDATE_INSTALL_PATH";
const ALLOW_INSECURE_ENV: &str = "PONY_UPDATE_ALLOW_INSECURE";

// ── 网络与重定向常量（契约 C4 / C5） ───────────────────────────────────────
/// 控制面（列表 / 清单）连接超时：沿用 `/update` 既有 5s 口径。
const CONTROL_PLANE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 控制面空闲读超时：10s。判据是「任意 10s 内没有字节到达」，而不是整段 body 的累计耗时
/// —— 反代/代理链路上 175KB 的 releases 列表实测要 1.7~3.4s，只看累计耗时必然误杀。
const CONTROL_PLANE_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// 控制面整体预算：30s（连接 + ≤5 跳重定向 + body 全量）。旧值 5s 会把「慢」误判成「错」：
/// body 读到一半被总超时打断后，reqwest 对 body 流错误统一走 `error::decode`，只报
/// `error decoding response body`（与网络/代理毫无字面关联），用户拿到的是不可诊断的结论。
const CONTROL_PLANE_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// 制品下载连接超时：10s（总预算由 `tokio::time::timeout` 包裹整段，见 C5 例外说明）。
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 手动跟随即的最多重定向跳数（契约 C4：>5 跳即失败）。
const MAX_REDIRECT_HOPS: usize = 5;
/// 列表 / 清单请求失败后的唯一一次重试退避。
const RETRY_BACKOFF: Duration = Duration::from_millis(500);
/// 生产受信 host（scheme 必须 https）。
const PRODUCTION_ALLOWED_HOSTS: [&str; 4] = [
    "api.github.com",
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
/// 逃生舱允许的本地 host（仅当 `PONY_UPDATE_MANIFEST_URL` + `PONY_UPDATE_ALLOW_INSECURE=1`）。
const LOOPBACK_ALLOWED_HOSTS: [&str; 4] = ["127.0.0.1", "localhost", "::1", "[::1]"];

/// 当前平台的发布标识。
///
/// 取值集合与发布侧一致：`linux-x86_64` / `linux-aarch64` / `macos-x86_64` /
/// `macos-aarch64` / `windows-x86_64`。
///
/// 失败条件：目标三元组不在上述集合内（不支持的 OS/架构）⇒ 返回 `None`，调用方须映射为
/// `UpdateError::UnsupportedPlatform`，不得猜测近似平台。
pub fn current_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        _ => None,
    }
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

/// `pony update` 的旗标（由 CLI 解析层填充；`check` 只检查不安装）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateOptions {
    pub check: bool,
    pub force: bool,
    pub pin: Option<String>,
}

/// 自更新成功收据（`run_self_update` 的产物，供 CLI 渲染人可读 / JSON 两种形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOutcome {
    pub previous_version: String,
    pub latest_version: String,
    /// `false` = 未安装（`--check`、或当前已是最新）；`true` = 已下载并安装成功。
    pub updated: bool,
    /// 仅当「版本相同但本机字节 ≠ 清单声明」时为 `true`：这是**本地构建/补丁版或被替换过的
    /// 二进制**，既不能算「已是最新」（旧实现会在此谎报 up-to-date，导致补丁版永远无法被
    /// `pony update` 发现），也不允许自动覆盖（不自作主张毁掉用户本机定制件）。出路是 `--force`。
    pub content_mismatch: bool,
    pub install_path: PathBuf,
    pub sha256: String,
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
    /// 稳定的机器可读错误码（`--json` 收据的 `error.code`；不随文案变化）。
    pub fn code(&self) -> &'static str {
        match self {
            UpdateError::Network(_) => "network",
            UpdateError::HttpStatus(_) => "http_status",
            UpdateError::Manifest(_) => "manifest",
            UpdateError::Signature(_) => "signature",
            UpdateError::NotWritable(_) => "not_writable",
            UpdateError::Downgrade(_) => "downgrade",
            UpdateError::UnsupportedPlatform(_) => "unsupported_platform",
            UpdateError::MissingAsset(_) => "missing_asset",
            UpdateError::Io(_) => "io",
            UpdateError::TooLarge(_) => "too_large",
        }
    }

    /// 面向用户的单行/多行中文说明：必须给出可执行的出路（换 `--pin`、改
    /// `PONY_UPDATE_INSTALL_PATH`、用 sudo、改用桌面安装包等），不得只回显内部错误。
    pub fn user_message(&self) -> String {
        match self {
            UpdateError::Network(detail) => format!(
                "网络请求失败：{detail}\n请检查网络 / 代理后重试；若在企业网络内，可先配置 HTTPS_PROXY。"
            ),
            UpdateError::HttpStatus(status) => format!(
                "更新源返回 HTTP {status}（发布资产或清单缺失）。\n请稍后重试；若持续 404，可用 `pony update --pin <CLI 版本> --force` 指定已发布的 CLI 版本（注意：--pin 指 CLI 版本，不是 release tag；指定更旧版本需同时给 --force，否则会被降级闸拦下）。"
            ),
            UpdateError::Manifest(detail) => format!(
                "更新清单不可用：{detail}\n请稍后重试；若清单持续损坏，请改用桌面端安装包，或从 Releases 页面手动下载。"
            ),
            UpdateError::Signature(detail) => format!(
                "校验失败，已放弃安装（原有二进制未改动）：{detail}\n请重试；若仍失败，可用 `pony update --pin <CLI 版本> --force` 指定其它已发布版本（指定更旧版本必须同时给 --force）。"
            ),
            UpdateError::NotWritable(detail) => format!(
                "安装失败（目标不可写）：{detail}\n可尝试：① 用 `sudo pony update` 重跑；② 设置 PONY_UPDATE_INSTALL_PATH 指向可写目录；③ 改用桌面端安装包。"
            ),
            UpdateError::Downgrade(detail) => format!(
                "拒绝降级：{detail}\n如确认要降级，请加 `--force`；若要指定更旧的 CLI 版本，必须写成 `pony update --pin <版本> --force`（只给 --pin 仍会被降级闸拦下）。"
            ),
            UpdateError::UnsupportedPlatform(detail) => format!(
                "当前平台不支持 CLI 自更新：{detail}\nWindows 请改用桌面端安装包（Pony.Agent_*_setup.exe）。"
            ),
            UpdateError::MissingAsset(detail) => format!(
                "未找到可用的发布资产：{detail}\n`--pin` 指 CLI 版本（core 版本线），不是 release tag；可先不带 `--pin` 重试。"
            ),
            UpdateError::Io(detail) => format!(
                "磁盘操作失败：{detail}\n请检查目录权限与磁盘空间；必要时设置 PONY_UPDATE_INSTALL_PATH 指向可写目录。"
            ),
            UpdateError::TooLarge(bytes) => format!(
                "制品体积 {bytes} 字节超过上限 {MAX_ARTIFACT_BYTES} 字节，已中止下载且未落盘。\n请确认更新源可信；如为误报请反馈。"
            ),
        }
    }

    /// 该错误对应的进程退出码（契约 C2：1 校验失败 / 2 用法错误 / 3 网络·HTTP·清单失败 /
    /// 4 安装失败）。调用方据此 `process::exit`，不得另行发明码值。
    pub fn exit_code(&self) -> i32 {
        match self {
            UpdateError::Signature(_) | UpdateError::TooLarge(_) => 1,
            UpdateError::Downgrade(_) => 2,
            UpdateError::Network(_)
            | UpdateError::HttpStatus(_)
            | UpdateError::Manifest(_)
            | UpdateError::MissingAsset(_)
            | UpdateError::UnsupportedPlatform(_) => 3,
            UpdateError::NotWritable(_) | UpdateError::Io(_) => 4,
        }
    }
}

/// 解析清单原文。
///
/// 失败条件（均映射为 `UpdateError::Manifest`）：JSON 非法；`schema != "pony-cli-manifest"`；
/// `version` 非可解析语义化版本；任一 `sha256` 非 64 位 hex（大写视为合法并规范化为小写）；
/// `platforms` 为空。
pub fn parse_cli_manifest(raw: &str) -> Result<CliManifest, UpdateError> {
    let mut manifest: CliManifest = serde_json::from_str(raw)
        .map_err(|error| UpdateError::Manifest(format!("清单 JSON 解析失败：{error}")))?;
    if manifest.schema != "pony-cli-manifest" {
        return Err(UpdateError::Manifest(format!(
            "清单 schema 应为 `pony-cli-manifest`，实际为 `{}`",
            manifest.schema
        )));
    }
    if parse_semver_triple(&manifest.version).is_none() {
        return Err(UpdateError::Manifest(format!(
            "清单 version 不是三段语义化版本：`{}`",
            manifest.version
        )));
    }
    if manifest.platforms.is_empty() {
        return Err(UpdateError::Manifest(
            "清单 platforms 为空，无法定位任何制品".to_string(),
        ));
    }
    for (platform, asset) in manifest.platforms.iter_mut() {
        let normalized = asset.sha256.trim().to_ascii_lowercase();
        if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(UpdateError::Manifest(format!(
                "平台 `{platform}` 的 sha256 不是 64 位十六进制：`{}`",
                asset.sha256
            )));
        }
        if asset.asset.trim().is_empty() {
            return Err(UpdateError::Manifest(format!(
                "平台 `{platform}` 的 asset 文件名为空"
            )));
        }
        asset.sha256 = normalized;
    }
    Ok(manifest)
}

/// 按平台标识取清单中的制品条目。
///
/// 失败条件：清单无该平台条目 ⇒ `None`（由调用方转 `UnsupportedPlatform` 并列出已有平台）。
pub fn select_platform_asset<'a>(
    manifest: &'a CliManifest,
    platform: &str,
) -> Option<&'a PlatformAsset> {
    manifest.platforms.get(platform)
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
    let asset = select_platform_asset(manifest, platform).ok_or_else(|| {
        let known: Vec<&str> = manifest
            .platforms
            .keys()
            .map(String::as_str)
            .collect();
        UpdateError::UnsupportedPlatform(format!(
            "清单没有当前平台 `{platform}` 的制品；清单已有平台：{}",
            if known.is_empty() {
                "（无）".to_string()
            } else {
                known.join(", ")
            }
        ))
    })?;
    let candidate = parse_semver_triple(&manifest.version);
    let current = parse_semver_triple(current_version);
    let is_same = matches!((candidate, current), (Some(candidate), Some(current)) if candidate == current);
    let is_downgrade =
        matches!((candidate, current), (Some(candidate), Some(current)) if candidate < current);
    if is_downgrade && !force {
        return Err(UpdateError::Downgrade(format!(
            "清单版本 v{} 低于当前版本 v{}",
            manifest.version, current_version
        )));
    }
    Ok(UpdatePlan {
        version: manifest.version.clone(),
        asset: asset.asset.clone(),
        sha256: asset.sha256.clone(),
        size: asset.size,
        is_downgrade,
        is_same,
    })
}

/// 流式计算文件 sha256（64KiB 块读，禁止整文件进内存；**只读，不改动任何文件**）。
///
/// 失败条件：文件不存在/不可读 ⇒ `Io`。
pub fn hash_file_sha256(path: &Path) -> Result<String, UpdateError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        UpdateError::Io(format!("无法打开待校验文件 {}：{error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            UpdateError::Io(format!("读取 {} 失败：{error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 判定「本机目标文件内容 == 清单声明的制品」：只读哈希，**绝不改动目标**。
///
/// 口径（调用方必须按此处理）：**只有**读到文件且摘要逐字相等才算一致；文件缺失 / 不可读 /
/// 摘要不等一律视为「不一致」——自更新里「无法证明一致」绝不能当成「已是最新」。
pub fn installed_matches_artifact(install_path: &Path, expected_hex: &str) -> bool {
    match hash_file_sha256(install_path) {
        Ok(actual) => actual == expected_hex.trim().to_ascii_lowercase(),
        Err(_) => false,
    }
}

/// 流式校验文件 sha256（64KiB 块读，禁止整文件进内存）。
///
/// 失败条件：文件不存在/不可读 ⇒ `Io`；摘要不符 ⇒ `Signature`（此时调用方**不得**触碰原二进制）。
pub fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), UpdateError> {
    let actual = hash_file_sha256(path)?;
    let expected = expected_hex.trim().to_ascii_lowercase();
    if actual != expected {
        // 冻结契约：摘要不符的下载产物必须当场丢弃（防空壳「校验失败却把坏制品留在盘上」
        // 被后续 install 误用）；只会删除**待安装产物**，绝不触碰既有二进制。
        let _ = std::fs::remove_file(path);
        return Err(UpdateError::Signature(format!(
            "sha256 不符（期望 {expected}，实际 {actual}）"
        )));
    }
    Ok(())
}

/// 原子安装：同目录临时名 `<exe>.new-<pid>` → 赋可执行位（unix `0o755`）→ POSIX rename 覆盖，
/// 目标在此过程中永不缺失；失败路径必须删除临时文件。
///
/// 失败条件：目标目录不可写/rename 失败 ⇒ `NotWritable`。
pub fn install_atomically(
    downloaded: &Path,
    target_exe: &Path,
) -> Result<InstallReceipt, UpdateError> {
    #[cfg(windows)]
    {
        let _ = (downloaded, target_exe);
        return Err(UpdateError::UnsupportedPlatform(
            "Windows 运行中的 exe 无法被替换".to_string(),
        ));
    }
    #[cfg(not(windows))]
    {
        let bytes = std::fs::metadata(downloaded)
            .map_err(|error| {
                UpdateError::Io(format!("无法读取下载产物 {}：{error}", downloaded.display()))
            })?
            .len();
        let temp_path = sibling_path(target_exe, "new");
        let outcome = (|| -> Result<(), UpdateError> {
            std::fs::copy(downloaded, &temp_path)
                .map_err(|error| write_error(&temp_path, error))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| write_error(&temp_path, error))?;
            }
            std::fs::rename(&temp_path, target_exe).map_err(|error| write_error(target_exe, error))
        })();
        if let Err(error) = outcome {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
        Ok(InstallReceipt {
            target: target_exe.to_path_buf(),
            bytes,
        })
    }
}

/// 解析本次更新的目标可执行文件路径。
///
/// 语义：默认 `std::env::current_exe()`；测试/CI 可由 `PONY_UPDATE_INSTALL_PATH` 覆盖。
/// 失败条件：取不到当前可执行文件路径或覆盖值非法 ⇒ `NotWritable` 或 `Io`。
pub fn resolve_install_path() -> Result<PathBuf, UpdateError> {
    if let Some(raw) = std::env::var(INSTALL_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(raw));
    }
    std::env::current_exe().map_err(|error| {
        UpdateError::Io(format!("无法解析当前可执行文件路径：{error}"))
    })
}

/// 解析语义化三元组版本号（`vX.Y.Z` 或 `X.Y.Z`）。
///
/// 失败条件：段数不是 3、任一段非十进制整数、空串 ⇒ `None`（如 `1.2`、`1.2.3.4`、`abc`）。
pub fn parse_semver_triple(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v.trim().trim_start_matches(|c| c == 'v' || c == 'V');
    let mut parts = clean.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// 候选版本是否严格高于当前版本；任一侧不可解析 ⇒ `false`（绝不因解析失败而放行更新）。
pub fn is_newer_semver(candidate: &str, current: &str) -> bool {
    match (parse_semver_triple(candidate), parse_semver_triple(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// `pony update` 自更新编排入口（发现 → 清单 → 下载 → 校验 → 原子安装 → 回验）。
///
/// 同步签名：内部自建 current-thread tokio 运行时（CLI 进程无共享 runtime，`main` 与
/// 测试都在同步上下文里调用），避免把 async 传染到 bin 的派发链。
pub fn run_self_update(options: &UpdateOptions) -> Result<UpdateOutcome, UpdateError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| UpdateError::Network(format!("异步运行时启动失败：{error}")))?;
    runtime.block_on(run_self_update_async(options))
}

async fn run_self_update_async(options: &UpdateOptions) -> Result<UpdateOutcome, UpdateError> {
    if cfg!(windows) {
        return Err(UpdateError::UnsupportedPlatform(
            "Windows 运行中的 exe 无法被替换（自更新仅在 Linux/macOS 支持）".to_string(),
        ));
    }
    let config = UpdateConfig::from_env();
    let install_path = resolve_install_path()?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let platform = current_platform().ok_or_else(|| {
        UpdateError::UnsupportedPlatform(format!(
            "未知平台 {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;

    let control_client = build_control_plane_client(&config)?;
    let resolved = match resolve_manifest(&control_client, &config, options.pin.as_deref()).await {
        Ok(resolved) => resolved,
        Err(error) => {
            // 失败路径同样打一行（observability 声明：status=fail 对所有阶段生效）。
            phase_log("discover", "fail", &format!("code={}", error.code()));
            return Err(error);
        }
    };
    phase_log(
        "discover",
        "ok",
        &format!("manifest={}", resolved.manifest_url),
    );
    let plan = match plan_update(&resolved.manifest, platform, &current_version, options.force) {
        Ok(plan) => plan,
        Err(error) => {
            phase_log("manifest", "fail", &format!("code={}", error.code()));
            return Err(error);
        }
    };
    phase_log(
        "manifest",
        "ok",
        &format!(
            "version={} asset={} size={}",
            plan.version, plan.asset, plan.size
        ),
    );

    let asset_url = asset_download_url(&resolved, &config, &plan.asset)?;

    if plan.is_same && !options.force {
        // 「版本相同」只说明 semver 同号，不说明本机跑的就是官方发布件：本地 `cargo build` 装的
        // 补丁版、被手工替换过的二进制都同号。旧实现直接回「已是最新」= 谎报，且把自愈通路掐断
        // （实测：装了我们补丁版的 0.1.158 永远拿不到官方 0.1.158，也无从发现差异）。故同号必须
        // 再用**内容哈希**判定；不一致时**不自动覆盖**（不自作主张毁掉本机定制件），只如实报告，
        // 出路是 `--force`。
        let content_mismatch = !installed_matches_artifact(&install_path, &plan.sha256);
        return Ok(UpdateOutcome {
            previous_version: current_version,
            latest_version: plan.version,
            updated: false,
            content_mismatch,
            install_path,
            sha256: plan.sha256,
        });
    }
    if options.check {
        return Ok(UpdateOutcome {
            previous_version: current_version,
            latest_version: plan.version,
            updated: false,
            content_mismatch: false,
            install_path,
            sha256: plan.sha256,
        });
    }
    if plan.size > MAX_ARTIFACT_BYTES {
        // 只按清单声明拒绝：不建锁、不写盘（「超限即不落盘」是硬不变量）。
        return Err(UpdateError::TooLarge(plan.size));
    }

    // 锁：同目录 `<exe>.update.lock`（create_new ⇒ 原子互斥）；Drop 只删自己创建的锁。
    let _lock = UpdateLock::acquire(&install_path)?;
    let part_path = part_path(&install_path);
    let download_client = build_download_client(&config)?;
    if let Err(error) =
        download_artifact(&download_client, &asset_url, &part_path, &plan, &config).await
    {
        phase_log("download", "fail", &format!("code={}", error.code()));
        return Err(error);
    }
    phase_log(
        "download",
        "ok",
        &format!("bytes={} url={asset_url}", plan.size),
    );

    if let Err(error) = verify_sha256(&part_path, &plan.sha256) {
        let _ = std::fs::remove_file(&part_path);
        phase_log("verify", "fail", &format!("code={}", error.code()));
        return Err(error);
    }
    phase_log("verify", "ok", &format!("sha256={}", plan.sha256));

    // 回滚面：安装前保留旧二进制副本，回验失败即用它恢复（正常路径安装后删除）。
    let backup_path = sibling_path(&install_path, "old");
    let had_existing = install_path.exists();
    if had_existing {
        if let Err(error) = std::fs::copy(&install_path, &backup_path) {
            let _ = std::fs::remove_file(&part_path);
            let error = write_error(&backup_path, error);
            phase_log("install", "fail", &format!("code={}", error.code()));
            return Err(error);
        }
    }
    let receipt = match install_atomically(&part_path, &install_path) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = std::fs::remove_file(&part_path);
            if had_existing {
                let _ = std::fs::remove_file(&backup_path);
            }
            phase_log("install", "fail", &format!("code={}", error.code()));
            return Err(error);
        }
    };
    let _ = std::fs::remove_file(&part_path);
    phase_log("install", "ok", &format!("path={}", receipt.target.display()));

    match verify_installed(&receipt.target, &plan.version) {
        Ok(()) => {
            if had_existing {
                let _ = std::fs::remove_file(&backup_path);
            }
            phase_log(
                "verify_installed",
                "ok",
                &format!("version={}", plan.version),
            );
            Ok(UpdateOutcome {
                previous_version: current_version,
                latest_version: plan.version,
                updated: true,
                content_mismatch: false,
                install_path,
                sha256: plan.sha256,
            })
        }
        Err(error) => {
            if had_existing {
                let _ = std::fs::rename(&backup_path, &install_path);
            }
            phase_log(
                "verify_installed",
                "fail",
                &format!("code={}", error.code()),
            );
            Err(error)
        }
    }
}

/// 单行阶段日志（stderr；`--json` 模式下 stdout 只留收据，绝不污染）。
fn phase_log(phase: &str, status: &str, detail: &str) {
    eprintln!("[pony-update] phase={phase} status={status} {detail}");
}

/// 进程内更新配置（由环境变量解析；非法值一律回退默认，见契约 C1 表）。
#[derive(Debug, Clone)]
struct UpdateConfig {
    api_base: String,
    repo: String,
    manifest_url: Option<String>,
    download_budget: Duration,
    allow_insecure: bool,
    user_agent: String,
}

impl UpdateConfig {
    fn from_env() -> Self {
        let user_agent = format!("pony-update/{}", env!("CARGO_PKG_VERSION"));
        let manifest_url_raw = std::env::var(MANIFEST_URL_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let allow_insecure_requested = std::env::var(ALLOW_INSECURE_ENV)
            .map(|value| value.trim() == "1")
            .unwrap_or(false);
        // 逃生舱需**同时**满足：设置 MANIFEST_URL 且 ALLOW_INSECURE=1（fail-closed，见 C4）。
        let allow_insecure = allow_insecure_requested && manifest_url_raw.is_some();
        let manifest_url = manifest_url_raw.filter(|url| {
            url.starts_with("https://") || (allow_insecure && is_local_http_url(url))
        });
        let allow_insecure = allow_insecure && manifest_url.is_some();
        let api_base = std::env::var(API_BASE_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| {
                value.starts_with("https://") || (allow_insecure && value.starts_with("http://"))
            })
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| DEFAULT_UPDATE_API_BASE.to_string());
        let repo = std::env::var(REPO_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| is_valid_repo_slug(value))
            .unwrap_or_else(|| DEFAULT_UPDATE_REPO.to_string());
        let download_budget = std::env::var(DOWNLOAD_BUDGET_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| (1..=MAX_ALLOWED_DOWNLOAD_BUDGET_SECS).contains(seconds))
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(DEFAULT_DOWNLOAD_BUDGET_SECS));
        Self {
            api_base,
            repo,
            manifest_url,
            download_budget,
            allow_insecure,
            user_agent,
        }
    }

    fn escape_hatch(&self) -> bool {
        self.allow_insecure && self.manifest_url.is_some()
    }

    fn manifest_host(&self) -> Option<String> {
        self.manifest_url
            .as_deref()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_string))
    }
}

fn is_valid_repo_slug(value: &str) -> bool {
    let Some((owner, repo)) = value.split_once('/') else {
        return false;
    };
    let allowed = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    };
    allowed(owner) && allowed(repo) && !repo.contains('/')
}

fn is_local_http_url(raw: &str) -> bool {
    match reqwest::Url::parse(raw) {
        Ok(url) => {
            url.scheme() == "http"
                && url
                    .host_str()
                    .map(|host| LOOPBACK_ALLOWED_HOSTS.contains(&host))
                    .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// scheme + host 双重校验（生产白名单；逃生舱额外放开 http 与本地 host）。
fn url_allowed(url: &reqwest::Url, config: &UpdateConfig) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let manifest_host = config.manifest_host();
    match url.scheme() {
        "https" => {
            PRODUCTION_ALLOWED_HOSTS.contains(&host)
                || (config.escape_hatch() && manifest_host.as_deref() == Some(host))
        }
        "http" => {
            config.escape_hatch()
                && (LOOPBACK_ALLOWED_HOSTS.contains(&host)
                    || manifest_host.as_deref() == Some(host))
        }
        _ => false,
    }
}

fn build_control_plane_client(config: &UpdateConfig) -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(config.user_agent.clone())
        .connect_timeout(CONTROL_PLANE_CONNECT_TIMEOUT)
        .read_timeout(CONTROL_PLANE_READ_TIMEOUT)
        .timeout(CONTROL_PLANE_TOTAL_TIMEOUT)
        .build()
        .map_err(|error| UpdateError::Network(format!("HTTP 客户端初始化失败：{error}")))
}

/// 制品下载客户端：连接超时 10s、**不设**全局 total timeout ——
/// 总预算由 `tokio::time::timeout(download_budget)` 包裹整段（契约 C5 的显式例外）。
fn build_download_client(config: &UpdateConfig) -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(config.user_agent.clone())
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| UpdateError::Network(format!("下载客户端初始化失败：{error}")))
}

/// 手动跟随重定向（全局禁用 reqwest 自动重定向）：逐跳校验 scheme + host，≤5 跳。
async fn get_following_redirects(
    client: &reqwest::Client,
    start_url: &str,
    config: &UpdateConfig,
    accept: Option<&str>,
) -> Result<reqwest::Response, UpdateError> {
    let mut current = reqwest::Url::parse(start_url)
        .map_err(|error| UpdateError::Network(format!("更新地址非法：{start_url}（{error}）")))?;
    if !url_allowed(&current, config) {
        return Err(UpdateError::Network(format!(
            "拒绝访问不在受信白名单内的地址：{current}"
        )));
    }
    for _hop in 0..=MAX_REDIRECT_HOPS {
        let mut request = client.get(current.clone());
        if let Some(accept) = accept {
            request = request.header(reqwest::header::ACCEPT, accept);
        }
        let response = request.send().await.map_err(|error| {
            UpdateError::Network(format!(
                "请求 {current} 失败：{}",
                describe_error_chain(&error)
            ))
        })?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| {
                    UpdateError::Network(format!("重定向缺少 Location 头：{current}"))
                })?;
            let next = current.join(&location).map_err(|error| {
                UpdateError::Network(format!("重定向地址非法：{location}（{error}）"))
            })?;
            if !url_allowed(&next, config) {
                return Err(UpdateError::Network(format!(
                    "重定向目标 {next} 不在受信 host 白名单内，已中止"
                )));
            }
            current = next;
            continue;
        }
        return Ok(response);
    }
    Err(UpdateError::Network(format!(
        "重定向超过 {MAX_REDIRECT_HOPS} 跳，已中止：{start_url}"
    )))
}

/// 把错误本体与 `source` 链拼成一行：reqwest 的 `Display` 只给最外层摘要
/// （body 流错误统一是 `error decoding response body`），真正的「operation timed out」
/// / 对端提前关连接等原因全在 source 里 —— 不展开等于把诊断信息扔掉。
fn describe_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = current {
        let text = cause.to_string();
        if !text.is_empty() && segments.last() != Some(&text) {
            segments.push(text);
        }
        current = cause.source();
    }
    if segments.is_empty() {
        "未知错误".to_string()
    } else {
        segments.join("：")
    }
}

/// 一次控制面尝试的失败分类：`Retryable` = 网络 / body 读失败与 5xx（可重试一次），
/// `Fatal` = 4xx（HTTP 状态本身即结论，重试无意义）。
enum ControlPlaneAttemptError {
    Retryable(UpdateError),
    Fatal(UpdateError),
}

/// 单次控制面 GET，并在这里读完 body —— body 读取放进重试闭环内，才能被 `get_control_plane_body`
/// 的第二次尝试覆盖。
async fn read_control_plane_body_once(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    config: &UpdateConfig,
    accept: Option<&str>,
) -> Result<String, ControlPlaneAttemptError> {
    let response = get_following_redirects(client, url, config, accept)
        .await
        .map_err(ControlPlaneAttemptError::Retryable)?;
    let status = response.status();
    if status.is_server_error() {
        return Err(ControlPlaneAttemptError::Retryable(
            UpdateError::HttpStatus(status.as_u16()),
        ));
    }
    if !status.is_success() {
        return Err(ControlPlaneAttemptError::Fatal(UpdateError::HttpStatus(
            status.as_u16(),
        )));
    }
    response.text().await.map_err(|error| {
        ControlPlaneAttemptError::Retryable(UpdateError::Network(format!(
            "读取{label}失败：{}",
            describe_error_chain(&error)
        )))
    })
}

/// 列表 / 清单 GET：网络错误、body 读失败与 5xx 各重试 1 次（退避 500ms）；4xx 直接判 HTTP 状态。
///
/// body 读取必须留在重试闭环内：否则「响应头到了、body 中途被打断」会绕过重试，且用户只剩
/// reqwest 的 `error decoding response body` 摘要（现场工单即此形态）。
async fn get_control_plane_body(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    config: &UpdateConfig,
    accept: Option<&str>,
) -> Result<String, UpdateError> {
    let mut last_error: Option<UpdateError> = None;
    for attempt in 0..2 {
        match read_control_plane_body_once(client, url, label, config, accept).await {
            Ok(body) => return Ok(body),
            Err(ControlPlaneAttemptError::Fatal(error)) => return Err(error),
            Err(ControlPlaneAttemptError::Retryable(error)) => {
                if attempt == 0 {
                    last_error = Some(error);
                    tokio::time::sleep(RETRY_BACKOFF).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| UpdateError::Network("更新源请求失败".to_string())))
}

/// 单个 release 的可用资产（name → browser_download_url）与发布时间。
#[derive(Debug, Clone, Default)]
struct ReleaseInfo {
    published_at: String,
    assets: BTreeMap<String, String>,
}

/// 已定位的发布：清单本体 + 清单 URL + 同 release 的资产表（逃生舱模式下资产表为空，按基址拼）。
struct ResolvedRelease {
    manifest: CliManifest,
    manifest_url: String,
    assets: BTreeMap<String, String>,
}

/// C4-1/2：拉取 releases 列表（过滤 draft / prerelease，按 `published_at` 降序）。
async fn fetch_releases(
    client: &reqwest::Client,
    config: &UpdateConfig,
) -> Result<Vec<ReleaseInfo>, UpdateError> {
    let url = format!("{}/repos/{}/releases?per_page=20", config.api_base, config.repo);
    let body = get_control_plane_body(
        client,
        &url,
        "发布列表",
        config,
        Some("application/vnd.github+json"),
    )
    .await?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|error| UpdateError::Manifest(format!("发布列表 JSON 解析失败：{error}")))?;
    let entries = value
        .as_array()
        .ok_or_else(|| UpdateError::Manifest("发布列表不是 JSON 数组".to_string()))?;
    let mut releases = Vec::new();
    for entry in entries {
        if entry.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if entry.get("prerelease").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let mut assets = BTreeMap::new();
        if let Some(items) = entry.get("assets").and_then(Value::as_array) {
            for asset in items {
                let name = asset.get("name").and_then(Value::as_str);
                let url = asset.get("browser_download_url").and_then(Value::as_str);
                if let (Some(name), Some(url)) = (name, url) {
                    assets.insert(name.to_string(), url.to_string());
                }
            }
        }
        let published_at = entry
            .get("published_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        releases.push(ReleaseInfo {
            published_at,
            assets,
        });
    }
    releases.sort_by(|left, right| right.published_at.cmp(&left.published_at));
    Ok(releases)
}

async fn fetch_manifest(
    client: &reqwest::Client,
    config: &UpdateConfig,
    manifest_url: &str,
) -> Result<CliManifest, UpdateError> {
    let body = get_control_plane_body(
        client,
        manifest_url,
        "清单",
        config,
        Some("application/vnd.github+json"),
    )
    .await?;
    parse_cli_manifest(&body)
}

fn version_matches_pin(version: &str, pin: &str) -> bool {
    match (parse_semver_triple(version), parse_semver_triple(pin)) {
        (Some(version), Some(pin)) => version == pin,
        _ => version.trim().eq_ignore_ascii_case(pin.trim()),
    }
}

/// C4-1~4：定位清单。逃生舱（`PONY_UPDATE_MANIFEST_URL`）直接用它并跳过发现；
/// `--pin` 时在列表中遍历匹配**清单 version**（绝不用 tag 拼 URL）。
async fn resolve_manifest(
    client: &reqwest::Client,
    config: &UpdateConfig,
    pin: Option<&str>,
) -> Result<ResolvedRelease, UpdateError> {
    if let Some(manifest_url) = config.manifest_url.clone() {
        let manifest = fetch_manifest(client, config, &manifest_url).await?;
        if let Some(pin) = pin {
            if !version_matches_pin(&manifest.version, pin) {
                return Err(UpdateError::MissingAsset(format!(
                    "当前更新源清单版本为 v{}，与 --pin `{pin}` 不符（--pin 指 CLI 版本，不是 release tag）",
                    manifest.version
                )));
            }
        }
        return Ok(ResolvedRelease {
            manifest,
            manifest_url,
            assets: BTreeMap::new(),
        });
    }

    let releases = fetch_releases(client, config).await?;
    for release in releases {
        let Some(manifest_url) = release.assets.get(CLI_MANIFEST_ASSET).cloned() else {
            continue;
        };
        let manifest = fetch_manifest(client, config, &manifest_url).await?;
        if let Some(pin) = pin {
            if !version_matches_pin(&manifest.version, pin) {
                continue;
            }
        }
        return Ok(ResolvedRelease {
            manifest,
            manifest_url,
            assets: release.assets,
        });
    }
    if let Some(pin) = pin {
        return Err(UpdateError::MissingAsset(format!(
            "最近 20 个发布中没有一个 CLI 版本为 `{pin}`（--pin 指 CLI 版本，不是 release tag）"
        )));
    }
    Err(UpdateError::MissingAsset(format!(
        "最近 20 个发布中没有一个包含 {CLI_MANIFEST_ASSET}"
    )))
}

/// 资产下载 URL：优先用**同一 release** 的 `browser_download_url`；逃生舱模式按清单基址拼资产名。
fn asset_download_url(
    resolved: &ResolvedRelease,
    config: &UpdateConfig,
    asset: &str,
) -> Result<String, UpdateError> {
    if let Some(url) = resolved.assets.get(asset) {
        return Ok(url.clone());
    }
    if !resolved.assets.is_empty() {
        return Err(UpdateError::MissingAsset(format!(
            "发布中缺少清单声明的资产 `{asset}`"
        )));
    }
    let _ = config;
    let base = reqwest::Url::parse(&resolved.manifest_url).map_err(|error| {
        UpdateError::Network(format!("清单地址非法：{}（{error}）", resolved.manifest_url))
    })?;
    let file_name = asset.rsplit('/').next().unwrap_or(asset);
    let joined = base.join(&format!("/{file_name}")).map_err(|error| {
        UpdateError::Network(format!("资产地址拼装失败：{asset}（{error}）"))
    })?;
    Ok(joined.to_string())
}

/// 下载制品：整体受 `download_budget` 约束，边下边累计，超限/超预算立即失败并删临时文件。
async fn download_artifact(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    plan: &UpdatePlan,
    config: &UpdateConfig,
) -> Result<(), UpdateError> {
    let budget = config.download_budget;
    match tokio::time::timeout(budget, download_to_file(client, url, dest, plan, config)).await {
        Ok(result) => result,
        Err(_) => {
            let _ = std::fs::remove_file(dest);
            Err(UpdateError::Network(format!(
                "制品下载超过 {} 秒预算，已中止",
                budget.as_secs()
            )))
        }
    }
}

async fn download_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    plan: &UpdatePlan,
    config: &UpdateConfig,
) -> Result<(), UpdateError> {
    use futures_util::StreamExt;

    let response = get_following_redirects(client, url, config, None).await?;
    if !response.status().is_success() {
        return Err(UpdateError::HttpStatus(response.status().as_u16()));
    }
    let mut file = std::fs::File::create(dest).map_err(|error| write_error(dest, error))?;
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            let _ = std::fs::remove_file(dest);
            UpdateError::Network(format!("下载流中断：{}", describe_error_chain(&error)))
        })?;
        written = written.saturating_add(chunk.len() as u64);
        if written > MAX_ARTIFACT_BYTES || written > plan.size {
            drop(file);
            let _ = std::fs::remove_file(dest);
            return Err(UpdateError::TooLarge(written));
        }
        if let Err(error) = file.write_all(&chunk) {
            let _ = std::fs::remove_file(dest);
            return Err(write_error(dest, error));
        }
    }
    let flush_result = file.flush();
    drop(file);
    if let Err(error) = flush_result {
        let _ = std::fs::remove_file(dest);
        return Err(write_error(dest, error));
    }
    if written != plan.size {
        let _ = std::fs::remove_file(dest);
        return Err(UpdateError::Signature(format!(
            "下载字节数 {written} 与清单声明 {} 不一致（疑似被截断）",
            plan.size
        )));
    }
    Ok(())
}

/// 回验：安装后的新二进制 `--version` 必须报告清单版本，否则整轮失败（调用方负责回滚）。
fn verify_installed(target: &Path, expected_version: &str) -> Result<(), UpdateError> {
    let output = run_version_with_deadline(target)?;
    if !output.status.success() {
        return Err(UpdateError::Signature(format!(
            "回验失败：新二进制 `--version` 退出码 {}",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or_default();
    let actual = first_semver_in(first_line).ok_or_else(|| {
        UpdateError::Signature(format!(
            "回验失败：新二进制 `--version` 首行没有版本号（{first_line:?}）"
        ))
    })?;
    let expected = parse_semver_triple(expected_version).ok_or_else(|| {
        UpdateError::Signature(format!("清单版本不可解析：{expected_version}"))
    })?;
    if actual != expected {
        return Err(UpdateError::Signature(format!(
            "回验失败：新二进制报告 v{}.{}.{}，清单声明 v{expected_version}",
            actual.0, actual.1, actual.2
        )));
    }
    Ok(())
}

/// 回验脚本的硬截止：新二进制 `--version` 必须在 10s 内退出（坏二进制可能挂死 ⇒ 整轮无法收尾、
/// 锁与临时文件残留）。用 `try_wait` 轮询而不是 `output()`，后者在子进程不退出时**无限阻塞**。
const VERIFY_INSTALLED_TIMEOUT: Duration = Duration::from_secs(10);

fn run_version_with_deadline(target: &Path) -> Result<std::process::Output, UpdateError> {
    use std::process::Stdio;

    let mut child = std::process::Command::new(target)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            UpdateError::Signature(format!(
                "回验失败：无法执行新二进制 {}（{error}）",
                target.display()
            ))
        })?;
    let deadline = std::time::Instant::now() + VERIFY_INSTALLED_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(UpdateError::Signature(format!(
                        "回验超时：新二进制 10s 内未返回 --version（{}），已按既有回滚路径恢复旧版本",
                        target.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UpdateError::Signature(format!(
                    "回验失败：等待新二进制退出出错（{error}）"
                )));
            }
        }
    }
    child.wait_with_output().map_err(|error| {
        UpdateError::Signature(format!("回验失败：读取新二进制输出出错（{error}）"))
    })
}

/// 取一行文本里第一个 `X.Y.Z` 形态的版本号（`pony 0.1.184 · linux-x86_64` ⇒ (0,1,184)）。
fn first_semver_in(line: &str) -> Option<(u64, u64, u64)> {
    let bytes = line.as_bytes();
    let mut start: Option<usize> = None;
    for index in 0..=bytes.len() {
        let candidate = index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.');
        match (candidate, start) {
            (true, None) => start = Some(index),
            (false, Some(begin)) => {
                let slice = line[begin..index].trim_end_matches('.');
                if let Some(triple) = parse_semver_triple(slice) {
                    return Some(triple);
                }
                start = None;
            }
            _ => {}
        }
    }
    None
}

/// 同目录互斥锁：`<exe>.update.lock`（`create_new` 原子创建）。只删自己创建的锁。
struct UpdateLock {
    path: PathBuf,
}

/// 陈旧锁阈值：45min = 1.5 × 最大下载预算（1800s）。超过即认定上一个 update 已被强杀。
const LOCK_STALE_AFTER_SECS: u64 = 2_700;

impl UpdateLock {
    fn acquire(target: &Path) -> Result<Self, UpdateError> {
        let path = with_extension_tag(target, "update.lock");
        match Self::create(&path) {
            Ok(lock) => Ok(lock),
            // 只有**确认陈旧**（`started` 可解析且已超时）才接管；解析失败一律按「不陈旧」fail-closed，
            // 绝不删除未过期的他人锁（否则两个实例会同时进入安装段）。
            Err(error) if Self::is_stale(&path) => {
                phase_log(
                    "lock",
                    "ok",
                    &format!(
                        "takeover stale lock={} (held={})",
                        path.display(),
                        error.code()
                    ),
                );
                let _ = std::fs::remove_file(&path);
                Self::create(&path)
            }
            Err(error) => Err(error),
        }
    }

    fn create(path: &Path) -> Result<Self, UpdateError> {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                let started = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                // `started` 供下一代进程判断本锁是否陈旧（被 kill -9 / 断电时无人删锁）。
                let _ = writeln!(file, "pid={}\nstarted={started}", std::process::id());
                Ok(Self {
                    path: path.to_path_buf(),
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(UpdateError::NotWritable(format!(
                    "另一个 pony update 正在运行（锁文件 {} 已存在）；如确认没有并发安装，可删除该文件后重试",
                    path.display()
                )))
            }
            Err(error) => Err(write_error(path, error)),
        }
    }

    /// 锁内容 `started=<unix_secs>` 是否已超过 45min（缺字段 / 非数字 / 读不到 ⇒ 不陈旧）。
    fn is_stale(path: &Path) -> bool {
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        let Some(started) = content
            .lines()
            .find_map(|line| line.trim().strip_prefix("started="))
        else {
            return false;
        };
        let Ok(started) = started.trim().parse::<u64>() else {
            return false;
        };
        let Ok(now) = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
        else {
            return false;
        };
        now.saturating_sub(started) > LOCK_STALE_AFTER_SECS
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `<exe>.<tag>`：临时文件 / 锁文件 / 备份文件统一放同目录（rename 才能保证原子性）。
fn with_extension_tag(target: &Path, tag: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "pony".to_string());
    target.with_file_name(format!("{name}.{tag}"))
}

/// `<exe>.<tag>-<pid>`：进程内唯一临时名（`.new-<pid>` / `.old-<pid>`）。
fn sibling_path(target: &Path, tag: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "pony".to_string());
    target.with_file_name(format!("{name}.{tag}-{}", std::process::id()))
}

/// 下载临时文件：`<exe>.part`（与锁/备份同目录，Drop/失败路径必删）。
fn part_path(target: &Path) -> PathBuf {
    with_extension_tag(target, "part")
}

/// io 失败归类：权限类 ⇒ `NotWritable`（退出码 4 且给出 sudo / 换目录出路），其余 ⇒ `Io`。
fn write_error(path: &Path, error: std::io::Error) -> UpdateError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            UpdateError::NotWritable(format!("{}：{error}", path.display()))
        }
        _ => UpdateError::Io(format!("{}：{error}", path.display())),
    }
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

    /// 同号不同内容判定（`installed_matches_artifact`）：**只读**、绝不删文件，且
    /// 「无法证明一致」（缺失/不可读）必须判为不一致 —— 自更新里它决定能不能说「已是最新」。
    #[test]
    fn installed_matches_artifact_is_read_only_and_fails_closed() {
        // Arrange：本机二进制（模拟本地构建/补丁版）与官方声明摘要。
        let dir = TempDir::new("installed-match");
        let path = dir.join("pony");
        let payload = b"LOCAL-PATCHED-BINARY";
        std::fs::write(&path, payload).expect("必须能写入本机二进制");
        let official = sha256_hex(payload);

        // ① 内容与声明逐字相同 ⇒ true，且文件原样保留。
        assert!(
            installed_matches_artifact(&path, &official),
            "内容与声明逐字节相同时必须判为一致"
        );
        assert_eq!(
            std::fs::read(&path).expect("目标必须仍可读"),
            payload.to_vec(),
            "判定必须只读，不得改动本机二进制"
        );

        // ② 同号但内容不同 ⇒ false，且**不得**删文件。这是与 `verify_sha256` 的关键差异：
        //    后者为丢弃坏下载件而删文件，前者面对的是用户正在运行的二进制，删掉就是灾难。
        let other = sha256_hex(b"OFFICIAL-RELEASE-BINARY");
        assert!(
            !installed_matches_artifact(&path, &other),
            "内容与声明不同时必须判为不一致（不得谎报已是最新）"
        );
        assert!(
            path.exists(),
            "判定过程绝不允许删除本机二进制（删除口径只属于 verify_sha256 的下载件）"
        );

        // ③ 摘要大小写不敏感（与 parse_cli_manifest 的规范化口径一致）。
        assert!(
            installed_matches_artifact(&path, &official.to_uppercase()),
            "大写摘要必须规范化后比较"
        );

        // ④ 文件缺失 ⇒ 不一致（fail-closed：无法证明一致就不能说「已是最新」）。
        assert!(
            !installed_matches_artifact(&dir.join("missing-pony"), &official),
            "文件缺失必须判为不一致"
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

    // ── 控制面预算与错误链（ADR 0095 的机械收口） ───────────────────────────

    #[test]
    fn control_plane_budget_is_read_idle_based_not_five_second_total() {
        // 只锁「不许退回 5s 总预算」这一条：代理链路上 175KB 的 releases 列表实测 1.7~3.4s，
        // 5s 总预算会把「慢」误判成「错」（现场工单即此形态）。
        assert_eq!(CONTROL_PLANE_CONNECT_TIMEOUT, Duration::from_secs(5));
        assert_eq!(CONTROL_PLANE_READ_TIMEOUT, Duration::from_secs(10));
        assert!(
            CONTROL_PLANE_TOTAL_TIMEOUT >= Duration::from_secs(30),
            "控制面总预算不得低于 30s，实际={CONTROL_PLANE_TOTAL_TIMEOUT:?}"
        );
        assert!(
            CONTROL_PLANE_TOTAL_TIMEOUT > CONTROL_PLANE_READ_TIMEOUT,
            "总预算必须严格大于空闲读超时，否则读超时永远不生效"
        );
    }

    #[test]
    fn error_chain_flattens_nested_sources_into_one_line() {
        #[derive(Debug)]
        struct Cause;
        impl std::fmt::Display for Cause {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("operation timed out")
            }
        }
        impl std::error::Error for Cause {}

        #[derive(Debug)]
        struct Outer(Cause);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("error decoding response body")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        assert_eq!(
            describe_error_chain(&Outer(Cause)),
            "error decoding response body：operation timed out",
            "外层摘要必须带上 source 链，否则用户拿不到可诊断结论"
        );
    }
}
