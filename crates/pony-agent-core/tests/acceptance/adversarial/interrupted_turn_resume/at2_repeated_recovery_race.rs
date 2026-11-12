//! AT2：重复恢复竞态（确定性：固定种子 LCG + 固定线程数/迭代数 + Barrier，零 sleep）。
//!
//! 判据：
//! 1. 同一中断状态被并发多次"恢复续跑"，产出的请求必须逐字确定（不得因调度顺序漂移）；
//! 2. 结果哨兵串在请求体里恰好出现一次（不重复注入 transcript）；
//! 3. 跨线程无共享可变状态污染（每个线程序列 == 单线程参考序列）。

use std::sync::{Arc, Barrier};
use std::thread;

use pony_agent_core::agent::session::interrupted_turn::{
    interrupted_turn_assistant_text, interrupted_turn_transcript_closure, ClosedToolStep, PendingToolCall,
};

use crate::{
    assert_roundtrips_closed, closed, interrupted_history, native_provider, request_body, snapshot_of,
    DELIVERED_TEXT, INTERRUPTED_USER, NEXT_USER,
};

const THREADS: usize = 4;
const ITERATIONS: usize = 8;
const SEED: u64 = 0x00C0_FFEE_1234_5678;

/// 固定种子 LCG（Numerical Recipes 常数）：序列可复现，不引入 rand 依赖。
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
}

fn steps_for(round: usize, draw: u64) -> (Vec<ClosedToolStep>, Vec<PendingToolCall>) {
    let closed_count = 1 + (draw % 3) as usize; // 1..=3
    let mut closed_steps = Vec::new();
    for index in 0..closed_count {
        closed_steps.push(closed(
            &format!("Tool_{round}_{index}"),
            &format!("RACE_RESULT_{round}_{index}_UNIQUE"),
        ));
    }
    let pending = vec![PendingToolCall {
        name: format!("Pending_{round}"),
    }];
    (closed_steps, pending)
}

fn recovery_body(round: usize, draw: u64) -> (String, Vec<serde_json::Value>, Vec<ClosedToolStep>) {
    let (closed_steps, pending) = steps_for(round, draw);
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("AT2：已闭合步骤非空 ⇒ 恢复路径必须产出闭合片段");
    let session = snapshot_of(
        &format!("at2-race-{round}"),
        interrupted_history(&assistant_text),
        closure.clone(),
    );
    let body = request_body(&native_provider(), &session, NEXT_USER);
    (body, closure, closed_steps)
}

#[test]
fn at2_concurrent_repeated_recovery_is_deterministic_and_never_duplicates() {
    // Arrange：单线程参考序列（同一固定种子）
    let mut reference_rng = Lcg::new(SEED);
    let reference: Vec<(String, Vec<serde_json::Value>, Vec<ClosedToolStep>)> = (0..ITERATIONS)
        .map(|round| recovery_body(round, reference_rng.next()))
        .collect();
    assert_eq!(
        reference.len(),
        ITERATIONS,
        "AT2 夹具自检：参考序列长度必须等于固定迭代数"
    );

    // Act：4 线程在同一个 Barrier 上同时起跑，各自跑同一固定种子序列
    let barrier = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|thread_index| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let mut rng = Lcg::new(SEED);
                let mut bodies = Vec::with_capacity(ITERATIONS);
                barrier.wait();
                for round in 0..ITERATIONS {
                    let (body, closure, _steps) = recovery_body(round, rng.next());
                    assert_roundtrips_closed(&closure, &format!("AT2-thread{thread_index}"));
                    bodies.push(body);
                }
                bodies
            })
        })
        .collect();
    let observed: Vec<Vec<String>> = handles
        .into_iter()
        .map(|handle| handle.join().expect("AT2：恢复线程不得 panic（SUT 栈 panic 即红相判据）"))
        .collect();

    // Assert 1：跨线程逐轮逐字确定（无共享状态污染）
    for (thread_index, bodies) in observed.iter().enumerate() {
        assert_eq!(
            bodies.len(),
            ITERATIONS,
            "AT2：线程 {thread_index} 的迭代数必须固定为 {ITERATIONS}"
        );
        for (round, body) in bodies.iter().enumerate() {
            assert_eq!(
                body, &reference[round].0,
                "AT2：线程 {thread_index} 第 {round} 轮恢复请求必须与参考序列逐字一致"
            );
        }
    }

    // Assert 2：每轮结果哨兵串在请求体里恰好出现一次（不重复注入 transcript）
    for (round, (body, closure, closed_steps)) in reference.iter().enumerate() {
        for step in closed_steps {
            let occurrences = body.matches(&step.result_summary).count();
            assert_eq!(
                occurrences, 1,
                "AT2：第 {round} 轮结果哨兵 {:?} 必须恰好出现 1 次（实际 {occurrences} 次）",
                step.result_summary
            );
        }
        assert!(
            !closure.is_empty(),
            "AT2：第 {round} 轮闭合片段不得为空"
        );
    }
}
