//! B148 性能探针：真实会话库分相位计时（诊断资产，不参与生产构建）。
//!
//! 用法（不要指向真实库！先 `cp` 到副本）：
//!   PONY_AGENT_DATA_DIR=/tmp/perf-copy-root \
//!   cargo run -p pony-agent-core --example perf_probe -- \
//!     /tmp/perf-copy-root/PonyAgent/sessions.db pa_01a1ba70-a314-7712-adf2-4bf675a96f99 3
//!
//! 输出每个相位每轮的毫秒耗时（原始数据，取中位由报告侧完成），以及 VmHWM。
//!
//! 只读相位全部走 `SqliteSessionBackend::new_read_only`，库文件零写入；
//! 唯一会写的是最后一组 `SessionStore::new()`（生产端到端路径，包含启动自愈回写），
//! 因此调用前必须用 `PONY_AGENT_DATA_DIR` 把默认库重定向到副本。
//!
//! 备注（API 可达性）：`PersistedStore` 的字段与 `SessionStore::session_count` 均非 pub，
//! 故各相位只能报告「是否装载成功」与 `SessionState` 的公开字段，不能报告行数。

use std::path::PathBuf;
use std::time::Instant;

use pony_agent_core::agent::session::{SeparateTraceTableMode, SessionBackend, SessionStore};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;


/// 进程 CPU 时间（utime+stime，单位：clock ticks，Linux 用户态 HZ=100）。
fn cpu_ticks() -> u64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let Some(rest) = s.rsplit_once(')').map(|(_, r)| r) else {
        return 0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // fields[0] 是 stat 第 3 字段 state，故 utime(14)/stime(15) 对应下标 11/12。
    let utime = fields.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let stime = fields.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    utime + stime
}

fn cpu_ms_delta(start: u64) -> f64 {
    (cpu_ticks().saturating_sub(start)) as f64 * 10.0
}

fn vm_status() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut out = Vec::new();
    for line in s.lines() {
        if line.starts_with("VmHWM:") || line.starts_with("VmRSS:") || line.starts_with("VmPeak:") {
            out.push(line.trim().to_string());
        }
    }
    out.join(" | ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let db_path = PathBuf::from(
        args.get(1)
            .cloned()
            .unwrap_or_else(|| "/tmp/perf-copy-root/PonyAgent/sessions.db".into()),
    );
    let session_id = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "pa_01a1ba70-a314-7712-adf2-4bf675a96f99".into());
    let rounds: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(3);

    println!("== perf_probe ==");
    println!("db_path={}", db_path.display());
    println!("session_id={session_id}");
    println!("rounds={rounds}");
    println!(
        "PONY_AGENT_DATA_DIR={:?}",
        std::env::var("PONY_AGENT_DATA_DIR").ok()
    );
    if let Ok(meta) = std::fs::metadata(&db_path) {
        println!("db_bytes={}", meta.len());
    }
    println!("baseline {}", vm_status());

    // 相位 A：只读 backend 构造（open + pragma，不装载数据）
    for r in 0..rounds {
        let t = Instant::now();
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let read_only = be.storage_read_only();
        println!(
            "phase=A_construct_read_only round={} ms={:.3} storage_read_only={read_only}",
            r + 1,
            t.elapsed().as_secs_f64() * 1000.0
        );
    }

    // 相位 B：load_index_store（仅索引视图）
    for r in 0..rounds {
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let t = Instant::now();
        let loaded = be.load_index_store();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=B_load_index_store round={} ms={:.3} has_store={}",
            r + 1,
            ms,
            loaded.is_some()
        );
    }

    // 相位 C：load_store（trait 全量装载）
    for r in 0..rounds {
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let cpu0 = cpu_ticks();
        let t = Instant::now();
        let loaded = be.load_store();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=C_load_store round={} ms={:.3} cpu_ms={:.0} has_store={}",
            r + 1,
            ms,
            cpu_ms_delta(cpu0),
            loaded.is_some()
        );
    }

    // 相位 D：单会话定向装载（最大会话）
    for r in 0..rounds {
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let cpu0 = cpu_ticks();
        let t = Instant::now();
        let loaded = be.load_normalized_session(&session_id);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let (history, turns) = match &loaded {
            Some(state) => (state.history.len(), state.turn_count),
            None => (0, 0),
        };
        println!(
            "phase=D_load_normalized_session round={} ms={:.3} cpu_ms={:.0} history={history} turn_count={turns}",
            r + 1,
            ms,
            cpu_ms_delta(cpu0)
        );
    }

    // 相位 E：SessionStore::with_backend（只读 backend）——启动装载 + 自愈判定的只读端到端
    for r in 0..rounds {
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let cpu0 = cpu_ticks();
        let t = Instant::now();
        let _store = SessionStore::with_backend(Box::new(be));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=E_SessionStore_with_backend_readonly round={} ms={:.3} cpu_ms={:.0}",
            r + 1,
            ms,
            cpu_ms_delta(cpu0)
        );
    }

    // 相位 H/I：用公开 trait 方法拆分 load_store 的成本（argv[4] = 会话 id 清单文件）
    if let Some(list_path) = args.get(4) {
        let ids: Vec<String> = std::fs::read_to_string(list_path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        println!("phase=H_input list={} ids={}", list_path, ids.len());
        let be = SqliteSessionBackend::new_read_only(db_path.clone());
        let cpu0 = cpu_ticks();
        let t = Instant::now();
        let mut trace_calls = 0usize;
        for sid in &ids {
            let _ = be.load_session_traces(sid);
            trace_calls += 1;
        }
        println!(
            "phase=H_load_session_traces_all ms={:.3} cpu_ms={:.0} calls={trace_calls}",
            t.elapsed().as_secs_f64() * 1000.0,
            cpu_ms_delta(cpu0)
        );

        let cpu0 = cpu_ticks();
        let t = Instant::now();
        let mut events = 0usize;
        for sid in &ids {
            events += be.load_turn_events(sid, None).len();
        }
        println!(
            "phase=I_load_turn_events_all ms={:.3} cpu_ms={:.0} sessions={} events={events}",
            t.elapsed().as_secs_f64() * 1000.0,
            cpu_ms_delta(cpu0),
            ids.len()
        );
    }

    // 相位 F：生产端到端 SessionStore::new()（写模式；仅当 PONY_AGENT_DATA_DIR 已重定向到副本）
    let redirected = std::env::var_os("PONY_AGENT_DATA_DIR").is_some();
    if !redirected {
        println!(
            "phase=F_SessionStore_new SKIPPED：未设置 PONY_AGENT_DATA_DIR，为避免写真实库不执行"
        );
    } else {
        for r in 0..rounds {
            let cpu0 = cpu_ticks();
        let t = Instant::now();
            let store = SessionStore::new();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "phase=F_SessionStore_new round={} ms={:.3} cpu_ms={:.0} ask_store={}",
                r + 1,
                ms,
                cpu_ms_delta(cpu0),
                store.ask_store().is_some()
            );
        }
    }

    // 相位 G：写模式 backend 构造对照（不装载）
    for r in 0..rounds {
        let t = Instant::now();
        let be = SqliteSessionBackend::new_with_trace_mode(
            db_path.to_path_buf(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let read_only = be.storage_read_only();
        println!(
            "phase=G_construct_write_mode round={} ms={:.3} storage_read_only={read_only}",
            r + 1,
            t.elapsed().as_secs_f64() * 1000.0
        );
    }

    report_lazy_phases(&session_id, rounds);

    report_metadata_steps(&db_path, rounds);

    println!("final {}", vm_status());
    println!("== done ==");
}

/// 相位 J/K/L/M：稀疏装载（Ready 成本）、定向 hydrate、列表一致、写集合收敛。
fn report_lazy_phases(session_id: &str, rounds: usize) {
    let redirected = std::env::var_os("PONY_AGENT_DATA_DIR").is_some();
    // 相位 J/K/L/M：B148 稀疏装载与写集合收敛（写模式；仅当库已重定向到副本）
    if !redirected {
        println!("phase=J..M SKIPPED：未设置 PONY_AGENT_DATA_DIR，为避免写真实库不执行");
    } else {
        let mut sparse_ids_per_round: Vec<Vec<String>> = Vec::new();
        // J：生产入口 `SessionStore::open_lazy()` = TUI 启动「Ready」前唯一的重活。
        for r in 0..rounds {
            let cpu0 = cpu_ticks();
            let t = Instant::now();
            let store = SessionStore::open_lazy();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let list = store.list_sessions();
            let ids = list
                .iter()
                .map(|item| item.conversation_id.clone())
                .collect::<Vec<_>>();
            println!(
                "phase=J_open_lazy round={} ms={:.3} cpu_ms={:.0} sparse={} meta_sessions={} list_len={}",
                r + 1,
                ms,
                cpu_ms_delta(cpu0),
                store.is_sparse(),
                store.meta_entries().len(),
                ids.len()
            );
            println!("phase=J_after_ready {}", vm_status());
            sparse_ids_per_round.push(ids);
        }
        // K：resume 首屏成本代理 = 定向 hydrate + 只读快照。
        for r in 0..rounds {
            let mut store = SessionStore::open_lazy();
            let cpu0 = cpu_ticks();
            let t = Instant::now();
            let ok = store.hydrate_session(session_id);
            let snapshot = store.snapshot_at_readonly(Some(session_id), None, &[]);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "phase=K_hydrate_targeted round={} ms={:.3} cpu_ms={:.0} loaded={ok} history={} turn_count={}",
                r + 1,
                ms,
                cpu_ms_delta(cpu0),
                snapshot.history.len(),
                snapshot.turn_count
            );
        }
        // L：列表口径一致性（稀疏列表 id 序列 vs 全量装载列表 id 序列）。
        let full = SessionStore::new();
        let full_ids = full
            .list_sessions()
            .iter()
            .map(|item| item.conversation_id.clone())
            .collect::<Vec<_>>();
        for (index, sparse_ids) in sparse_ids_per_round.iter().enumerate() {
            let identical = *sparse_ids == full_ids;
            println!(
                "phase=L_list_id_parity round={} identical={} sparse_len={} full_len={}",
                index + 1,
                identical,
                sparse_ids.len(),
                full_ids.len()
            );
            if !identical {
                let missing = full_ids
                    .iter()
                    .filter(|id| !sparse_ids.contains(id))
                    .collect::<Vec<_>>();
                let extra = sparse_ids
                    .iter()
                    .filter(|id| !full_ids.contains(id))
                    .collect::<Vec<_>>();
                println!(
                    "phase=L_list_id_parity_diff missing={:?} extra={:?}",
                    &missing[..missing.len().min(8)],
                    &extra[..extra.len().min(8)]
                );
            }
        }
        // M：写集合收敛（稀疏提交只写已装载会话 vs 全量整库提交）。
        for r in 0..rounds {
            let mut store = SessionStore::open_lazy();
            let loaded = store.hydrate_session(session_id);
            let cpu0 = cpu_ticks();
            let t = Instant::now();
            let renamed = store
                .rename_session(session_id, &format!("perf-probe-{}", r + 1))
                .is_ok();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "phase=M_sparse_rename round={} ms={:.3} cpu_ms={:.0} loaded={loaded} renamed={renamed} write_set={}",
                r + 1,
                ms,
                cpu_ms_delta(cpu0),
                store.loaded_session_ids().len()
            );
        }
        for r in 0..rounds {
            let mut store = SessionStore::new();
            let cpu0 = cpu_ticks();
            let t = Instant::now();
            let renamed = store
                .rename_session(session_id, &format!("perf-probe-full-{}", r + 1))
                .is_ok();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "phase=M_full_rename round={} ms={:.3} cpu_ms={:.0} renamed={renamed}",
                r + 1,
                ms,
                cpu_ms_delta(cpu0)
            );
        }
    }
}

/// 相位 N：稀疏装载两个子步骤的独立成本（定位 Ready 预算归属）。
fn report_metadata_steps(db_path: &std::path::Path, rounds: usize) {
    // 相位 N：稀疏装载的两个子步骤各自成本（定位 Ready 预算归属：J 慢在哪一步）。
    for r in 0..rounds {
        let be = SqliteSessionBackend::new_with_trace_mode(
            db_path.to_path_buf(),
            SeparateTraceTableMode::WriteSeparate,
        );
        let t = Instant::now();
        let metadata = be.load_store_metadata();
        let metadata_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let index = be.load_session_meta_index();
        let index_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let schema = be.validate_event_schema();
        let schema_ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "phase=N_metadata_steps round={} metadata_ms={:.3} has_metadata={} index_ms={:.3} index_sessions={} schema_ms={:.3} schema_ok={}",
            r + 1,
            metadata_ms,
            metadata.is_some(),
            index_ms,
            index.as_ref().map(|entries| entries.len()).unwrap_or(0),
            schema_ms,
            schema.is_ok()
        );
    }
}
