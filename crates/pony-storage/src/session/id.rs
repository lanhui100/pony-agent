//! 会话 id 的唯一真值源：格式、生成与分类全部收敛在此。
//!
//! 规范：`pa_<uuidv7>`（RFC 9562 version 7，小写连字符 8-4-4-4-12）。
//!
//! 桌面端在渲染进程内生成同格式 id（`src/lib/runtime/sessions.ts` 的
//! `createNextSessionId`）：新建会话的瞬间就要落本地状态，绕后端往返没有收益。
//! 因此两端共享的是**格式契约**而不是实现，各自测试锁定同一正则。
//!
//! 历史格式 `session-<8hex>`（旧 TUI）与 `cli-session-<uuidv4>`（旧 CLI）仍然
//! 可读、可 resume，只是不再生成——兼容读、统一写。

use uuid::Uuid;

/// 规范会话 id 的前缀。
pub const SESSION_ID_PREFIX: &str = "pa_";

/// `new_session_id_unless` 查重时的最大尝试次数。
///
/// uuidv7 在单毫秒内仍有 74 bit 随机位，连续冲突 16 次在物理上不可能发生；
/// 该上限只是防止调用方的 `exists` 判定恒真时把生成函数变成死循环。
pub const MAX_UNIQUE_ATTEMPTS: usize = 16;

const LEGACY_SESSION_PREFIX: &str = "session-";
const LEGACY_CLI_SESSION_PREFIX: &str = "cli-session-";
const SUBAGENT_SESSION_PREFIX: &str = "subagent-";
const TEAMMATE_SESSION_PREFIX: &str = "teammate-";

/// 生成规范主会话 id：`pa_` + UUIDv7。
///
/// 不查重；需要冲突检测时用 [`new_session_id_unless`]。
pub fn new_session_id() -> String {
    format!("{SESSION_ID_PREFIX}{}", Uuid::now_v7())
}

/// 生成规范主会话 id；`exists(&candidate)` 为 true 时重新生成。
///
/// 连续 [`MAX_UNIQUE_ATTEMPTS`] 次都冲突时返回 `None`，由调用方决定兜底策略
/// （通常是再调一次 [`new_session_id`]）。不 panic：`exists` 是调用方提供的
/// 判定，库函数不应该替它判死刑。
pub fn new_session_id_unless<F>(mut exists: F) -> Option<String>
where
    F: FnMut(&str) -> bool,
{
    (0..MAX_UNIQUE_ATTEMPTS)
        .map(|_| new_session_id())
        .find(|candidate| !exists(candidate))
}

/// 严格判定：`pa_` 前缀 + 标准小写连字符形式 + RFC 9562 version 7。
///
/// 32 位 simple、大写、`urn:uuid:`、带花括号等非规范形态一律拒绝——这些形态
/// 无法与桌面端生成的 id 在字符串层面等价比较，放进 `sessions.json` 会制造
/// 「看起来是同一个会话但键不同」的重复记录。
pub fn is_canonical_session_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix(SESSION_ID_PREFIX) else {
        return false;
    };
    match Uuid::parse_str(rest) {
        Ok(parsed) => {
            parsed.get_version_num() == 7 && parsed.hyphenated().to_string() == rest
        }
        Err(_) => false,
    }
}

/// 会话 id 的形态分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdKind {
    /// 现行规范：`pa_<uuidv7>`。
    Canonical,
    /// 历史入口生成的主会话 id（旧 TUI / 旧 CLI），继续按主会话对待。
    LegacyMain,
    /// 子代理会话。
    Subagent,
    /// 队友会话（只经 `/agents` 入口）。
    Teammate,
    /// 其他（含用户手填的任意 id）。
    Other,
}

/// 按前缀/格式对会话 id 分类，判定顺序即优先级。
pub fn classify_session_id(id: &str) -> SessionIdKind {
    if is_canonical_session_id(id) {
        return SessionIdKind::Canonical;
    }
    if id.starts_with(SUBAGENT_SESSION_PREFIX) {
        return SessionIdKind::Subagent;
    }
    if id.starts_with(TEAMMATE_SESSION_PREFIX) {
        return SessionIdKind::Teammate;
    }
    if id.starts_with(LEGACY_SESSION_PREFIX) || id.starts_with(LEGACY_CLI_SESSION_PREFIX) {
        return SessionIdKind::LegacyMain;
    }
    SessionIdKind::Other
}

/// 主会话判定：规则与改造前 TUI 的实现**逐字节等价**——只排除队友与子代理。
///
/// 刻意不做「必须是 `pa_` 前缀」的白名单：历史 `session-xxxxxxxx` /
/// `cli-session-<uuid>` 会话仍要出现在 `/resume` 默认列表；收紧成白名单会让
/// 存量用户会话凭空消失，属用户可见回归。
pub fn is_main_session_id(id: &str) -> bool {
    !matches!(
        classify_session_id(id),
        SessionIdKind::Subagent | SessionIdKind::Teammate
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn new_session_id_has_pa_prefix_and_uuid_v7_shape() {
        let id = new_session_id();
        assert!(id.starts_with(SESSION_ID_PREFIX), "prefix: {id}");
        let rest = id.strip_prefix(SESSION_ID_PREFIX).expect("prefix stripped");
        assert_eq!(rest.len(), 36, "hyphenated uuid length: {id}");
        let parsed = Uuid::parse_str(rest).expect("uuid parse");
        assert_eq!(parsed.get_version_num(), 7, "version: {id}");
        assert_eq!(parsed.get_variant(), uuid::Variant::RFC4122, "variant: {id}");
        assert!(is_canonical_session_id(&id), "canonical: {id}");
    }

    #[test]
    fn new_session_id_is_unique_across_1000_calls() {
        let ids: HashSet<String> = (0..1000).map(|_| new_session_id()).collect();
        assert_eq!(ids.len(), 1000, "uuidv7 连续生成出现重复");
    }

    #[test]
    fn canonical_check_rejects_non_canonical_forms() {
        let id = new_session_id();
        let rest = id.strip_prefix(SESSION_ID_PREFIX).expect("prefix");
        let simple = rest.replace('-', "");
        assert!(!is_canonical_session_id(&format!("{SESSION_ID_PREFIX}{simple}")));
        assert!(!is_canonical_session_id(&format!(
            "{SESSION_ID_PREFIX}{}",
            rest.to_uppercase()
        )));
        assert!(!is_canonical_session_id(&format!("urn:uuid:{rest}")));
        assert!(!is_canonical_session_id(&format!("{{{rest}}}")));
        assert!(!is_canonical_session_id(rest));
        assert!(!is_canonical_session_id(""));
    }

    #[test]
    fn canonical_check_rejects_non_v7_uuid() {
        let v4 = format!("{SESSION_ID_PREFIX}{}", Uuid::new_v4());
        assert!(!is_canonical_session_id(&v4));
    }

    #[test]
    fn classify_covers_every_known_shape() {
        assert_eq!(
            classify_session_id(&new_session_id()),
            SessionIdKind::Canonical
        );
        assert_eq!(
            classify_session_id("session-1a2b3c4d"),
            SessionIdKind::LegacyMain
        );
        assert_eq!(
            classify_session_id("cli-session-1a2b3c4d-0000-4000-8000-000000000000"),
            SessionIdKind::LegacyMain
        );
        assert_eq!(
            classify_session_id("subagent-1a2b3c4d"),
            SessionIdKind::Subagent
        );
        assert_eq!(
            classify_session_id("teammate-reviewer"),
            SessionIdKind::Teammate
        );
        assert_eq!(classify_session_id("custom-id"), SessionIdKind::Other);
        assert_eq!(classify_session_id(""), SessionIdKind::Other);
    }

    #[test]
    fn is_main_session_id_matches_legacy_semantics() {
        assert!(is_main_session_id(""));
        assert!(is_main_session_id("session-1a2b3c4d"));
        assert!(is_main_session_id("cli-session-abc"));
        assert!(is_main_session_id("custom-id"));
        assert!(is_main_session_id(&new_session_id()));
        assert!(!is_main_session_id("subagent-abc"));
        assert!(!is_main_session_id("teammate-abc"));
    }

    #[test]
    fn new_session_id_unless_retries_on_conflict() {
        let mut probes = 0usize;
        let picked = new_session_id_unless(|_| {
            probes += 1;
            probes == 1
        })
        .expect("首次冲突后应能拿到未冲突 id");
        assert_eq!(probes, 2, "只应在首次冲突后重试一次");
        assert!(is_canonical_session_id(&picked), "重试结果仍是规范 id");
    }

    #[test]
    fn new_session_id_unless_gives_up_without_panicking() {
        assert_eq!(new_session_id_unless(|_| true), None);
    }
}
