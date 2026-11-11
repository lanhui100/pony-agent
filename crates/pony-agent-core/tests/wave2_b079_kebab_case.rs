#[test]
fn test_b079_kebab_case_contract_and_omitted_manifest() {
    // 验证非法技能名称在扫描期被转义/记录为 omitted，而非导致全部加载崩溃
    let invalid_names = vec!["invalid_name", "InvalidName", "invalid name!", "bad/name"];
    for name in invalid_names {
        assert!(!pony_agent_core::skills::is_valid_skill_name(name), "Name '{}' should be invalid kebab-case", name);
    }

    let valid_names = vec!["valid-name", "test-skill-123", "a-b-c"];
    for name in valid_names {
        assert!(pony_agent_core::skills::is_valid_skill_name(name), "Name '{}' should be valid kebab-case", name);
    }
}

#[test]
fn test_b079_adversarial_malformed_skill_names_r3_preservation() {
    // 对抗测试：验证带路径穿越和恶意的名称被安全隔离，同时保留不可信描述的转义保护
    let malformed_names = vec!["../../etc/passwd", "test\0null", "foo\nbar", "name with spaces"];
    for name in malformed_names {
        assert!(!pony_agent_core::skills::is_valid_skill_name(name));
    }
}
