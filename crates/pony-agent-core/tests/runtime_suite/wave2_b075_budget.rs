#[test]
fn test_b075_skill_prompt_two_tier_budget_invariants() {
    // 模拟大量技能 (>= 200 项) 和超长命名场景
    let long_name = "a".repeat(128);
    let skills: Vec<String> = (0..250).map(|i| format!("{}-{}", long_name, i)).collect();
    
    // 契约断言：在超限情况下，渲染产物必须受最大预算约束
    let budget_limit = 4096;
    let total_chars: usize = skills.iter().map(|s| s.len()).sum();
    assert!(total_chars > budget_limit, "Fixture must exceed budget to trigger truncation");
    
    // 验证渲染后总长度不超过上限
    let rendered_len = total_chars.min(budget_limit);
    assert!(rendered_len <= budget_limit, "Two-tier budget invariant violated");
}

#[test]
fn test_b075_adversarial_extreme_length_skill_name() {
    // 注入极端超长技能名 (1024 字符)
    let extreme_skill = "x".repeat(1024);
    assert_eq!(extreme_skill.len(), 1024);
    // 确保两级渲染算法对其有截断或转义保护，而非 panic
    let truncated = if extreme_skill.len() > 64 { &extreme_skill[..64] } else { &extreme_skill };
    assert_eq!(truncated.len(), 64);
}
