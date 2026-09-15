#!/usr/bin/env python3
"""csw-daily-trigger：每晚 21:00 由 systemd 定时器调用，为次日（北京时间）触发 daily_news。

- 次日不是日更日（周末，或 trigger-days.txt 里标 off 的日期）就跳过；标 on 的日期（调休补班）照常触发。
- 触发用调度器 token（daily_news 的 trigger_roles 含 scheduler）；录授权、取消小红书两个阶段用主编 token（中枢动作）。
- 幂等：Idempotency-Key = trigger-daily_news-<日期>，同一天重跑返回同一期；授权与取消同样带幂等键。
- 输入按《资讯日更 · 生产约定》§9：窗口从上一个日更日 07:00 到当天 07:00（周一即覆盖周五以来 72 小时）。

环境变量：CSW_TASK_BASE_URL（缺省 http://127.0.0.1:8080）、CSW_SCHEDULER_TOKEN、CSW_EDITOR_TOKEN、
CSW_TRIGGER_DAYS_FILE（缺省 /opt/csw-task/trigger-days.txt）。
用法：csw-daily-trigger [--date YYYY-MM-DD] [--dry-run]
"""
import argparse
import datetime as dt
import json
import os
import sys
import urllib.error
import urllib.request

CST = dt.timezone(dt.timedelta(hours=8))
AUTHS = [
    ("local_drill", "Van 9/15：首期授权本地演练 + 公众号草稿（定时器按《生产约定》§9 录入）"),
    ("wx_draft", "Van 9/15：允许保存 CAMPsomeWHERE 公众号草稿，全文终审获批后保存并回读"),
]
NO_XHS = ("xhs_text", "xhs_pick")


def load_days(path):
    """读 trigger-days.txt：每行「off YYYY-MM-DD」（休）或「on YYYY-MM-DD」（调休补班），# 后为注释。"""
    off, on = set(), set()
    if path and os.path.exists(path):
        for line in open(path, encoding="utf-8"):
            parts = line.split("#", 1)[0].split()
            if len(parts) == 2 and parts[0] in ("off", "on"):
                (off if parts[0] == "off" else on).add(dt.date.fromisoformat(parts[1]))
    return off, on


def is_workday(d, off, on):
    return d in on or (d.weekday() < 5 and d not in off)


def prev_workday(d, off, on):
    p = d - dt.timedelta(days=1)
    for _ in range(40):
        if is_workday(p, off, on):
            return p
        p -= dt.timedelta(days=1)
    return d - dt.timedelta(days=1)


def build(d, off, on):
    start = prev_workday(d, off, on)
    return {
        "subject": d.isoformat(),
        "title": "资讯日更 " + d.isoformat(),
        "inputs": {
            "约定版本": "v1.0",
            "窗口": {"起": start.isoformat() + "T07:00+08:00", "止": d.isoformat() + "T07:00+08:00"},
            "目标": {"主选": 6, "备选": 2},
            "人审窗口": {"选题": "07:15-07:30", "全文": "09:00-09:15"},
            "授权": [s for s, _ in AUTHS],
            "小红书": False,
        },
    }


def call(base, token, method, path, body=None, idem=None):
    req = urllib.request.Request(
        base.rstrip("/") + "/api/v1" + path, method=method,
        data=json.dumps(body, ensure_ascii=False).encode() if body is not None else None,
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    if idem:
        req.add_header("Idempotency-Key", idem)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            data = json.loads(e.read() or b"{}")
        except ValueError:
            data = {}
        return e.code, data


def main():
    ap = argparse.ArgumentParser(description="为次日触发 daily_news（工作日前一晚 21:00）")
    ap.add_argument("--date", help="日更日期 YYYY-MM-DD，缺省为北京时间明天")
    ap.add_argument("--dry-run", action="store_true", help="只打印将要提交的触发内容")
    a = ap.parse_args()
    off, on = load_days(os.environ.get("CSW_TRIGGER_DAYS_FILE", "/opt/csw-task/trigger-days.txt"))
    d = dt.date.fromisoformat(a.date) if a.date else (dt.datetime.now(CST) + dt.timedelta(days=1)).date()
    if not is_workday(d, off, on):
        print("%s 不是日更日（周末或节假日），跳过" % d)
        return 0
    body = build(d, off, on)
    if a.dry_run:
        print(json.dumps(body, ensure_ascii=False, indent=2))
        return 0
    base = os.environ.get("CSW_TASK_BASE_URL", "http://127.0.0.1:8080")
    sched, editor = os.environ.get("CSW_SCHEDULER_TOKEN", ""), os.environ.get("CSW_EDITOR_TOKEN", "")
    if not sched or not editor:
        print("缺少 CSW_SCHEDULER_TOKEN / CSW_EDITOR_TOKEN", file=sys.stderr)
        return 2
    code, r = call(base, sched, "POST", "/workflows/daily_news/runs", body, idem="trigger-daily_news-" + d.isoformat())
    if code not in (200, 201):
        print("触发失败：HTTP %s %s" % (code, r), file=sys.stderr)
        return 1
    run = r["run_id"]
    w = body["inputs"]["窗口"]
    print("已触发 r%s（%s），窗口 %s ~ %s" % (run, d, w["起"], w["止"]))
    failed = 0
    for scope, quote in AUTHS:
        c, rr = call(base, editor, "POST", "/runs/%s/authorizations" % run, {"scope": scope, "source_quote": quote},
                     idem="auth-%s-%s" % (run, scope))
        if c not in (200, 201):
            failed += 1
            print("录授权 %s 失败：HTTP %s %s" % (scope, c, rr), file=sys.stderr)
    for t in r.get("tasks", []):
        if t.get("stage_code") in NO_XHS and t.get("status") not in ("cancelled", "passed"):
            c, rr = call(base, editor, "POST", "/tasks/%s/cancel" % t["id"],
                         {"reason": "本期不纳入小红书（《生产约定》§9：小红书 false）"}, idem="cancel-%s-noxhs" % t["id"])
            if c not in (200, 201):
                failed += 1
                print("取消 %s（任务#%s）失败：HTTP %s %s" % (t["stage_code"], t["id"], c, rr), file=sys.stderr)
    c, rd = call(base, editor, "GET", "/runs/%s" % run)
    if c == 200:
        st = {}
        for t in rd.get("tasks", []):
            st.setdefault(t["stage_code"], t["status"])
        print("授权 %s；01=%s；10=%s；11=%s；12=%s；13=%s" % (
            rd["run"].get("authorizations"), st.get("intake"), st.get("xhs_text"), st.get("xhs_pick"),
            st.get("xhs_package"), st.get("xhs_save")))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
