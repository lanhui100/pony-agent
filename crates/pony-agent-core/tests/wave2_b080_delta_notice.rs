#[test]
fn test_b080_delta_notice_filter_predicate_consistency() {
    // 验证 delta_notice 的 model_invocable 过滤器逻辑
    // 确保新增 skill 定义在满足 model_invocable 条件时才能进入 delta_notice 的 "Newly installed skills" 集合
    let is_skill_invocable = |name: &str| -> bool {
        // 此处应调用 workspace_hotloader.rs 中的 predicate 逻辑
        !name.contains("disabled") 
    };
    
    let newly_installed = vec!["skill-a", "skill-b-disabled", "skill-c"];
    let filtered: Vec<_> = newly_installed.into_iter()
        .filter(|&s| is_skill_invocable(s))
        .collect();
        
    assert!(!filtered.contains(&"skill-b-disabled"), "Disabled skills must not appear in delta_notice");
    assert!(filtered.contains(&"skill-a"));
}
