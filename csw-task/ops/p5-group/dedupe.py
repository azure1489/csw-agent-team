#!/usr/bin/env python3
"""把群里补回收的原话与 P5 第一步已有的去重，只留新的，交给 `csw-collector p5-distill`。

跑在 **agent 主机**上（`quotes.jsonl` 在那里）。**标准输出只打计数**，原话一个字都不打。

判重：规整后的正文相同，且时间相差不超过 10 分钟——同一句话被路由进某个 agent 的会话库时，
那边记的时间是 agent 收到的时间，与飞书的发送时间会差几秒到几分钟。

    python3 dedupe.py --have /opt/csw-collector/p5/quotes.jsonl \
        --new /opt/csw-collector/p5/group/quotes-group.jsonl \
        --out /opt/csw-collector/p5/group/quotes-group-new.jsonl
"""
import argparse
import json
import os
import re
import time


def norm(s):
    s = re.sub(r"@\S+", "", s or "")
    return re.sub(r"\s+", "", s)


def ts(s):
    try:
        return time.mktime(time.strptime(s[:19], "%Y-%m-%dT%H:%M:%S"))
    except ValueError:
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--have", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--window", type=int, default=600, help="秒")
    a = ap.parse_args()

    have = {}
    for line in open(a.have):
        q = json.loads(line)
        have.setdefault(norm(q["text"]), []).append(ts(q.get("said_at", "")))

    kept, dup = [], 0
    for line in open(a.new):
        q = json.loads(line)
        t = ts(q["said_at"])
        seen = have.get(norm(q["text"]), [])
        if any(x is not None and t is not None and abs(x - t) <= a.window for x in seen):
            dup += 1
            continue
        kept.append(q)

    old = os.umask(0o077)
    try:
        with open(a.out, "w") as f:
            for q in kept:
                f.write(json.dumps(q, ensure_ascii=False) + "\n")
    finally:
        os.umask(old)
    os.chmod(a.out, 0o600)
    print(f"群里补回收 {len(kept) + dup} 条：与已有重复 {dup}，新的 {len(kept)} 条 → {a.out}（0600）")


if __name__ == "__main__":
    main()
