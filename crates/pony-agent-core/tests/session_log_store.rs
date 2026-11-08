//! ADR 0099 阶段 1-A 验收：`SessionLogStore`（每会话一份 append-only 日志文件）。
//!
//! 覆盖门禁要求的六类用例，全部**真断言**：
//! 1. append/read 往返，`seq` 严格连续；2. 撕裂尾行注入（含 30 组截断长度）不 panic 且丢弃；
//! 3. header 四态各一例；4. 8 个会话并发写零交叉污染；5. 同会话第二个 `open_write` 被拒；
//! 6. 目录逃逸防护（`../`、绝对路径、`..` 等被拒且不落盘）。
//!
//! 每个用例用 `std::env::temp_dir()` 下的唯一目录，`Drop` 清理；**绝不触碰**
//! `~/.local/share/PonyAgent/`。

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use pony_agent_core::agent::session::{
    HeaderClass, LogStoreError, NewSessionLogEvent, SessionLogMeta, SessionLogStore,
    SESSION_LOG_FILE_NAME, SESSION_LOG_FORMAT_VERSION,
};

// ---------------------------------------------------------------------------
// 夹具：唯一临时根目录 + Drop 清理
// ---------------------------------------------------------------------------

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let unique = format!(
            "pony-agent-logstore-{label}-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("创建临时根目录");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn store(&self) -> SessionLogStore {
        SessionLogStore::new(&self.path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn session(id: &str, cwd: &str) -> SessionLogMeta {
    SessionLogMeta::new(id, cwd)
}

fn seqs_of(events: &[pony_agent_core::agent::session::SessionLogEvent]) -> Vec<u64> {
    events.iter().map(|event| event.seq).collect()
}

fn read_all(store: &SessionLogStore, id: &str) -> Vec<u64> {
    let mut reader = store.open_read(id, None).expect("open_read");
    let events = reader.collect_events().expect("collect_events");
    seqs_of(&events)
}

// ---------------------------------------------------------------------------
// 1. append / read 往返，seq 严格连续
// ---------------------------------------------------------------------------

#[test]
fn append_and_read_roundtrip_keeps_seq_contiguous() {
    let tmp = TempRoot::new("roundtrip");
    let store = tmp.store();
    let id = "pa_roundtrip";
    let cwd = "/tmp/pony-roundtrip-project";
    let meta = session(id, cwd);

    let mut writer = store.open_write(id, &meta).expect("首次 open_write");
    assert_eq!(writer.next_seq(), 0, "新会话首个 seq 必须是 0");

    let mut assigned = Vec::new();
    for n in 0..5u64 {
        let seq = writer
            .append_one(NewSessionLogEvent::with_ts(
                "turn/start",
                1_000 + n as i64,
                serde_json::json!({ "n": n }),
            ))
            .expect("append_one");
        assert_eq!(seq, n, "seq 必须从 0 严格连续递增");
        assigned.push(seq);
    }
    let batch = writer
        .append(&[
            NewSessionLogEvent::with_ts("turn/end", 2_000, serde_json::json!({ "ok": true })),
            NewSessionLogEvent::with_ts("turn/end", 2_001, serde_json::json!({ "ok": false })),
        ])
        .expect("append 批量");
    assert_eq!(batch, vec![5, 6], "批量 append 返回连续 seq");
    assert_eq!(writer.next_seq(), 7);
    let log_path = writer.path().to_path_buf();
    assert_eq!(fs::metadata(&log_path).unwrap().len(), writer.committed_len());
    drop(writer);

    // 落盘形态：首行 header + 7 行事件，一行一条 JSON。
    let raw = fs::read_to_string(&log_path).expect("读日志文件");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 8, "header + 7 条事件");
    assert!(
        lines[0].contains(r#""type":"session""#),
        "首行必须是自描述 header，实际: {}",
        lines[0]
    );
    assert_eq!(log_path.file_name().unwrap(), SESSION_LOG_FILE_NAME);
    // 目录布局：<root>/--<normalized-cwd>--/<encoded-session-id>/session.jsonl
    let session_dir = log_path.parent().expect("会话目录");
    assert_eq!(session_dir.file_name().unwrap(), id, "会话目录名 = 转义后的 id");
    let group = session_dir.parent().expect("cwd 分组目录");
    let group_name = group.file_name().unwrap().to_string_lossy().to_string();
    assert!(group_name.starts_with("--") && group_name.ends_with("--"), "分组目录形态: {group_name}");
    assert!(
        group_name.contains("tmp-pony-roundtrip-project"),
        "分组目录必须携带可读的规范化 cwd: {group_name}"
    );
    assert_eq!(group.parent().unwrap(), tmp.path(), "分组目录直接位于 root 下");

    // 全量读回：seq 连续、type/ts/payload 保真。
    let mut reader = store.open_read(id, None).expect("open_read");
    let events = reader.collect_events().expect("collect_events");
    assert_eq!(seqs_of(&events), assigned.iter().copied().chain([5, 6]).collect::<Vec<_>>());
    assert_eq!(events[0].kind, "turn/start");
    assert_eq!(events[0].ts, 1_000);
    assert_eq!(events[0].payload, serde_json::json!({ "n": 0 }));
    assert_eq!(events[4].payload, serde_json::json!({ "n": 4 }));
    assert_eq!(events[6].kind, "turn/end");
    assert_eq!(events[6].payload, serde_json::json!({ "ok": false }));

    // after_seq 是排他上界：只回 seq > 4。
    let tail = store
        .open_read(id, Some(4))
        .expect("open_read after_seq")
        .collect_events()
        .expect("collect tail");
    assert_eq!(seqs_of(&tail), vec![5, 6]);

    // list_sessions 从 header 还原摘要。
    let listed = store.list_sessions().expect("list_sessions");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session_id, id);
    assert_eq!(listed[0].cwd, cwd);
    assert_eq!(listed[0].created_at, meta.created_at);

    // 重新打开：seq 接着已提交前缀，不重号。
    let mut reopened = store.open_write(id, &meta).expect("重开 open_write");
    assert_eq!(reopened.next_seq(), 7);
    assert_eq!(
        reopened
            .append_one(NewSessionLogEvent::with_ts("after/reopen", 3_000, serde_json::Value::Null))
            .expect("append after reopen"),
        7
    );
    drop(reopened);
    assert_eq!(read_all(&store, id), vec![0, 1, 2, 3, 4, 5, 6, 7]);
}

// ---------------------------------------------------------------------------
// 2. 撕裂尾行注入：丢弃尾行、保留前缀、不 panic
// ---------------------------------------------------------------------------

#[test]
fn torn_tail_is_discarded_and_committed_prefix_survives() {
    let tmp = TempRoot::new("torn");
    let store = tmp.store();
    let meta = session("pa_torn", "/tmp/pony-torn-project");
    let id = "pa_torn";

    let mut writer = store.open_write(id, &meta).expect("open_write");
    for n in 0..3u64 {
        writer
            .append_one(NewSessionLogEvent::with_ts(
                "turn/start",
                n as i64,
                serde_json::json!({ "n": n }),
            ))
            .expect("append");
    }
    let log_path = writer.path().to_path_buf();
    let committed = writer.committed_len();
    drop(writer);

    // (A) 无换行结束符的半行 JSON：崩溃在 write 中途的形态。
    let mut handle = fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .expect("append 注入");
    handle.write_all(br#"{"seq":3,"type":"x""#).expect("写半行");
    handle.flush().expect("flush");
    drop(handle);
    assert_eq!(read_all(&store, id), vec![0, 1, 2], "半行尾行必须被丢弃");

    // (B) 有换行但 JSON 损坏：同样是不可提交后缀。
    let mut handle = fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .expect("append 注入");
    handle.write_all(b"not-json-at-all\n").expect("写坏行");
    handle.flush().expect("flush");
    drop(handle);
    assert_eq!(read_all(&store, id), vec![0, 1, 2], "坏 JSON 尾行必须被丢弃");

    // (C) 30 组不同截断长度：每一组都必须只返回前 3 条且不 panic。
    let victim = br#"{"seq":3,"type":"turn/start","ts":9,"payload":{"n":3}}"#;
    for cut in 1..=30usize {
        let id = format!("pa_torn_cut{cut}");
        let meta = session(&id, "/tmp/pony-torn-project");
        let mut writer = store.open_write(&id, &meta).expect("open_write cut");
        for n in 0..3u64 {
            writer
                .append_one(NewSessionLogEvent::with_ts(
                    "turn/start",
                    n as i64,
                    serde_json::json!({ "n": n }),
                ))
                .expect("append cut");
        }
        let path = writer.path().to_path_buf();
        drop(writer);
        let mut handle = fs::OpenOptions::new().append(true).open(&path).expect("open cut");
        handle.write_all(&victim[..cut]).expect("写截断行");
        drop(handle);
        assert_eq!(
            read_all(&store, &id),
            vec![0, 1, 2],
            "截断长度 {cut} 的尾行必须被丢弃"
        );
    }

    // (D) open_write 恢复：截断撕裂尾行，seq 接着已提交前缀继续（不重号、不越号）。
    let mut recovered = store.open_write(id, &meta).expect("恢复 open_write");
    assert_eq!(recovered.next_seq(), 3, "恢复后 seq 接已提交前缀");
    assert_eq!(
        recovered.committed_len(),
        committed,
        "恢复必须把文件截断回已提交字节长度"
    );
    assert_eq!(fs::metadata(&log_path).unwrap().len(), committed);
    let seq = recovered
        .append_one(NewSessionLogEvent::with_ts(
            "turn/end",
            4_000,
            serde_json::json!({ "ok": true }),
        ))
        .expect("恢复后 append");
    assert_eq!(seq, 3);
    drop(recovered);
    assert_eq!(read_all(&store, id), vec![0, 1, 2, 3]);
}

// ---------------------------------------------------------------------------
// 3. header 四态各一例
// ---------------------------------------------------------------------------

/// 直接落一份手工 header（模拟别的版本 / 损坏文件写出的日志）。
fn seed_header(store: &SessionLogStore, id: &str, cwd: &str, header: &str) -> PathBuf {
    let path = store.log_path(id, cwd).expect("log_path");
    fs::create_dir_all(path.parent().unwrap()).expect("建会话目录");
    fs::write(&path, format!("{header}\n")).expect("写 header");
    path
}

#[test]
fn header_classification_covers_current_migration_unsupported_malformed() {
    let tmp = TempRoot::new("header");
    let store = tmp.store();
    let cwd = "/tmp/pony-header-project";

    // Current：版本 == 当前实现。
    let current_id = "pa_header_current";
    seed_header(
        &store,
        current_id,
        cwd,
        &format!(
            r#"{{"type":"session","version":{SESSION_LOG_FORMAT_VERSION},"id":"{current_id}","cwd":"{cwd}","createdAt":1234}}"#
        ),
    );
    match store.read_session_header(current_id).expect("read header") {
        HeaderClass::Current(header) => {
            assert_eq!(header.version, SESSION_LOG_FORMAT_VERSION);
            assert_eq!(header.session_id, current_id);
            assert_eq!(header.cwd, cwd);
            assert_eq!(header.created_at, 1234);
        }
        other => panic!("期望 Current，实际 {}", other.label()),
    }
    assert!(store
        .read_session_header(current_id)
        .unwrap()
        .is_current());
    // current header 的会话可正常打开读（空事件流）。
    assert_eq!(read_all(&store, current_id), Vec::<u64>::new());

    // MigrationRequired：版本低于当前实现。
    let old_id = "pa_header_old";
    seed_header(
        &store,
        old_id,
        cwd,
        &format!(
            r#"{{"type":"session","version":0,"id":"{old_id}","cwd":"{cwd}","createdAt":1}}"#
        ),
    );
    match store.read_session_header(old_id).expect("read old header") {
        HeaderClass::MigrationRequired(header) => {
            assert_eq!(header.version, 0);
            assert_eq!(header.session_id, old_id);
        }
        other => panic!("期望 MigrationRequired，实际 {}", other.label()),
    }
    match store.open_read(old_id, None) {
        Err(LogStoreError::HeaderMigrationRequired { found, current }) => {
            assert_eq!(found, 0);
            assert_eq!(current, SESSION_LOG_FORMAT_VERSION);
        }
        other => panic!("旧版本必须拒绝读取，实际 {other:?}"),
    }

    // Unsupported：版本高于当前实现。
    let future_id = "pa_header_future";
    seed_header(
        &store,
        future_id,
        cwd,
        &format!(
            r#"{{"type":"session","version":{},"id":"{future_id}","cwd":"{cwd}","createdAt":2}}"#,
            SESSION_LOG_FORMAT_VERSION + 1
        ),
    );
    match store
        .read_session_header(future_id)
        .expect("read future header")
    {
        HeaderClass::Unsupported(header) => {
            assert_eq!(header.version, SESSION_LOG_FORMAT_VERSION + 1);
            assert_eq!(header.session_id, future_id);
        }
        other => panic!("期望 Unsupported，实际 {}", other.label()),
    }
    match store.open_read(future_id, None) {
        Err(LogStoreError::HeaderUnsupported { found, .. }) => {
            assert_eq!(found, SESSION_LOG_FORMAT_VERSION + 1)
        }
        other => panic!("更高版本必须拒绝读取，实际 {other:?}"),
    }

    // Malformed：首行 JSON 不完整。
    let broken_id = "pa_header_broken";
    let broken_path = seed_header(&store, broken_id, cwd, r#"{"type":"session","vers"#);
    match store.read_session_header(broken_id).expect("read broken") {
        HeaderClass::Malformed { reason } => {
            assert!(!reason.is_empty(), "分类必须带原因");
        }
        other => panic!("期望 Malformed，实际 {}", other.label()),
    }
    // 独立函数 read_header 与存储方法给出同一分类。
    let direct = pony_agent_core::agent::session::read_header(&broken_path).expect("read_header");
    assert_eq!(direct.label(), "malformed");
    // 损坏 header 不进入会话列表（列表只报 current）。
    let listed = store.list_sessions().expect("list_sessions");
    assert_eq!(
        listed.iter().map(|m| m.session_id.as_str()).collect::<Vec<_>>(),
        vec![current_id],
        "list_sessions 只返回 current header 的会话"
    );
}

// ---------------------------------------------------------------------------
// 4. 8 个不同会话并发写：全部成功、各自内容正确、零交叉污染
// ---------------------------------------------------------------------------

#[test]
fn eight_concurrent_sessions_write_without_cross_contamination() {
    const WORKERS: u64 = 8;
    const EVENTS: u64 = 20;
    let tmp = TempRoot::new("concurrent");
    let root = tmp.path().to_path_buf();
    let cwd = "/tmp/pony-concurrent-project";

    let mut handles = Vec::new();
    for worker in 0..WORKERS {
        let root = root.clone();
        let cwd = cwd.to_string();
        handles.push(std::thread::spawn(move || {
            let store = SessionLogStore::new(&root);
            let id = format!("pa_worker{worker}");
            let meta = SessionLogMeta::new(&id, &cwd);
            let mut writer = store.open_write(&id, &meta).expect("并发 open_write");
            let mut assigned = Vec::new();
            for n in 0..EVENTS {
                let seq = writer
                    .append_one(NewSessionLogEvent::with_ts(
                        "turn/start",
                        n as i64,
                        serde_json::json!({ "worker": worker, "n": n }),
                    ))
                    .expect("并发 append");
                assigned.push(seq);
            }
            assert_eq!(assigned, (0..EVENTS).collect::<Vec<u64>>(), "每个会话 seq 独立从 0 连续");
            let path = writer.path().to_path_buf();
            drop(writer);

            // 文件结构：header + EVENTS 行，一行不多一行不少（无交错写）。
            let raw = fs::read_to_string(&path).expect("读并发日志");
            assert_eq!(raw.lines().count() as u64, EVENTS + 1);

            // 内容：只含本 worker 自己的事件，seq 与 n 对齐。
            let mut reader = store.open_read(&id, None).expect("并发 open_read");
            let events = reader.collect_events().expect("并发 collect");
            assert_eq!(events.len() as u64, EVENTS);
            for (index, event) in events.iter().enumerate() {
                assert_eq!(event.seq, index as u64);
                assert_eq!(event.payload["worker"], serde_json::json!(worker));
                assert_eq!(event.payload["n"], serde_json::json!(index as u64));
            }
            id
        }));
    }

    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.join().expect("并发线程不得 panic"));
    }
    assert_eq!(ids.len() as u64, WORKERS);

    let store = tmp.store();
    let listed = store.list_sessions().expect("list_sessions");
    assert_eq!(listed.len() as u64, WORKERS, "8 个会话都必须可列举");
    let mut distinct = BTreeSet::new();
    for meta in &listed {
        assert!(distinct.insert(meta.session_id.clone()), "会话 id 不得重复");
        assert_eq!(meta.cwd, cwd);
    }
    assert_eq!(distinct.len() as u64, WORKERS);
    for id in &ids {
        assert_eq!(read_all(&store, id).len() as u64, EVENTS);
        let header = store.read_session_header(id).expect("read header");
        assert!(header.is_current());
        assert_eq!(header.header().unwrap().session_id, *id);
    }
}

// ---------------------------------------------------------------------------
// 5. 同会话重复 open_write：第二个被明确拒绝（文件级独占）
// ---------------------------------------------------------------------------

#[test]
fn second_open_write_for_same_session_is_rejected() {
    let tmp = TempRoot::new("exclusive");
    let store = tmp.store();
    let id = "pa_exclusive";
    let meta = session(id, "/tmp/pony-exclusive-project");

    let mut first = store.open_write(id, &meta).expect("第一个 writer");
    first
        .append_one(NewSessionLogEvent::with_ts(
            "turn/start",
            1,
            serde_json::json!({ "owner": "first" }),
        ))
        .expect("first append");
    let expected_path = first.path().to_path_buf();

    match store.open_write(id, &meta) {
        Err(LogStoreError::Locked { path }) => {
            assert_eq!(path, expected_path, "拒绝必须指向被锁的那份日志文件")
        }
        other => panic!("第二个 open_write 必须被拒绝为 Locked，实际 {other:?}"),
    }

    // 拒绝不得静默写：文件内容仍是 header + 1 行。
    let raw = fs::read_to_string(&expected_path).expect("读被锁文件");
    assert_eq!(raw.lines().count(), 2, "被拒的 writer 不得写入任何字节");

    // 持锁期间 delete 也必须被拒（不能从活跃 writer 底下删目录）。
    match store.delete(id) {
        Err(LogStoreError::Locked { .. }) => {}
        other => panic!("活跃 writer 下的 delete 必须被拒绝，实际 {other:?}"),
    }
    assert!(expected_path.exists(), "被拒的 delete 不得删除文件");

    // 锁随句柄关闭释放：drop 后可再次独占打开，seq 接着前缀。
    drop(first);
    let mut second = store.open_write(id, &meta).expect("锁释放后可重开");
    assert_eq!(second.next_seq(), 1);
    second
        .append_one(NewSessionLogEvent::with_ts(
            "turn/end",
            2,
            serde_json::json!({ "owner": "second" }),
        ))
        .expect("second append");
    drop(second);
    assert_eq!(read_all(&store, id), vec![0, 1]);
}

// ---------------------------------------------------------------------------
// 6. 目录逃逸防护
// ---------------------------------------------------------------------------

#[test]
fn traversal_like_session_ids_are_rejected_without_touching_disk() {
    let tmp = TempRoot::new("traversal");
    let store = tmp.store();
    let unsafe_ids = [
        "../evil",
        "../../etc/passwd",
        "/abs/path",
        "..",
        ".",
        "a/b",
        "a\\b",
        "c:evil",
        "",
        "with\nnewline",
    ];
    for bad in unsafe_ids {
        let meta = session(bad, "/tmp/pony-traversal-project");
        match store.open_write(bad, &meta) {
            Err(LogStoreError::InvalidSessionId { .. }) => {}
            other => panic!("`{bad}` 必须被拒绝为 InvalidSessionId，实际 {other:?}"),
        }
        match store.open_read(bad, None) {
            Err(LogStoreError::InvalidSessionId { .. }) => {}
            other => panic!("`{bad}` 的 open_read 必须被拒绝，实际 {other:?}"),
        }
        match store.delete(bad) {
            Err(LogStoreError::InvalidSessionId { .. }) => {}
            other => panic!("`{bad}` 的 delete 必须被拒绝，实际 {other:?}"),
        }
    }
    // 任何逃逸尝试都不得在根目录下留下条目。
    let entries = fs::read_dir(tmp.path()).expect("读临时根").count();
    assert_eq!(entries, 0, "被拒的 id 不得创建任何文件或目录");

    // 单射转义：仅大小写不同的 id 落到不同目录（大小写不敏感文件系统也不会撞车）。
    let upper = store
        .log_path("pa_AB", "/tmp/pony-traversal-project")
        .expect("encode upper");
    let lower = store
        .log_path("pa_ab", "/tmp/pony-traversal-project")
        .expect("encode lower");
    assert_ne!(upper, lower, "大小写不同的 id 必须映射到不同路径段");
    // 合法 id 仍可正常用（拒绝面没有误伤）。
    let meta = session("pa_legal", "/tmp/pony-traversal-project");
    store.open_write("pa_legal", &meta).expect("合法 id 可写");
    assert_eq!(read_all(&store, "pa_legal"), Vec::<u64>::new());
}
