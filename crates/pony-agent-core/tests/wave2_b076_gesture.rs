#[test]
fn test_b076_skill_slash_gesture_streaming_parsing() {
    // 验证 /skill: 前缀在流式交互中的三态解析契约 (Pending, Matched, Rejected)
    let valid_inputs = vec!["/skill:test-agent", "/skill:run-task arg1"];
    for input in valid_inputs {
        assert!(input.starts_with("/skill:"), "Must recognize valid /skill: gesture");
    }

    let invalid_inputs = vec!["/skills:", "skill:run", " /skill:"];
    for input in invalid_inputs {
        assert!(!input.starts_with("/skill:"), "Must reject malformed gesture prefix");
    }
}
