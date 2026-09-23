#!/usr/bin/env python3
"""P5 群消息补回收（第一步的补充）：从飞书编辑部群取 Van 的发言，写成与 `quotes.jsonl` 同格式的文件。

为什么要补：P5 第一步读的是 Hermes 各 agent 的会话库，**每个库只存路由给那个 agent 的群消息**，
编辑部群里她说的话大半不在里面（09-22 判断台账反馈第七项要的是「编辑部群及所有私聊」）。

跑在**引擎主机**上（飞书应用凭证在 /opt/csw-task/server.env），产物只落本机磁盘（0600），
**标准输出只打计数**，原话一个字都不打——它要经 ssh 回到操作者那边。

    set -a; . /opt/csw-task/server.env; set +a
    python3 fetch_group_quotes.py --chat oc_... --who ou_... --since 2026-06-01 --out /root/p5-group/quotes-group.jsonl

需要应用有 im:message.group_msg（获取群组中所有消息）与 im:message:readonly。
"""
import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

API = "https://open.feishu.cn/open-apis"


def post(url, body):
    r = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(r, timeout=20))


def get(url, tok):
    r = urllib.request.Request(url, headers={"Authorization": "Bearer " + tok})
    try:
        return json.load(urllib.request.urlopen(r, timeout=30))
    except urllib.error.HTTPError as e:
        return json.load(e)


def text_of(m):
    """消息正文 → 纯文本。@ 占位换回名字；图片、文件等没有文字的返回空串。"""
    try:
        c = json.loads(m.get("body", {}).get("content") or "{}")
    except ValueError:
        return ""
    t = m.get("msg_type")
    if t == "text":
        s = c.get("text", "")
    elif t == "post":
        parts = []
        if c.get("title"):
            parts.append(c["title"])
        for line in c.get("content") or []:
            seg = []
            for el in line:
                if el.get("tag") == "text":
                    seg.append(el.get("text", ""))
                elif el.get("tag") == "a":
                    seg.append(el.get("text") or el.get("href", ""))
                elif el.get("tag") == "at":
                    seg.append("@" + (el.get("user_name") or ""))
            parts.append("".join(seg))
        s = "\n".join(p for p in parts if p)
    else:
        return ""
    for men in m.get("mentions") or []:
        s = s.replace(men.get("key", "\0"), "@" + (men.get("name") or ""))
    return s.strip()


def iso(ms):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(int(ms) / 1000))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--chat", required=True)
    ap.add_argument("--who", required=True, help="她在这个应用下的 open_id（按应用隔离）")
    ap.add_argument("--since", default="2026-06-01")
    ap.add_argument("--context", type=int, default=3)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    tok = post(f"{API}/auth/v3/tenant_access_token/internal",
               {"app_id": os.environ["CSW_LARK_APP_ID"], "app_secret": os.environ["CSW_LARK_APP_SECRET"]})["tenant_access_token"]
    start = int(time.mktime(time.strptime(a.since, "%Y-%m-%d")))
    url = (f"{API}/im/v1/messages?container_id_type=chat&container_id={a.chat}"
           f"&start_time={start}&page_size=50&sort_type=ByCreateTimeAsc")
    msgs, pt = [], None
    while True:
        d = get(url + (f"&page_token={pt}" if pt else ""), tok)
        if d.get("code") != 0:
            print("接口返回", d.get("code"), d.get("msg"), file=sys.stderr)
            sys.exit(1)
        msgs.extend(d["data"].get("items", []))
        if not d["data"].get("has_more"):
            break
        pt = d["data"].get("page_token")

    lines = []
    for m in msgs:
        s = m.get("sender", {})
        lines.append({
            "id": m.get("message_id"),
            "who": s.get("id") if s.get("sender_type") == "user" else "",
            "role": "user" if s.get("sender_type") == "user" else "assistant",
            "at": iso(m["create_time"]),
            "text": text_of(m),
        })

    def ctx(rows):
        return [{"role": r["role"], "at": r["at"], "text": r["text"][:400]} for r in rows if r["text"]]

    out = []
    for i, ln in enumerate(lines):
        if ln["who"] != a.who or not ln["text"]:
            continue
        out.append({
            "profile": "lark-group",
            "session_id": a.chat,
            "chat_type": "group",
            "message_id": ln["id"],
            "said_at": ln["at"],
            "text": ln["text"],
            "context_before": ctx(lines[max(0, i - a.context):i]),
            "context_after": ctx(lines[i + 1:i + 1 + a.context]),
        })

    os.makedirs(os.path.dirname(a.out), exist_ok=True)
    old = os.umask(0o077)
    try:
        with open(a.out, "w") as f:
            for q in out:
                f.write(json.dumps(q, ensure_ascii=False) + "\n")
    finally:
        os.umask(old)
    os.chmod(a.out, 0o600)
    months = {}
    for q in out:
        months[q["said_at"][:7]] = months.get(q["said_at"][:7], 0) + 1
    print(f"群消息 {len(msgs)} 条；她的有文字的发言 {len(out)} 条；按月 {sorted(months.items())}；写到 {a.out}（0600）")


if __name__ == "__main__":
    main()
