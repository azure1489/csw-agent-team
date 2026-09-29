#!/usr/bin/env python3
"""主编定时推进的前置检查（cron monitor script）。

有要主编处理的（待审 / 待派工 / 失败 / 自己手上的任务）且主编此刻没有在跑的回合，
就输出待办清单 + 本时段标记——每次都不同，这一跳一定会叫醒主编；
没事、或主编正在群聊里干活（有未过期的回合租约），就输出固定的 "0"——不叫醒，不花模型钱。
09-29：主编只在被 @ 时才动、每轮只挪一小步，03 选题方案拖了一个下午。
"""
import json, os, sqlite3, sys, time, urllib.request

QUIET = "0"
base, tok = os.environ.get("CSW_TASK_BASE_URL"), os.environ.get("CSW_TASK_TOKEN")
if not base or not tok:
    print(QUIET)
    sys.exit(0)

# 主编正在群聊那边跑一个回合：不插手，免得两边同时做同一件事
try:
    db = os.path.expanduser("~/.hermes/profiles/chief/state.db")
    c = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=5)
    busy = c.execute("SELECT count(*) FROM session_turn_leases WHERE expires_at > ?", (time.time(),)).fetchone()[0]
    c.close()
    if busy:
        print(QUIET)
        sys.exit(0)
except Exception:
    pass

try:
    req = urllib.request.Request(base.rstrip("/") + "/api/v1/me/inbox", headers={"Authorization": "Bearer " + tok})
    data = json.load(urllib.request.urlopen(req, timeout=20))
except Exception:
    print(QUIET)
    sys.exit(0)

lines = []
for x in data.get("review_queue") or []:
    # Van 闸等的是 Van 本人：Van 回复会在群里直接 @主编，群聊那边自己会醒；催 Van 由引擎的待审提醒做。
    # 这里叫醒主编也只能确认「还在等 Van」，每跳白花一次模型钱（09-29 16:45 起跑了十几次空转）
    if x.get("relayed_by_hub"):
        continue
    lines.append(f"待审 r{x.get('run_id')} 任务#{x.get('task_id')} {x.get('stage_name')} v{x.get('version')} 交付物#{x.get('deliverable_id')} 闸={x.get('gate_name') or x.get('next_gate')}")
for x in data.get("dispatch_queue") or []:
    lines.append(f"待派工 r{x.get('run_id')} 任务#{x.get('id')} {x.get('stage_name')} 状态={x.get('status')}")
for x in data.get("failed_queue") or []:
    lines.append(f"失败待处置 r{x.get('run_id')} 任务#{x.get('id')} {x.get('stage_name')} 原因={str(x.get('fail_reason') or '')[:80]}")
for x in data.get("my_tasks") or []:
    if x.get("status") in ("dispatched", "in_progress", "returned"):
        lines.append(f"我手上 r{x.get('run_id')} 任务#{x.get('id')} {x.get('stage_name')} 状态={x.get('status')} v{x.get('cur_version')}")

if not lines:
    print(QUIET)
    sys.exit(0)
print("主编待办（引擎 /me/inbox，" + time.strftime("%H:%M") + "）：")
print("\n".join(lines))
