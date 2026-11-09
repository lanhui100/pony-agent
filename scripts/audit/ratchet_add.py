#!/usr/bin/env python3
"""把已通过的维度写入棘轮集（PA-169 C4/C6）。

纪律（ADR 0090 同款）：棘轮集**只增不减**；重复写入同一维度必须幂等；
同一维度已登记但脚本路径变化 → 判红（防止偷偷换掉断言口径）。
"""
import argparse
import json
import os
import sys


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dim", required=True)
    parser.add_argument("--round", required=True)
    parser.add_argument("--script", required=True)
    parser.add_argument("--ratchet", required=True)
    args = parser.parse_args()

    data = {"schema": 1, "entries": []}
    if os.path.exists(args.ratchet):
        try:
            with open(args.ratchet, encoding="utf-8") as handle:
                data = json.load(handle)
        except json.JSONDecodeError as error:
            print(f"棘轮文件损坏：{error}", file=sys.stderr)
            return 3

    entries = data.setdefault("entries", [])
    for entry in entries:
        if entry.get("dimension") == args.dim:
            if entry.get("script") != args.script:
                print(
                    f"维度 {args.dim} 已登记脚本 {entry.get('script')}，"
                    f"拒绝替换为 {args.script}（棘轮只增不减）",
                    file=sys.stderr,
                )
                return 3
            entry["last_pass_round"] = args.round
            break
    else:
        entries.append(
            {"dimension": args.dim, "script": args.script, "first_pass_round": args.round,
             "last_pass_round": args.round}
        )

    entries.sort(key=lambda item: item.get("dimension", ""))
    os.makedirs(os.path.dirname(os.path.abspath(args.ratchet)), exist_ok=True)
    with open(args.ratchet, "w", encoding="utf-8") as handle:
        json.dump(data, handle, ensure_ascii=False, indent=1)
        handle.write("\n")
    print(f"棘轮已登记 dim{args.dim}（共 {len(entries)} 维）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
