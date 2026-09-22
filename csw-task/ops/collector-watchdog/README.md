# 首期看门狗

盯**早上那一轮**有没有按点走。切换后启用，稳定后停掉
（连续 5 个工作日、含一个周一无触发）。

## 它怎么判断

全部来自 `curl 127.0.0.1:8090/metrics`——不装 sqlite3、不拿会话、不连引擎。
七个指标（`csw_collector_last_task_round_*`）说的是**最近一个派单轮**：
手动轮与预取轮不算，它盯的是真派下来的那一期。

| 北京时间 | 看什么 | 不对就 |
|---|---|---|
| 05:35 | 今天那一轮开了没 | **回退** |
| 05:40 | 图片识别 ≥95% | 告警 |
| 05:50 | 首批交了没 | 告警 |
| 06:05 | 十步走完没 | **回退** |
| 06:20 | 判断覆盖 = 100%（每条都判是硬口径） | 告警 |
| 随时 | 磁盘 ≥92% | **回退** |
| 随时 | 磁盘 ≥88%、outbox 有 conflict、healthz 不 ok | 告警 |

## 自动回退默认是关的

计划里写的是「触发即自动回退」。能力按计划做全了，但默认关着
（`AUTO_ROLLBACK=0`），理由是：**这个看门狗自己一期都还没跑过**，
而误判的代价是「在一切正常的时候把执行者切走」——那比晚二十分钟严重得多。

关着的时候它照样判断，只是把该做的事**告诉人**而不是自己做。

开着它的正确时机是**首期人盯着的时候**：人在旁边，它误判了能立刻按回去。

```bash
echo AUTO_ROLLBACK=1 >> /opt/csw-collector/watchdog.env
```

## 告警去重

同一件事一天只喊一次（标记落在 `data/watchdog/<日期>.<键>`）。
每 5 分钟一次、不去重的话，一个没 ack 会在早上刷出二十条一模一样的告警，
然后就再没有人看它了。标记保留 7 天。

## 装 / 停

```bash
# 装
scp -P 9318 watchdog.sh csw-collector-watchdog.{service,timer} root@8.138.23.218:/tmp/
ssh -p 9318 root@8.138.23.218 '
  install -m 755 /tmp/watchdog.sh /opt/csw-collector/bin/watchdog.sh
  install -m 644 /tmp/csw-collector-watchdog.{service,timer} /etc/systemd/system/
  systemctl daemon-reload && systemctl enable --now csw-collector-watchdog.timer'

# 先空跑一次看它说什么（不会触发回退，因为默认关着）
ssh -p 9318 root@8.138.23.218 '/opt/csw-collector/bin/watchdog.sh'

# 停
systemctl disable --now csw-collector-watchdog.timer
```

## 回退只做了一半的时候

`stop csw-collector` 成了、`start hermes-gateway-collector` 没成——
那一刻**两边都没有收集员**，这一期没人做 01。这是回退路径上最该喊的一种，
所以它有独立的告警键（`rollback_half`），不跟成功那条挤同一个去重标记。

报告在**执行之后**发，内容是 `systemctl is-active` 的真实回答：
先报「已回退」再去执行，万一 stop 或 start 失败，那条告警就是假的。

回退标记（`<日期>.rolled_back`）在执行**之前**落——万一做到一半进程被杀，
下次跑不会再回退一遍；那种情况下 csw-collector 已经停了，
盘点第三节的 `svc_down` 会把它喊出来。

## 本地验过什么

拿一个假 `/metrics` 端点 + 伪造北京时钟（`WATCHDOG_FAKE_HM`）+ 假 `systemctl`
跑过十一种情形：磁盘 93 / 89、outbox conflict、05:36 没接到单、
05:41 识别 60% / 98%、05:51 首批没交、06:06 十步没走完、06:21 判断 80%、
06:21 一切按点该安静、连跑三次的告警去重，以及自动回退开着时
Hermes 起得来 / 起不来两条路径。

抓到过两个 bash 的真 bug，都只在**要报警的那条路径上**才发作：

1. 告警文案里裸 `$MEDIA` 后面紧跟中文全角括号，bash 把那几个字节当成变量名，
   `set -u` 让脚本当场死掉。
2. `systemctl is-active` 对 inactive / failed 是**有输出的非零退出**，
   于是 `$(cmd || echo unknown)` 把两个都拼进了状态串，
   告警里的状态夹着换行和 unknown——而那正是要人一眼看清状态的地方。

## 没验证过的

〔**真机上一次都没跑过**〕——告警能不能发出去、`systemctl --user` 在 timer
的环境里能不能操作 root 的用户级单元（脚本备了 `XDG_RUNTIME_DIR=/run/user/0`
这条退路，但没验证）。装上之后**先手动跑一次**，再等它自己跑。
