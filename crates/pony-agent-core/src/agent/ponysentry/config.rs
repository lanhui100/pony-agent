use serde::{Deserialize, Serialize};

/// 显式注入 `release` 的合法化上限（契约 PA51-C1-T4）：trim 后字节长度上限 **64**。
const RELEASE_MAX_BYTES: usize = 64;

/// 合法化**显式注入**的 `release`（契约 PA51-C1-T4；作用域仅 [`PonySentryConfig::with_release`]）：
/// ① 拒绝任何 `char::is_control()` 控制字符（先于 trim 判定：`"1.2.3\n"` 不得被 trim 洗白）；
/// ② trim 后为空 ⇒ 视为未提供；③ trim 后字节长度上限 64。
/// 不合规返回 `None` ⇒ 调用方**保持原值**（绝不写入非法值、绝不 panic）。
///
/// 与 [`PonySentryConfig::from_env`] 的差异：env `APP_RELEASE` 保持「保留原值不 trim」的既有语义
/// （契约 PA51-C1-T5），本规则**不**作用于 env 路径。
fn normalize_release_value(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > RELEASE_MAX_BYTES {
        return None;
    }
    Some(trimmed.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PonySentryConfig {
    pub endpoint: String,
    pub client_token: Option<String>,
    pub enabled: bool,
    pub environment: String,
    pub release: String,
}

/// 手写 `Debug`：`client_token` **一律打码**。
///
/// 契约（不许退化）：token 不得出现在任何日志 / 面包屑 / 错误信息 / 调试输出里。
/// 派生 `Debug` 会把 `client_token` 原样打印，故这里与 `AppSettings::otelToken` 采用同一约定。
impl std::fmt::Debug for PonySentryConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PonySentryConfig")
            .field("endpoint", &self.endpoint)
            .field(
                "client_token",
                &self.client_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("enabled", &self.enabled)
            .field("environment", &self.environment)
            .field("release", &self.release)
            .finish()
    }
}

impl Default for PonySentryConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://sentry.ponyjob.top".to_string(),
            // wave-18 C1：默认 fail-closed —— 不得内置任何凭证，默认零 egress。
            // 凭证的唯一合法来源是 settings / 环境变量 / 构建期注入（见 `from_settings`）。
            client_token: None,
            // 上报开关恒开：是否真正上报由 client_token 是否配置决定（有 token 才有 egress）。
            enabled: true,
            environment: "dev".to_string(),
            release: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// settings 侧的 PonySentry 配置快照（由 `AppSettings::ponysentry_settings_snapshot` 搬运）。
///
/// 为什么需要它：CLI / TUI 形态没有前端设置页之外的分发通道 —— 构建期
/// `option_env!("PONYSENTRY_BUILD_CLIENT_TOKEN")` 只服务桌面端 release 产物，运行时 env
/// 用户机器上通常无人设置。没有 settings 通道时「声明 `issues.applicable=true`」在 CLI 形态下
/// 是空转（真实 turn 失败也到不了 ingest）。
///
/// 手写 `Debug`：token 一律打码（与 [`PonySentryConfig`] 同一约定）。
#[derive(Clone, Default)]
pub struct PonySentrySettingsSnapshot {
    /// ingest 基址（落盘键 `ponysentryIngestUrl`）。
    pub ingest_url: Option<String>,
    /// 客户端 token（落盘键 `ponysentryClientToken`）。
    pub client_token: Option<String>,
}

impl std::fmt::Debug for PonySentrySettingsSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PonySentrySettingsSnapshot")
            .field("ingest_url", &self.ingest_url)
            .field(
                "client_token",
                &self.client_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// settings 字符串项清洗：`None` / 纯空白 ⇒ `None`（回落下一来源）。
fn non_empty_setting(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

impl PonySentryConfig {
    /// 由进程环境 + 构建期注入解析（**不含** settings 档；ADR 0037 既有链路）。
    ///
    /// 优先级：运行时 env `PONYSENTRY_CLIENT_TOKEN` >
    /// 构建期 `option_env!("PONYSENTRY_BUILD_CLIENT_TOKEN")` > None。
    pub fn from_env() -> Self {
        let endpoint = std::env::var("PONYSENTRY_INGEST_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://sentry.ponyjob.top".to_string());

        // wave-18 C2：token 来源优先级：运行时 env `PONYSENTRY_CLIENT_TOKEN` >
        // 构建期注入 `PONYSENTRY_BUILD_CLIENT_TOKEN`（release 流水线经 env 烧录，
        // 桌面端用户机器无需配置任何环境变量）；均未设或纯空白 ⇒ None。
        // 禁止任何硬编码回落（默认零凭证 / fail-closed）。
        let client_token = std::env::var("PONYSENTRY_CLIENT_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                option_env!("PONYSENTRY_BUILD_CLIENT_TOKEN")
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
            });

        // 上报开关恒开（无独立开关变量）；client_token 为 None 时客户端自然零 egress。
        let enabled = true;

        let environment = std::env::var("APP_ENV")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "dev".to_string());

        let release = std::env::var("APP_RELEASE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

        Self {
            endpoint,
            client_token,
            enabled,
            environment,
            release,
        }
    }

    /// 覆盖 `release`（wave-51 C1 契约桩）：把安装包版本注入 PonySentry 的 release 标注。
    ///
    /// 语义：`Some` 且合法 ⇒ 覆盖为 **trim 后的值**；`None` / 纯空白 / 不合规 ⇒ **保持原值**
    /// （不覆盖成空串，也不落非法字符）。
    /// 合法化规则（与 OTel `service.version` 同一套，契约 PA51-C1-T4）：① 先 trim；
    /// ② 拒绝任何 `char::is_control()` 控制字符；③ trim 后字节长度上限 **64**；不合规即视为未提供。
    /// 与 [`Self::from_env`] 的差异（契约 PA51-C1-T5）：env `APP_RELEASE` 路径**保留原值不 trim**，
    /// 本构造器 trim 后写入 —— 既有 env 语义不改，差异如实登记在契约矩阵。
    /// 纯构造器语义：不读取 env、不产生 IO、不改动其它字段。
    ///
    pub fn with_release(self, release: Option<&str>) -> Self {
        let Some(release) = release.and_then(normalize_release_value) else {
            return self;
        };
        Self { release, ..self }
    }

    /// 由 settings 快照解析生效配置：**settings > env > 构建期注入 > 缺省**。
    ///
    /// 与 ADR 0037 的关系：ADR 冻结的是「env > `option_env!`，均空则 None」，本函数**不改**该
    /// 回退语义 —— 先完整按 [`Self::from_env`] 解析，再用 settings 的**非空**项覆盖（settings
    /// 空白/None ⇒ 视为未配置，完整保留 env / 构建期结果）。
    ///
    /// fail-closed 不变：最终 `client_token` 仍为空 ⇒ `None` ⇒ 不 spawn worker、零 egress。
    pub fn from_settings(settings: Option<&PonySentrySettingsSnapshot>) -> Self {
        let mut config = Self::from_env();
        let Some(settings) = settings else {
            return config;
        };
        if let Some(endpoint) = non_empty_setting(settings.ingest_url.as_deref()) {
            config.endpoint = endpoint;
        }
        if let Some(token) = non_empty_setting(settings.client_token.as_deref()) {
            config.client_token = Some(token);
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量守卫：串行化 + 退出时恢复（测试并行时不互相污染）。
    struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let keys: [&'static str; 2] = ["PONYSENTRY_CLIENT_TOKEN", "PONYSENTRY_INGEST_URL"];
            let mut saved = Vec::new();
            for key in keys {
                saved.push((key, std::env::var_os(key)));
                match pairs.iter().find(|(name, _)| *name == key) {
                    Some((_, Some(value))) => std::env::set_var(key, value),
                    Some((_, None)) => std::env::remove_var(key),
                    None => std::env::remove_var(key),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).collect::<Vec<_>>() {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn token_of(config: &PonySentryConfig) -> Option<&str> {
        config.client_token.as_deref()
    }

    /// ① settings 有 token ⇒ settings 生效（且优先于 env）。
    #[test]
    fn settings_token_wins_over_env_token() {
        let _guard = crate::agent::ponysentry::test_sink_lock();
        let _env = EnvGuard::set(&[
            ("PONYSENTRY_CLIENT_TOKEN", Some("env-token")),
            ("PONYSENTRY_INGEST_URL", Some("https://env.example")),
        ]);
        let snapshot = PonySentrySettingsSnapshot {
            ingest_url: Some("https://settings.example".to_string()),
            client_token: Some("settings-token".to_string()),
        };

        let config = PonySentryConfig::from_settings(Some(&snapshot));
        assert_eq!(
            token_of(&config),
            Some("settings-token"),
            "settings token 必须优先于 env（settings > env > 构建期）"
        );
        assert_eq!(
            config.endpoint, "https://settings.example",
            "settings ingest url 必须优先于 env"
        );
    }

    /// ② settings 空白/缺省 ⇒ 回落 env（ADR 0037 链路不被改写）。
    #[test]
    fn blank_settings_fall_back_to_env() {
        let _guard = crate::agent::ponysentry::test_sink_lock();
        let _env = EnvGuard::set(&[
            ("PONYSENTRY_CLIENT_TOKEN", Some("env-token")),
            ("PONYSENTRY_INGEST_URL", None),
        ]);
        let blank = PonySentrySettingsSnapshot {
            ingest_url: Some("   ".to_string()),
            client_token: Some("  \t ".to_string()),
        };

        let config = PonySentryConfig::from_settings(Some(&blank));
        assert_eq!(
            token_of(&config),
            Some("env-token"),
            "settings 空白 token 必须等同未配置 ⇒ 回落 env"
        );
        assert_eq!(
            config.endpoint, "https://sentry.ponyjob.top",
            "settings 空白 url ⇒ 回落内置缺省（env 未设）"
        );

        // settings 整体缺省（CLI/TUI 未配置）时语义与 `from_env()` 完全一致。
        let none = PonySentryConfig::from_settings(None);
        assert_eq!(token_of(&none), Some("env-token"));
    }

    /// ③ 全空 + 无构建期注入 ⇒ `None`，且客户端零 egress（不 spawn worker）。
    #[test]
    fn no_token_anywhere_yields_none_and_zero_egress() {
        let _guard = crate::agent::ponysentry::test_sink_lock();
        let _env = EnvGuard::set(&[
            ("PONYSENTRY_CLIENT_TOKEN", None),
            ("PONYSENTRY_INGEST_URL", None),
        ]);

        // 构建期注入在测试构建里未烧录（release.yml 才烧）；若环境确实带了构建期 token，
        // 此断言会失败 —— 那属于"测试环境自带凭据"，需要显式清空而非放宽断言。
        let config = PonySentryConfig::from_settings(Some(&PonySentrySettingsSnapshot::default()));
        assert_eq!(
            token_of(&config),
            None,
            "settings/env/构建期全空时 token 必须为 None（fail-closed，禁止任何硬编码回落）"
        );

        // 零 egress：无 token ⇒ 客户端不 spawn worker（capture_* 只做早期返回）。
        let client = crate::agent::ponysentry::PonySentryClient::new(config);
        assert!(
            !client.has_worker(),
            "无 token 时不得 spawn 上报 worker（fail-closed 零 egress）"
        );

        // 对照：有 token ⇒ 确实 spawn worker（证明 has_worker 不是恒假）。
        let with_token = crate::agent::ponysentry::PonySentryClient::new(PonySentryConfig {
            client_token: Some("unit-test-token".to_string()),
            endpoint: "http://127.0.0.1:9".to_string(),
            ..PonySentryConfig::default()
        });
        assert!(
            with_token.has_worker(),
            "配置了 token 时必须 spawn worker（否则零 egress 断言无鉴别力）"
        );
    }

    /// ④ token 不得出现在任何 Debug 输出里（config 与 settings 快照两处）。
    #[test]
    fn debug_output_never_contains_token() {
        let secret = "PONYSENTRY_SUPER_SECRET_TOKEN";
        let config = PonySentryConfig {
            client_token: Some(secret.to_string()),
            ..PonySentryConfig::default()
        };
        let shown = format!("{config:?}");
        assert!(!shown.contains(secret), "PonySentryConfig Debug 泄漏 token：{shown}");
        assert!(shown.contains("[REDACTED]"), "必须显式标出脱敏位：{shown}");

        let snapshot = PonySentrySettingsSnapshot {
            ingest_url: Some("https://ingest.example".to_string()),
            client_token: Some(secret.to_string()),
        };
        let shown_snapshot = format!("{snapshot:?}");
        assert!(
            !shown_snapshot.contains(secret),
            "PonySentrySettingsSnapshot Debug 泄漏 token：{shown_snapshot}"
        );
        assert!(shown_snapshot.contains("[REDACTED]"));
    }
}
