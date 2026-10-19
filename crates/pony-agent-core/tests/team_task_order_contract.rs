//! 独立验收契约测试：Team 任务板「生成次序」排序契约
//!
//! 契约来源（本轮变更）：
//! - `TeamTaskItem.created_at_ms: u64` 由 `team_task_create` 写入 `now_unix_ms()`
//! - `team_task_list()` 按 `created_at_ms` 升序排序，同毫秒以 `task_id` 升序为次级键
//!
//! 说明：`TEAM_TASKS` 为进程级全局单例，创建的任务会在同一测试进程内累积，
//! 因此全部断言集中在单个 `#[test]` 函数内完成，且不对任务总数做强断言。

use pony_agent_core::agent::tools::{
    team_task_create, team_task_list, TeamTaskCreateArgs, TeamTaskItem,
};
use std::collections::HashSet;

fn create(subject: &str) -> TeamTaskItem {
    team_task_create(TeamTaskCreateArgs {
        subject: subject.to_string(),
        description: format!("contract probe: {}", subject),
        write_scopes: None,
        blocked_by: None,
    })
    .expect("team_task_create must succeed")
}

/// 校验一个任务序列满足全序：(created_at_ms asc, task_id asc)
fn assert_sorted_by_created_then_id(tasks: &[TeamTaskItem], ctx: &str) {
    for pair in tasks.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let ordered = a.created_at_ms < b.created_at_ms
            || (a.created_at_ms == b.created_at_ms && a.task_id < b.task_id);
        assert!(
            ordered,
            "{}: 违反 (created_at_ms asc, task_id asc) 全序: \
             ({}, {}) 出现在 ({}, {}) 之前",
            ctx, a.task_id, a.created_at_ms, b.task_id, b.created_at_ms
        );
    }
}

#[test]
fn test_team_task_created_at_ms_and_list_ordering_contract() {
    // ---------- 1. created_at_ms 非零且随时间单调不减 ----------
    let mut created: Vec<TeamTaskItem> = Vec::new();
    for idx in 0..3 {
        let item = create(&format!("order-contract-seq-{}", idx));
        assert_ne!(
            item.created_at_ms, 0,
            "team_task_create 必须写入非零 created_at_ms（clock 退化为 0 即为违约）"
        );
        created.push(item);
        // 拉大时间差，保证三个任务落在不同的毫秒桶内，使「次序」可被独立核对
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    for pair in created.windows(2) {
        assert!(
            pair[0].created_at_ms <= pair[1].created_at_ms,
            "created_at_ms 序列必须非递减: {} -> {}",
            pair[0].created_at_ms,
            pair[1].created_at_ms
        );
    }
    // 非递减很容易被「恒定值」伪实现满足，因此要求至少出现 2 个不同时间戳
    let distinct: HashSet<u64> = created.iter().map(|t| t.created_at_ms).collect();
    assert!(
        distinct.len() >= 2,
        "created_at_ms 必须随真实时间推进而变化，实际取值集合: {:?}",
        distinct
    );

    // ---------- 2. 同毫秒密集创建，用于触发/覆盖次级键分支 ----------
    let burst: Vec<TeamTaskItem> = (0..64)
        .map(|idx| create(&format!("order-contract-burst-{:02}", idx)))
        .collect();
    let burst_ids: HashSet<&str> = burst.iter().map(|t| t.task_id.as_str()).collect();
    let burst_ms: HashSet<u64> = burst.iter().map(|t| t.created_at_ms).collect();
    println!(
        "[contract] burst tasks={} distinct created_at_ms={} ({:?})",
        burst.len(),
        burst_ms.len(),
        burst.iter().map(|t| t.created_at_ms).collect::<Vec<_>>()
    );

    // ---------- 3. team_task_list() 顺序严格等于 (created_at_ms, task_id) 升序 ----------
    let first = team_task_list().expect("team_task_list must succeed");
    assert!(
        first.tasks.len() >= created.len() + burst.len(),
        "list 必须包含本轮创建的全部任务: len={} expected>= {}",
        first.tasks.len(),
        created.len() + burst.len()
    );

    let listed_ids: HashSet<&str> = first.tasks.iter().map(|t| t.task_id.as_str()).collect();
    for id in burst_ids.iter() {
        assert!(listed_ids.contains(id), "list 必须包含 burst 任务 {}", id);
    }
    for task in created.iter() {
        assert!(
            listed_ids.contains(task.task_id.as_str()),
            "list 必须包含顺序探针任务 {}",
            task.task_id
        );
    }

    // 3a. 与「独立计算的期望顺序」逐元素严格相等
    let mut expected: Vec<&TeamTaskItem> = first.tasks.iter().collect();
    expected.sort_by(|a, b| {
        a.created_at_ms
            .cmp(&b.created_at_ms)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    let actual: Vec<&str> = first.tasks.iter().map(|t| t.task_id.as_str()).collect();
    let expect: Vec<&str> = expected.iter().map(|t| t.task_id.as_str()).collect();
    assert_eq!(
        actual, expect,
        "team_task_list() 返回顺序必须严格等于 created_at_ms 升序（同毫秒 task_id 升序）"
    );

    // 3b. 全序性质逐对断言（含同毫秒次级键）
    assert_sorted_by_created_then_id(&first.tasks, "team_task_list()");
    // 3c. 次级键条件断言：每个同毫秒分组内部 task_id 严格升序
    let mut cursor = 0usize;
    let mut tie_groups = 0usize;
    while cursor < first.tasks.len() {
        let ms = first.tasks[cursor].created_at_ms;
        let mut end = cursor + 1;
        while end < first.tasks.len() && first.tasks[end].created_at_ms == ms {
            end += 1;
        }
        if end - cursor > 1 {
            tie_groups += 1;
            for pair in first.tasks[cursor..end].windows(2) {
                assert!(
                    pair[0].task_id < pair[1].task_id,
                    "同毫秒({}ms)分组内次级键必须 task_id 升序: {} !< {}",
                    ms,
                    pair[0].task_id,
                    pair[1].task_id
                );
            }
        }
        cursor = end;
    }
    println!("[contract] same-millisecond groups observed = {}", tie_groups);

    // 3d. 本轮 3 个顺序探针必须按创建先后出现在 list 中
    let mut last_pos: Option<usize> = None;
    for task in created.iter() {
        let pos = first
            .tasks
            .iter()
            .position(|t| t.task_id == task.task_id)
            .expect("probe task must be listed");
        if let Some(prev) = last_pos {
            assert!(
                pos > prev,
                "先创建的任务必须排在后面创建的任务之前: {} (pos {}) 应先于 pos {}",
                task.task_id,
                prev,
                pos
            );
        }
        last_pos = Some(pos);
    }

    // ---------- 4. 确定性：不是 HashMap 随机序 ----------
    let snapshot = |list: &[TeamTaskItem]| -> Vec<(String, u64)> {
        list.iter()
            .map(|t| (t.task_id.clone(), t.created_at_ms))
            .collect()
    };
    let baseline = snapshot(&first.tasks);
    for round in 0..20 {
        let again = team_task_list().expect("team_task_list must succeed");
        assert_eq!(
            snapshot(&again.tasks),
            baseline,
            "第 {} 次调用返回顺序与首次不一致（存在非确定性排序）",
            round + 1
        );
        assert_sorted_by_created_then_id(&again.tasks, &format!("round-{}", round + 1));
    }
}
