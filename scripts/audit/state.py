#!/usr/bin/env python3
"""trace 审核 loop 状态机（docs/audit/loop-state.json）的唯一读写实现。

归属：ADR 0101 §状态机 / PA-169。所有阶段迁移必须经本脚本做 CAS，禁止各节点
脚本自己 open/write 状态文件（否则双跑与冻结检查都会失效）。

子命令与退出码（0=成功；1=业务拒绝；2=参数/解析错误；3=lease 冲突）：

  get        --path a.b.c                 读取字段（JSON 标量按原样打印）
  take-lease --session S [--ttl 1800] [--takeover --reason R]
                                          取/续 lease；他人持有时未过期 → exit 3；
                                          --takeover 显式接管（必须给 --reason，落 history）
  release-lease --session S [--reason R]  释放 lease（停点必须释放，便于他人接管）
  transition --session S --to PHASE [--patch JSON] [--note TEXT]
                                          CAS 迁移：校验 lease 归属 + expected_hash 一致
  freeze     --session S                  冻结 corpus_hash 与 assertion_hashes（批准时调用）
  check-freeze                            核对冻结值未漂移（0=未漂移；4=漂移）

CAS 语义：每次写入前读文件原文算 sha256，调用方可用 --expect <hash> 声明期望值；
不一致即 exit 1（禁止静默覆盖他人推进）。
"""
import argparse
import hashlib
import json
import os
import sys
import time

STATE_DEFAULT = os.path.join("docs", "audit", "loop-state.json")
AUDIT_DEFAULT = os.path.join("docs", "audit")


def sha256_text(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def load(path):
    with open(path, encoding="utf-8") as handle:
        raw = handle.read()
    return json.loads(raw), raw


def dump(path, state):
    text = json.dumps(state, ensure_ascii=False, indent=2) + "\n"
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.replace(tmp, path)
    return sha256_text(text)


def get_path(state, dotted):
    node = state
    for part in dotted.split("."):
        if isinstance(node, list):
            node = node[int(part)]
        elif isinstance(node, dict):
            node = node.get(part)
        else:
            return None
        if node is None:
            return None
    return node


def set_path(state, dotted, value):
    parts = dotted.split(".")
    node = state
    for part in parts[:-1]:
        if part not in node or not isinstance(node[part], dict):
            node[part] = {}
        node = node[part]
    node[parts[-1]] = value


def lease_holder(state):
    lease = state.get("lease") or {}
    return lease.get("session_id"), int(lease.get("expires_unix") or 0)


def require_lease(state, session, allow_expired=False):
    holder, expires = lease_holder(state)
    now = int(time.time())
    if holder != session:
        print(f"lease 属于 {holder}，本会话 {session} 无权写入", file=sys.stderr)
        return False
    if not allow_expired and expires <= now:
        print(f"lease 已过期（expires_unix={expires}，now={now}），请先 take-lease 续租", file=sys.stderr)
        return False
    return True


def cmd_get(args):
    state, _ = load(args.state)
    value = get_path(state, args.path)
    if value is None:
        return 1
    if isinstance(value, (dict, list)):
        print(json.dumps(value, ensure_ascii=False))
    else:
        print(value)
    return 0


def cmd_take_lease(args):
    state, raw = load(args.state)
    holder, expires = lease_holder(state)
    now = int(time.time())
    if holder and holder != args.session and expires > now:
        if not args.takeover:
            print(
                f"lease 被 {holder} 持有至 {expires}（剩余 {expires - now}s）→ 双跑冲突，no-op",
                file=sys.stderr,
            )
            return 3
        # 显式接管：用户已裁定「另一会话暂停」，但持有方进程仍可能存活。
        # 因此强制要求给出理由（落 history 可审计），且必须带 --expect 做 CAS，
        # 防止在「对方其实刚写过」的瞬间静默覆盖推进权。
        if not args.reason:
            print("--takeover 必须同时给出 --reason（写明接管依据，落 history）", file=sys.stderr)
            return 2
        if not args.expect:
            print("--takeover 必须同时给出 --expect <hash>（CAS 守卫，防止覆盖对方刚写入的推进）", file=sys.stderr)
            return 2
    if args.expect and args.expect != sha256_text(raw):
        print(f"CAS 失配：期望 {args.expect}，实际 {sha256_text(raw)}", file=sys.stderr)
        return 1
    previous = holder
    state["lease"] = {
        "session_id": args.session,
        "expires_unix": now + args.ttl,
        "taken_over_from": previous if previous and previous != args.session else None,
        "taken_over_at_unix": now,
    }
    if previous and previous != args.session:
        state.setdefault("history", []).append(
            {
                "at_unix": now,
                "session": args.session,
                "transition": f"lease takeover: {previous} -> {args.session}",
                "note": args.reason,
            }
        )
    new_hash = dump(args.state, state)
    print(json.dumps({"lease": state["lease"], "hash": new_hash}, ensure_ascii=False))
    return 0


def cmd_release_lease(args):
    state, raw = load(args.state)
    holder, _ = lease_holder(state)
    if holder != args.session:
        print(f"lease 属于 {holder}，不能由 {args.session} 释放", file=sys.stderr)
        return 1
    if args.expect and args.expect != sha256_text(raw):
        print(f"CAS 失配：期望 {args.expect}，实际 {sha256_text(raw)}", file=sys.stderr)
        return 1
    state["lease"] = {
        "session_id": None,
        "expires_unix": 0,
        "released_by": args.session,
        "released_reason": args.reason,
    }
    print(dump(args.state, state))
    return 0


def cmd_transition(args):
    state, raw = load(args.state)
    if not require_lease(state, args.session):
        return 3
    if args.expect and args.expect != sha256_text(raw):
        print(f"CAS 失配：期望 {args.expect}，实际 {sha256_text(raw)}", file=sys.stderr)
        return 1
    current = state.setdefault("current", {})
    previous_phase = current.get("phase")
    if args.to:
        current["phase"] = args.to
    if args.patch:
        try:
            patch = json.loads(args.patch)
        except json.JSONDecodeError as error:
            print(f"--patch 不是合法 JSON：{error}", file=sys.stderr)
            return 2
        for key, value in patch.items():
            if key.startswith("current."):
                current[key[len("current."):]] = value
            else:
                set_path(state, key, value)
    state.setdefault("history", []).append(
        {
            "at_unix": int(time.time()),
            "session": args.session,
            "transition": f"{previous_phase} -> {args.to}" if args.to else "patch",
            "note": args.note or "",
        }
    )
    if args.note:
        state["next_action"] = args.note
    new_hash = dump(args.state, state)
    print(json.dumps({"phase": current.get("phase"), "hash": new_hash}, ensure_ascii=False))
    return 0


def _hash_dir(target_dir, hashes, prefix=""):
    """收集目录下 .sh/.py 的哈希；键口径：`-`→`_`，保留扩展名。"""
    if not os.path.isdir(target_dir):
        return
    for name in sorted(os.listdir(target_dir)):
        if not name.endswith((".sh", ".py")):
            continue
        with open(os.path.join(target_dir, name), "rb") as handle:
            hashes[prefix + name.replace("-", "_")] = hashlib.sha256(handle.read()).hexdigest()


# 冻结集分两类，理由不同：
#
# 1. **判定语义**（必须冻结）：阈值断言、取证口径、裁决者、棘轮、重放/采集器。
#    改了它们等于改了「什么叫通过」，必须在批准时冻结、事后改动须新提案重审。
# 2. **检查器自身**（不冻结）：`state.py` / `freeze-check.sh` / `scope_check.py` /
#    `lib.sh` / `selftest.sh`。它们**实现**冻结检查而不是**定义**通过标准；
#    若把自己也冻结，任何一次修 bug 都会立刻变成「冻结漂移」假红（自指缺陷，
#    r1-dim6 实测一次）。其完整性由 git 历史 + code review + `selftest.sh` 自测承担。
JUDGING_FILES = (
    "assertions/dim1.sh",
    "assertions/dim3.sh",
    "assertions/dim6.sh",
    "analyze_latency_gaps.py",
    "analyze_replay_isolation.py",
    "analyze_span_fidelity.py",
    "collect_dim1.py",
    "collect_dim3.py",
    "fetch.sh",
    "replay.sh",
    "ratchet.sh",
    "ratchet_add.py",
    "verdict.sh",
)


def collect_assertion_hashes(args):
    """冻结集 = 本轮人工等价脚本（rounds/<round>/tools）+ 判定语义节点（见 JUDGING_FILES）。"""
    hashes = {}
    _hash_dir(os.path.join(args.audit_root, "rounds", args.round, "tools"), hashes)
    repo_root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    scripts_dir = os.path.join(repo_root, "scripts", "audit")
    for rel in JUDGING_FILES:
        path = os.path.join(scripts_dir, rel)
        if not os.path.exists(path):
            continue
        key = "scripts_" + rel.replace("/", "_").replace("-", "_")
        with open(path, "rb") as handle:
            hashes[key] = hashlib.sha256(handle.read()).hexdigest()
    return hashes


def cmd_freeze(args):
    state, raw = load(args.state)
    if not require_lease(state, args.session):
        return 3
    corpus_dir = os.path.join(args.audit_root, "corpus")
    tasks_dir = os.path.join(corpus_dir, "tasks")
    names = sorted(name for name in os.listdir(tasks_dir) if name.endswith(".md"))
    concatenated = b""
    per_task = {}
    for name in names:
        path = os.path.join(tasks_dir, name)
        with open(path, "rb") as handle:
            data = handle.read()
        concatenated += data
        per_task[name] = hashlib.sha256(data).hexdigest()
    aggregate = hashlib.sha256(concatenated).hexdigest()

    assertion_hashes = collect_assertion_hashes(args)

    current = state.setdefault("current", {})
    current["corpus_hash"] = aggregate
    current["corpus_hash_frozen"] = True
    current["corpus_task_hashes"] = per_task
    current["assertion_hashes"] = assertion_hashes
    new_hash = dump(args.state, state)
    print(
        json.dumps(
            {"corpus_hash": aggregate, "tasks": per_task, "assertion_hashes": assertion_hashes,
             "hash": new_hash},
            ensure_ascii=False,
            indent=1,
        )
    )
    return 0


def cmd_check_freeze(args):
    state, _ = load(args.state)
    current = state.get("current") or {}
    frozen_corpus = current.get("corpus_hash")
    frozen_assertions = current.get("assertion_hashes") or {}
    if not current.get("corpus_hash_frozen"):
        print("corpus_hash 尚未冻结（批准前不可冻结，属正常）")
        return 0
    # 空断言冻结集 ≠ 「没有漂移」，而是**防篡改根本没装上**：阈值脚本可被随意改写而检查恒绿。
    # r2-dim1 实测：批准→implement 未跑 freeze，assertion_hashes={} 仍被 check-freeze 判 PASS。
    # 按「UNKNOWN ≠ PASS」纪律返回 2（证据缺失），由 verdict 统一判 UNKNOWN，绝不折算成 pass。
    if not frozen_assertions:
        print("assertion_hashes 为空：断言脚本未被冻结（批准时漏跑 freeze）→ UNKNOWN", file=sys.stderr)
        return 2

    tasks_dir = os.path.join(args.audit_root, "corpus", "tasks")
    concatenated = b""
    for name in sorted(name for name in os.listdir(tasks_dir) if name.endswith(".md")):
        with open(os.path.join(tasks_dir, name), "rb") as handle:
            concatenated += handle.read()
    actual = hashlib.sha256(concatenated).hexdigest()
    drift = []
    if actual != frozen_corpus:
        drift.append(f"corpus: frozen={frozen_corpus} actual={actual}")

    actual_assertions = collect_assertion_hashes(args)
    for key, expected in frozen_assertions.items():
        actual_value = actual_assertions.get(key)
        if actual_value is None:
            drift.append(f"assertion 缺失: {key}")
        elif actual_value != expected:
            drift.append(f"assertion 漂移: {key}")

    if drift:
        for item in drift:
            print(f"DRIFT {item}", file=sys.stderr)
        return 4
    print(f"冻结一致：corpus={actual} assertions={len(frozen_assertions)}")
    return 0


def cmd_archive(args):
    """把当前轮归档为 `rounds/<round>/round-state.json`（不可变轮次快照）。

    动机（r1-dim6 实测缺陷）：`loop-state.current` 只描述**当前**轮；一旦队列推进到
    下一轮，旧轮的 baseline/candidate/write_scope/冻结哈希全部丢失，`verdict.sh`
    复跑会把新轮字段当旧轮用 ⇒ 假 UNKNOWN。轮次快照让「历史轮可复跑裁决」成立，
    这也是 patrol 巡检能重放历史断言的前提。
    """
    state, _ = load(args.state)
    current = state.get("current") or {}
    round_id = args.round or current.get("round")
    if not round_id:
        print("无 round 可归档", file=sys.stderr)
        return 2
    snapshot_dir = os.path.join(args.audit_root, "rounds", round_id)
    os.makedirs(snapshot_dir, exist_ok=True)
    snapshot_path = os.path.join(snapshot_dir, "round-state.json")
    snapshot = {
        "schema": 1,
        "kind": "round-snapshot",
        "round": round_id,
        "archived_unix": int(time.time()),
        "current": dict(current),
        # 快照自带 lease 以便 freeze 等需要 lease 的子命令可对历史轮操作；
        # 它不参与并发控制（并发控制只认 loop-state.json 的 lease）。
        "lease": {"session_id": args.session, "expires_unix": int(time.time()) + 3600},
    }
    new_hash = dump(snapshot_path, snapshot)
    print(json.dumps({"snapshot": snapshot_path, "hash": new_hash}, ensure_ascii=False))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--state", default=STATE_DEFAULT)
    parser.add_argument("--audit-root", default=AUDIT_DEFAULT)
    parser.add_argument("--round", default="r1-dim6")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_get = sub.add_parser("get")
    p_get.add_argument("--path", required=True)
    p_get.set_defaults(func=cmd_get)

    p_take = sub.add_parser("take-lease")
    p_take.add_argument("--session", required=True)
    p_take.add_argument("--ttl", type=int, default=1800)
    p_take.add_argument("--expect", default="")
    p_take.add_argument("--takeover", action="store_true",
                        help="显式接管他人未过期 lease（须同时给 --reason 与 --expect）")
    p_take.add_argument("--reason", default="")
    p_take.set_defaults(func=cmd_take_lease)

    p_rel = sub.add_parser("release-lease")
    p_rel.add_argument("--session", required=True)
    p_rel.add_argument("--reason", default="停点释放")
    p_rel.add_argument("--expect", default="")
    p_rel.set_defaults(func=cmd_release_lease)

    p_tr = sub.add_parser("transition")
    p_tr.add_argument("--session", required=True)
    p_tr.add_argument("--to", default="")
    p_tr.add_argument("--patch", default="")
    p_tr.add_argument("--note", default="")
    p_tr.add_argument("--expect", default="")
    p_tr.set_defaults(func=cmd_transition)

    p_fr = sub.add_parser("freeze")
    p_fr.add_argument("--session", required=True)
    p_fr.set_defaults(func=cmd_freeze)

    p_cf = sub.add_parser("check-freeze")
    p_cf.set_defaults(func=cmd_check_freeze)

    p_ar = sub.add_parser("archive")
    p_ar.add_argument("--session", required=True)
    p_ar.add_argument("--round", default="")
    p_ar.set_defaults(func=cmd_archive)

    args = parser.parse_args()
    sys.exit(args.func(args))


if __name__ == "__main__":
    main()
