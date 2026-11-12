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
    assert_roundtrips_closed, build_request, closed, interrupted_history, native_provider,
    request_body, snapshot_of, DELIVERED_TEXT, INTERRUPTED_USER, NEXT_USER,
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

struct RecoveryArtifacts {
    /// 完整 `ProviderRequest` JSON（跨线程确定性判据）。
    body: String,
    /// native 通道 JSON（重复注入判据的**唯一**合法计数面）。
    native_body: String,
    /// input carry 通道 JSON（可选更严判据：同一条往返在 carry 通道内也只能出现一次）。
    carry_body: String,
    closure: Vec<serde_json::Value>,
    closed_steps: Vec<ClosedToolStep>,
}

fn recovery_body(round: usize, draw: u64) -> RecoveryArtifacts {
    let (closed_steps, pending) = steps_for(round, draw);
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("AT2：已闭合步骤非空 ⇒ 恢复路径必须产出闭合片段");
    let session = snapshot_of(
        &format!("at2-race-{round}"),
        interrupted_history(&assistant_text),
        closure.clone(),
    );
    let provider = native_provider();
    let request = build_request(&provider, &session, NEXT_USER);
    RecoveryArtifacts {
        body: serde_json::to_string(&request).expect("AT2：请求必须可序列化"),
        native_body: serde_json::to_string(&request.native_messages)
            .expect("AT2：native_messages 必须可序列化"),
        carry_body: serde_json::to_string(&request.input).expect("AT2：input carry 必须可序列化"),
        closure,
        closed_steps,
    }
}

#[test]
fn at2_concurrent_repeated_recovery_is_deterministic_and_never_duplicates() {
    // Arrange：单线程参考序列（同一固定种子）
    let mut reference_rng = Lcg::new(SEED);
    let reference: Vec<RecoveryArtifacts> = (0..ITERATIONS)
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
                    let artifacts = recovery_body(round, rng.next());
                    assert_roundtrips_closed(&artifacts.closure, &format!("AT2-thread{thread_index}"));
                    bodies.push(artifacts.body);
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
                body, &reference[round].body,
                "AT2：线程 {thread_index} 第 {round} 轮恢复请求必须与参考序列逐字一致"
            );
        }
    }

    // Assert 2：结果哨兵串在 **native 通道内**恰好出现 1 次（不重复注入 transcript）。
    //
    // 契约演进（见 `.dev-team/red-phase-interrupted-turn-resume.json` 的 `amendments`）：
    // 原断言对**整个 `ProviderRequest` JSON** 计数，但 native 工具往返在请求里必然出现 3 次
    // （`native_messages` + `input` carry + `observation.semi_stable_context_text`），
    // 全局 ==1 数学不可能（下界 2）；且 observation 的 carry 渲染被 1518 条 `--lib` 基线钉住
    // （`agent/context.rs:2047-2051`），不得为过门禁改基线。故计数面收敛为 native 通道，
    // 与 C6 同口径；另加"carry 通道内 ≤1 次"作为更严的旁证。
    for (round, artifacts) in reference.iter().enumerate() {
        for step in &artifacts.closed_steps {
            let native_occurrences = artifacts.native_body.matches(&step.result_summary).count();
            assert_eq!(
                native_occurrences, 1,
                "AT2：第 {round} 轮结果哨兵 {:?} 在 native 通道内必须恰好出现 1 次（实际 {native_occurrences} 次），native = {}",
                step.result_summary, artifacts.native_body
            );
            let carry_occurrences = artifacts.carry_body.matches(&step.result_summary).count();
            assert!(
                carry_occurrences <= 1,
                "AT2：第 {round} 轮结果哨兵 {:?} 在 carry 通道内不得重复注入（实际 {carry_occurrences} 次），carry = {}",
                step.result_summary, artifacts.carry_body
            );
        }
        assert!(
            !artifacts.closure.is_empty(),
            "AT2：第 {round} 轮闭合片段不得为空"
        );
    }
}
