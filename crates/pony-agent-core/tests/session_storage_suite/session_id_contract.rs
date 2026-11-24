//! 会话 id 格式契约（跨入口）：`pa_<uuidv7>`。
//!
//! 与桌面端渲染进程的契约测试 `tests/sidebar-groups.spec.ts::createNextSessionId
//! generates unique ids with pa_ prefix and UUIDv7 format` 锁定同一形状：
//!
//! ```text
//! ^pa_[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$
//! ```
//!
//! 这里从 crate 外部只经公开 API 断言，防止 core 单测与真实导出面漂移。

use std::collections::HashSet;

use pony_agent_core::agent::session::{
    classify_session_id, is_canonical_session_id, is_main_session_id, new_session_id,
    new_session_id_unless, SessionIdKind, MAX_UNIQUE_ATTEMPTS, SESSION_ID_PREFIX,
};

/// 与前端契约测试逐字符同形的形状检查（不引入 regex 依赖）。
fn matches_contract_shape(id: &str) -> bool {
    let Some(rest) = id.strip_prefix(SESSION_ID_PREFIX) else {
        return false;
    };
    let bytes = rest.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            14 => {
                if *byte != b'7' {
                    return false;
                }
            }
            19 => {
                if !matches!(byte, b'8' | b'9' | b'a' | b'b') {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte) {
                    return false;
                }
            }
        }
    }
    true
}

#[test]
fn generated_id_matches_shared_contract_shape() {
    for _ in 0..64 {
        let id = new_session_id();
        assert!(
            matches_contract_shape(&id),
            "生成结果不符合前端同款正则：{id}"
        );
        assert!(is_canonical_session_id(&id), "生成结果必须是规范形态：{id}");
    }
}

#[test]
fn generated_ids_are_unique_and_timestamp_ordered() {
    let ids: Vec<String> = (0..1000).map(|_| new_session_id()).collect();
    assert_eq!(
        ids.iter().collect::<HashSet<_>>().len(),
        1000,
        "uuidv7 连续生成出现重复"
    );
    // 前 8 位是毫秒时间戳：桌面端侧边栏「最新在前」的排序依赖时间有序性，
    // 若退回 uuidv4 则排序随机。同毫秒内不约束到纳秒级（那是实现细节，不是契约）。
    let stamps: Vec<&str> = ids.iter().map(|id| &id[SESSION_ID_PREFIX.len()..][..8]).collect();
    assert!(
        stamps.windows(2).all(|pair| pair[0] <= pair[1]),
        "uuidv7 时间戳前缀必须单调不减"
    );
}

#[test]
fn dedupe_generator_skips_existing_keys() {
    let mut probes = 0usize;
    let picked = new_session_id_unless(|_| {
        probes += 1;
        probes == 1
    })
    .expect("冲突一次后应能拿到新 id");
    assert_eq!(probes, 2, "只应在首次冲突后重试一次");
    assert!(is_canonical_session_id(&picked), "重试结果仍是规范 id");
}

#[test]
fn dedupe_generator_gives_up_after_bounded_attempts() {
    let mut probes = 0usize;
    let picked = new_session_id_unless(|_| {
        probes += 1;
        true
    });
    assert_eq!(picked, None, "恒冲突时必须返回 None 而不是死循环/panic");
    assert_eq!(probes, MAX_UNIQUE_ATTEMPTS, "尝试次数必须受上限约束");
}

#[test]
fn legacy_ids_stay_readable_and_classified() {
    // 兼容读：存量会话仍按主会话对待（/resume 默认列表不得凭空少条目）。
    for legacy in ["session-1a2b3c4d", "cli-session-1a2b3c4d", "custom-id", ""] {
        assert!(
            is_main_session_id(legacy),
            "存量主会话 id 必须继续可见：{legacy}"
        );
    }
    assert_eq!(
        classify_session_id("session-1a2b3c4d"),
        SessionIdKind::LegacyMain
    );
    assert_eq!(
        classify_session_id("cli-session-1a2b3c4d"),
        SessionIdKind::LegacyMain
    );
    assert!(!is_main_session_id("subagent-1a2b3c4d"));
    assert!(!is_main_session_id("teammate-reviewer"));
}

#[test]
fn canonical_check_rejects_non_canonical_encodings() {
    let id = new_session_id();
    let rest = id.strip_prefix(SESSION_ID_PREFIX).expect("前缀");
    let simple = rest.replace('-', "");
    let upper = rest.to_uppercase();
    for candidate in [
        format!("{SESSION_ID_PREFIX}{simple}"),
        format!("{SESSION_ID_PREFIX}{upper}"),
        format!("urn:uuid:{rest}"),
        format!("{{{rest}}}"),
        rest.to_string(),
        String::new(),
        // v4 不属于规范形态：时间有序性是契约的一部分。
        format!("{SESSION_ID_PREFIX}{}", uuid_v4_like()),
    ] {
        assert!(
            !is_canonical_session_id(&candidate),
            "非规范形态不得通过：{candidate}"
        );
    }
}

/// 构造一个 version=4 的 uuid 字面量（不引入 uuid 依赖，手写常量）。
fn uuid_v4_like() -> &'static str {
    "0f8fad5b-d9cb-469f-a165-70867728950e"
}
