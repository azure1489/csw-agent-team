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

## 没验证过的

〔**整份都没在真机上跑过**〕——包括告警能不能发出去、`systemctl --user` 在
timer 的环境里能不能操作 root 的用户级单元（脚本里备了 `XDG_RUNTIME_DIR=/run/user/0`
这条退路，但没验证）。装上之后**先手动跑一次**，再等它自己跑。
