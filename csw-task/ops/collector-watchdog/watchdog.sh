#!/usr/bin/env bash
# 情报收集员工作台的首期看门狗。装在 agent 主机上，systemd timer 每 5 分钟叫一次。
#
# 它只做两件事：**盯早上那一轮有没有按点走**，以及**在明确的几种情形下回退**。
# 判断全部来自 `curl 127.0.0.1:8090/metrics`——不装 sqlite3、不拿会话、不连引擎。
#
# ⚠️ **自动回退默认是关的**（AUTO_ROLLBACK=0）。
#
#    计划里写的是「触发即自动回退」。实现按计划做全了，但默认关着，理由是：
#    这个看门狗自己一期都还没跑过，而误判的代价是「在一切正常的时候把执行者切走」，
#    那比晚二十分钟严重得多。开着它的正确时机是**首期人盯着的时候**——
#    人在旁边，它误判了你能立刻按回去。
#
#    打开：在 /opt/csw-collector/watchdog.env 里写 AUTO_ROLLBACK=1
#
# 装：
#   scp -P 9318 watchdog.sh csw-collector-watchdog.{service,timer} root@8.138.23.218:/tmp/
#   ssh -p 9318 root@8.138.23.218 '
#     install -m 755 /tmp/watchdog.sh /opt/csw-collector/bin/watchdog.sh
#     install -m 644 /tmp/csw-collector-watchdog.service /tmp/csw-collector-watchdog.timer /etc/systemd/system/
#     systemctl daemon-reload && systemctl enable --now csw-collector-watchdog.timer'
#
# 停（切换稳定后，连续 5 个工作日含一个周一无触发）：
#   systemctl disable --now csw-collector-watchdog.timer
set -uo pipefail          # 不要 -e：看门狗自己挂掉比它报的问题更糟

METRICS="${METRICS:-http://127.0.0.1:8090/metrics}"
HEALTHZ="${HEALTHZ:-http://127.0.0.1:8090/healthz}"
STATE_DIR="${STATE_DIR:-/opt/csw-collector/data/watchdog}"
ENV_FILE="${ENV_FILE:-/opt/csw-collector/watchdog.env}"
AUTO_ROLLBACK="${AUTO_ROLLBACK:-0}"
WEBHOOK="${CSW_COLLECTOR_ALERT_WEBHOOK:-}"

# shellcheck disable=SC1090
[ -f "$ENV_FILE" ] && . "$ENV_FILE"
mkdir -p "$STATE_DIR"

# ── 说话 ──────────────────────────────────────────────────────────

say() { printf '%s %s\n' "$(date '+%F %T')" "$*"; }

# 同一件事一天只喊一次。看门狗每 5 分钟跑一次，不去重的话
# 一个没 ack 会在早上刷出二十条一模一样的告警，然后没有人再看它。
alert_once() {
  local key="$1" text="$2"
  local mark="$STATE_DIR/$(date +%F).$key"
  [ -f "$mark" ] && return 0
  : > "$mark"
  say "告警 [$key] $text"
  [ -n "$WEBHOOK" ] || { say "（没配 webhook，只进日志）"; return 0; }
  curl -s -m 10 -X POST "$WEBHOOK" \
    -H 'Content-Type: application/json' \
    -d "$(printf '{"msg_type":"text","content":{"text":%s}}' \
          "$(printf '%s' "【收集员工作台看门狗】$text" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')")" \
    >/dev/null || say "告警发不出去（只进日志）"
}

# 昨天以前的去重标记清掉，别让 STATE_DIR 长成一片坟场
find "$STATE_DIR" -maxdepth 1 -name '20*' -mtime +7 -delete 2>/dev/null

# ── 取数 ──────────────────────────────────────────────────────────

M="$(curl -s -m 10 "$METRICS")"
if [ -z "$M" ]; then
  alert_once svc_down "服务不响应（$METRICS 取不到）。先看 systemctl status csw-collector"
  exit 0                  # 服务都没了，下面的判断没有意义
fi
g() { printf '%s' "$M" | awk -v k="$1" '$1==k {print $2; exit}'; }

STARTED=$(g csw_collector_last_task_round_started_unix); STARTED=${STARTED:-0}
STEP=$(g csw_collector_last_task_round_step);            STEP=${STEP:-0}
CANDS=$(g csw_collector_last_task_round_candidates);     CANDS=${CANDS:-0}
JUDGED=$(g csw_collector_last_task_round_judged);        JUDGED=${JUDGED:-0}
MEDIA=$(g csw_collector_last_task_round_media);          MEDIA=${MEDIA:-0}
DESCED=$(g csw_collector_last_task_round_media_described); DESCED=${DESCED:-0}
DELIV=$(g csw_collector_last_task_round_delivered);      DELIV=${DELIV:-0}
CONFLICT=$(g csw_collector_outbox_conflict);             CONFLICT=${CONFLICT:-0}
DISK=$(g csw_collector_disk_used_pct);                   DISK=${DISK:--1}

# 北京时间的今天几点几分。服务内部一律 UTC，但盘点的时刻是按北京定的。
#
# `WATCHDOG_FAKE_HM` 是**测试钩子**：早上那几条判断是这份脚本最要紧的部分，
# 而它们一天只有一次能自然验证。没有这个钩子就只能等到明天早上，
# 或者干脆不验——那正是看门狗最容易带着错上线的方式。
BJ_HM=${WATCHDOG_FAKE_HM:-$(TZ=Asia/Shanghai date +%H%M)}
BJ_HM=$((10#$BJ_HM))
TODAY_0530=$(TZ=Asia/Shanghai date -d 'today 05:30' +%s 2>/dev/null \
          || TZ=Asia/Shanghai date -j -f '%Y-%m-%d %H:%M' "$(TZ=Asia/Shanghai date +%F) 05:30" +%s 2>/dev/null \
          || echo 0)

# 今天那一轮开了没：开工时刻要落在今天 05:00 之后
started_today() { [ "$STARTED" -gt 0 ] && [ "$STARTED" -ge "$((TODAY_0530 - 1800))" ]; }

pct() { [ "$2" -gt 0 ] && echo $(( $1 * 100 / $2 )) || echo 100; }

# 告警文案里的变量一律写成 ${VAR}。裸 $VAR 后面紧跟中文标点时，
# bash 会把那几个字节当成变量名的一部分，于是 set -u 让脚本当场死掉——
# **而且只在要报警的那条路径上死**，平时看着一切正常。这个坑踩过一次。

say "轮次开工=$STARTED 步=$STEP 候选=$CANDS 判完=$JUDGED 图=$DESCED/$MEDIA 交付=$DELIV 冲突=$CONFLICT 盘=$DISK%"

# ── 回退 ──────────────────────────────────────────────────────────

rollback() {
  local why="$1"
  if [ "$AUTO_ROLLBACK" != "1" ]; then
    alert_once rollback_would "**本该回退**（自动回退没开）：$why

要人来做：
  systemctl stop csw-collector
  systemctl --user enable --now hermes-gateway-collector.service
主编那边要知道：这一期收集员换回 Hermes 了。"
    return 0
  fi
  local mark="$STATE_DIR/$(date +%F).rolled_back"
  [ -f "$mark" ] && { say "今天已经回退过，不再重复"; return 0; }
  # **标记先落**：万一下面做到一半这个进程被杀，下次跑不会再回退一遍。
  # 那种情况下 csw-collector 已经停了，第三节的 svc_down 会把它喊出来。
  : > "$mark"
  say "开始回退：$why"

  systemctl stop csw-collector
  # Hermes 的网关是 root 的**用户级**单元
  systemctl --user enable --now hermes-gateway-collector.service 2>/dev/null ||
    XDG_RUNTIME_DIR=/run/user/0 systemctl --user enable --now hermes-gateway-collector.service 2>/dev/null

  # 做完再报，而且报的是**真实状态**。
  # 先报「已回退」再去执行，万一 stop 或 start 失败，那条告警就是假的。
  # `systemctl is-active` 对 inactive / failed **是有输出的非零退出**，
  # 所以不能写 `$(cmd || echo unknown)`——那样两个都会进变量，
  # 状态串里就夹着换行和 unknown，而这正是要人一眼看清状态的地方。
  # 取输出，空了才兜底。
  local gone hermes
  gone=$(systemctl is-active csw-collector 2>/dev/null)
  gone=${gone:-unknown}
  hermes=$(systemctl --user is-active hermes-gateway-collector.service 2>/dev/null)
  [ -z "$hermes" ] &&
    hermes=$(XDG_RUNTIME_DIR=/run/user/0 systemctl --user is-active hermes-gateway-collector.service 2>/dev/null)
  hermes=${hermes:-unknown}
  say "回退执行完毕：csw-collector=${gone} hermes-gateway-collector=${hermes}"

  if [ "$hermes" = "active" ]; then
    alert_once rollback_done "**已自动回退**：$why

csw-collector 已停（${gone}），Hermes 收集员已起。
引擎那边不用动——02 / 03 读不到台账会自己退回 intake-trace。
要切回来：systemctl --user disable --now hermes-gateway-collector && systemctl start csw-collector"
  else
    # **两边都没有收集员**了，这是回退路径上最该喊的一种，
    # 用独立的键，不跟成功那条挤同一个去重标记
    alert_once rollback_half "⚠️ **回退只做了一半，现在两边都没有收集员** — $why

csw-collector=${gone}，但 hermes-gateway-collector=${hermes}（起不来）。
这一期没人做 01，要人立刻处理：
  systemctl --user --machine=root@ status hermes-gateway-collector.service
  # 起不来就先把工作台开回去：systemctl start csw-collector"
  fi
}

# ── 盘点 ──────────────────────────────────────────────────────────

# 一、磁盘。到拒开新轮那条线就不必等早上了
if [ "$DISK" -ge 92 ] 2>/dev/null; then
  rollback "磁盘 ${DISK}%，已经到拒开新轮的线（92%）"
fi
if [ "$DISK" -ge 88 ] 2>/dev/null && [ "$DISK" -lt 92 ]; then
  alert_once disk_warn "磁盘 ${DISK}%，到告警线了（92% 就拒开新轮）。清 data/blobs 里的老目录，删掉只会让下次重下"
fi

# 二、outbox 有要人核实的。它不会自己好，因为 conflict 不许自动重试
if [ "$CONFLICT" -gt 0 ] 2>/dev/null; then
  alert_once outbox_conflict "有 $CONFLICT 条写引擎的结果不确定（idempotency_in_progress_or_uncertain）。
**别换幂等键重发**，那会写重。去引擎查那一条落没落，再决定标 confirmed 还是 pending"
fi

# 三、早上那几个盘点。只在 05:30–07:10 之间看，其余时间这些判断没有意义
if [ "$BJ_HM" -ge 530 ] && [ "$BJ_HM" -le 710 ]; then

  # 05:35 还没开轮 = 没接到单
  if [ "$BJ_HM" -ge 535 ] && ! started_today; then
    rollback "05:35 了还没接到单。查两件事：引擎派没派，以及有没有派给另一行 agent
（任务在触发时就钉死到 ActiveAgentByRole 返回的那一行，新建的 agent 接不到单）"
  fi

  if started_today; then
    # 05:40 图片识别覆盖 <95%
    if [ "$BJ_HM" -ge 540 ] && [ "$MEDIA" -gt 0 ]; then
      P=$(pct "$DESCED" "$MEDIA")
      [ "$P" -lt 95 ] && alert_once media_low "05:40 了图片识别才 ${P}%（${DESCED}/${MEDIA}）。
多半是下载或模型网关的事，不是判断的事。**每张图都识别**是硬口径，识别不成的那条会落待核"
    fi

    # 05:50 首批还没交
    if [ "$BJ_HM" -ge 550 ] && [ "$DELIV" -eq 0 ]; then
      alert_once first_batch_late "05:50 了首批还没交（目标 ≤20 分钟）。
看它卡在第几步：现在做完 $STEP 步。预取轮没跑成的话从零跑要一个半小时，这一期就会迟到"
    fi

    # 06:05 整轮还没提交完
    if [ "$BJ_HM" -ge 605 ] && [ "$STEP" -lt 10 ]; then
      rollback "06:05 了整轮还没走完（做完 $STEP / 10 步，交付 $DELIV 件）"
    fi

    # 判断覆盖：每条都判是硬口径，判完的应当等于候选数
    if [ "$BJ_HM" -ge 620 ] && [ "$CANDS" -gt 0 ]; then
      P=$(pct "$JUDGED" "$CANDS")
      [ "$P" -lt 100 ] && alert_once judge_gap "判断覆盖 ${P}%（${JUDGED}/${CANDS}）。
**每条都判**是硬口径，差一条就说明有批次失败了没重跑成"
    fi
  fi
fi

# 四、服务活着但库读不了
HZ="$(curl -s -m 10 "$HEALTHZ")"
printf '%s' "$HZ" | grep -q '"ok":true' ||
  alert_once db_bad "healthz 说不健康（本地库读不了就是它）：$HZ"

exit 0
