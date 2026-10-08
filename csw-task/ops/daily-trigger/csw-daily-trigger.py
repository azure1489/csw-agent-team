#!/usr/bin/env python3
"""csw-daily-trigger：每天 06:00（北京时间）由 systemd 定时器调用，为当天触发 daily_news。

- **每天都触发**，周末、节假日照跑（2026-10-09 起；此前只在工作日 05:30，按 trigger-days.txt 跳过节假日）。
- 窗口固定为过去 24 小时：前一天 06:00 到当天 06:00。不再按上一个日更日往回推，假期后不会拉出多天的长窗口。
- 01 自动派给情报收集员工作台，做完交付即由引擎在编辑部群 @主编待审（不再等主编手动开期）。
- 服务器宕机后补跑（timer Persistent=true）照常触发：窗口固定、幂等键按日期，晚触发内容不变。
- 触发、录授权或取消失败时，经 notifier 机器人在编辑部群 @主编，请他手动触发（凭证取自 server.env）。
- 触发用调度器 token（daily_news 的 trigger_roles 含 scheduler）；录授权、取消小红书两个阶段用主编 token（中枢动作）。
- 幂等：Idempotency-Key = trigger-daily_news-<日期>，同一天重跑返回同一期；授权与取消同样带幂等键。

环境变量：CSW_TASK_BASE_URL（缺省 http://127.0.0.1:8080）、CSW_SCHEDULER_TOKEN、CSW_EDITOR_TOKEN；
告警用 CSW_LARK_APP_ID / CSW_LARK_APP_SECRET、CSW_DB_PATH。
用法：csw-daily-trigger [--date YYYY-MM-DD] [--dry-run] [--alert-check]
兼容 Python 3.6（生产服务器）。
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
WINDOW_HOUR = 6  # 窗口起止的钟点（北京时间），与定时器的触发时刻一致


def parse_date(s):
    return dt.datetime.strptime(s, "%Y-%m-%d").date()  # date.fromisoformat 要 3.7+


def build(d):
    start = d - dt.timedelta(days=1)
    hh = "T%02d:00+08:00" % WINDOW_HOUR
    return {
        "subject": d.isoformat(),
        "title": "资讯日更 " + d.isoformat(),
        "inputs": {
            "约定版本": "v1.0",
            "窗口": {"起": start.isoformat() + hh, "止": d.isoformat() + hh},
            "目标": {"主选": 6, "备选": 2},
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


def post_json(url, body, token=None):
    req = urllib.request.Request(url, method="POST", data=json.dumps(body, ensure_ascii=False).encode(),
                                 headers={"Content-Type": "application/json; charset=utf-8"})
    if token:
        req.add_header("Authorization", "Bearer " + token)
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read())


def alert_target():
    """告警目标：编辑部群与主编的 @ 标签。读不到返回 (None, 原因)。"""
    app_id, secret = os.environ.get("CSW_LARK_APP_ID", ""), os.environ.get("CSW_LARK_APP_SECRET", "")
    if not app_id or not secret:
        return None, "未配置飞书凭证"
    import sqlite3
    con = sqlite3.connect("file:%s?mode=ro" % os.environ.get("CSW_DB_PATH", "/opt/csw-task/data/csw-task.db"), uri=True)
    chat = con.execute("SELECT chat_key FROM chats ORDER BY id LIMIT 1").fetchone()
    row = con.execute("SELECT open_id, display_name FROM chat_members WHERE role_code='editor' LIMIT 1").fetchone()
    if not chat:
        return None, "花名册未配置群"
    at = '<at user_id="%s">%s</at> ' % (row[0], row[1] or "主编") if row and row[0] else ""
    tok = post_json("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal",
                    {"app_id": app_id, "app_secret": secret}).get("tenant_access_token")
    if not tok:
        return None, "取不到飞书 tenant token"
    return (chat[0], at, tok), ""


def alert(text):
    """定时触发出问题时经 notifier 机器人在编辑部群 @主编（尽力而为，失败只写日志）。"""
    try:
        target, why = alert_target()
        if not target:
            print("无法告警：%s" % why, file=sys.stderr)
            return False
        chat, at, tok = target
        r = post_json("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id",
                      {"receive_id": chat, "msg_type": "text", "content": json.dumps({"text": at + text}, ensure_ascii=False)}, tok)
        return r.get("code") == 0
    except Exception as e:  # 告警失败不影响退出码
        print("告警发送失败：%s" % e, file=sys.stderr)
        return False


def main():
    ap = argparse.ArgumentParser(description="为当天触发 daily_news（每天 06:00，窗口过去 24 小时）")
    ap.add_argument("--date", help="日更日期 YYYY-MM-DD，缺省为北京时间今天")
    ap.add_argument("--dry-run", action="store_true", help="只打印将要提交的触发内容")
    ap.add_argument("--alert-check", action="store_true", help="只检查告警通道（群、@主编、飞书凭证），不发消息")
    a = ap.parse_args()
    if a.alert_check:
        try:
            target, why = alert_target()
        except Exception as e:
            target, why = None, str(e)
        print("告警通道可用：群 %s…，%s" % (target[0][:12], "会 @主编" if target[1] else "花名册里没有主编，不会 @") if target else "告警通道不可用：%s" % why)
        return 0 if target else 1
    d = parse_date(a.date) if a.date else dt.datetime.now(CST).date()
    body = build(d)
    if a.dry_run:
        print(json.dumps(body, ensure_ascii=False, indent=2))
        return 0
    base = os.environ.get("CSW_TASK_BASE_URL", "http://127.0.0.1:8080")
    sched, editor = os.environ.get("CSW_SCHEDULER_TOKEN", ""), os.environ.get("CSW_EDITOR_TOKEN", "")
    if not sched or not editor:
        print("缺少 CSW_SCHEDULER_TOKEN / CSW_EDITOR_TOKEN", file=sys.stderr)
        return 2
    try:
        code, r = call(base, sched, "POST", "/workflows/daily_news/runs", body, idem="trigger-daily_news-" + d.isoformat())
    except Exception as e:  # 引擎不可达等
        code, r = 0, {"error": str(e)}
    if code not in (200, 201):
        print("触发失败：HTTP %s %s" % (code, r), file=sys.stderr)
        alert("【日更定时触发失败 · %s】HTTP %s %s。请按 csw-task skill §4.1 手动触发（inputs 按《生产约定》§9），"
              "并录本地演练与公众号草稿授权、取消 10/11。" % (d, code, str(r)[:120]))
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
    if failed:
        alert("【日更定时触发 · %s】r%s 已触发，但有 %d 项授权或取消未完成（见服务器日志 journalctl -u csw-daily-trigger）。"
              "请在引擎里补录本地演练与公众号草稿授权、取消 10/11。" % (d, run, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
