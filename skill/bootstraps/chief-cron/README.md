# 主编定时推进（agent 主机 chief profile）

- 任务：`hermes --profile chief cron`，id `e0c493db6629`，名「CSW主编定时推进（一轮做到底）」，每 10 分钟，`--skill csw-task`，`--deliver local`（不发群）。
- 前置检查：`csw_hub_push.py` → `/root/.hermes/profiles/chief/scripts/`。有要主编处理的（非 Van 闸的待审、待派工、失败、自己手上的任务）且主编此刻没有在跑的回合，才输出待办清单并叫醒主编；否则输出固定的 `0`，不花模型钱。
- Van 闸的待审不叫醒：等的是 Van 本人，Van 回复会在群里直接 @主编；催 Van 由引擎的待审提醒做。
- 起因：09-29 r56 #594 主编只在被 @ 时才动、每轮只挪一小步。上线后第一跳（16:39）就把 03 选题方案做完提交。
