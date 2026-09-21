# -*- coding: utf-8 -*-
"""情报收集员工作台 · 完整流程（派单为起点）确认页。用法：python build_flow.py out.html"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from diagrams import box, arrow, svg, fig, esc, ACC

out = sys.argv[1]

# ---------- 图 A：全流程（纵向，行 = 步，列 = 谁在做） ----------
def fig_full():
    W = 1180
    lanes = [("引擎（真相）", 40, 260), ("情报收集员工作台（采集服务 + Web）", 300, 340), ("来源 · 模型", 640, 310), ("人", 950, 190)]
    y0, RH, bh = 56, 66, 44
    def ry(row):  # 行 2 起下移给「采集媒体信息」容器留标题位，行 5 起再下移给容器底注留位
        return y0 + row * RH + (0 if row < 2 else 18 if row < 5 else 34)
    H = ry(13) + bh + 16
    b = ""
    for t, x, w in lanes:
        b += f'<rect x="{x}" y="14" width="{w}" height="{H - 20}" fill="none" stroke="currentColor" stroke-opacity=".22"/>'
        b += f'<text x="{x + w / 2}" y="36" text-anchor="middle" font-size="12.5" font-weight="700" fill="currentColor" opacity=".75">{esc(t)}</text>'
    pos = {}
    def rb(lane, row, title, sub=None, accent=False, dashed=False):
        _, lx, lw = lanes[lane]
        bw = {0: 170, 1: 240, 2: 220, 3: 130}[lane]
        x = lx + (lw - bw) / 2
        y = ry(row)
        pos[(lane, row)] = (x, y, bw)
        return box(x, y, bw, bh, title, sub, accent=accent, dashed=dashed, fs=12.5)
    def L(lane, row): x, y, w = pos[(lane, row)]; return x, y + bh / 2
    def R(lane, row): x, y, w = pos[(lane, row)]; return x + w, y + bh / 2
    def T(lane, row): x, y, w = pos[(lane, row)]; return x + w / 2, y
    def B(lane, row): x, y, w = pos[(lane, row)]; return x + w / 2, y + bh

    # 行 0：派单
    b += rb(0, 0, "0 派单 05:30", "引擎触发 run · 01 派给收集员")
    b += rb(3, 0, "0 手动开启", "方案 · 窗口 · 关联任务")
    # 行 1：接单
    b += rb(1, 1, "1 接单 / 手动开工", "任务：ack · 心跳 · 读作业标准")
    b += rb(0, 1, "任务：已接单", "派工单 · 作业标准 · 首批时限")
    x, y = R(0, 0); tx, ty = T(1, 1)
    b += arrow(x, y, tx, ty, "派工单 · 群播报", via=[(tx, y)], lx=x + 8, ly=y - 8, anchor="start")
    x, y = L(3, 0)
    b += arrow(x, y, tx, ty, "人在工作台点开始 · 参数写审计", via=[(tx, y)], lx=(x + tx) / 2, ly=y - 8)
    x, y = L(1, 1); x2, y2 = R(0, 1)
    b += arrow(x, y, x2, y2, "ack · 心跳", lx=(x + x2) / 2, ly=y - 8)
    # 行 2–4：采集媒体信息（一个整体）
    ctop, cbot = ry(2) - 24, ry(4) + bh + 22
    b += f'<rect x="308" y="{ctop}" width="324" height="{cbot - ctop}" rx="8" fill="none" stroke="{ACC}" stroke-width="1.6"/>'
    b += f'<text x="316" y="{ctop + 15}" font-size="12" font-weight="700" fill="{ACC}">2 采集媒体信息</text>'
    b += f'<text x="316" y="{cbot - 7}" font-size="11" fill="{ACC}" opacity=".85">取候选 · 下载 · 识别不拆开</text>'
    b += rb(1, 2, "2a 取候选", "按任务类型跑采集方案 · 采集轮")
    b += rb(2, 2, "来源（采集器各跑一轮）", "csw 贴文 · 小红书 · 网页 · 链接 · 可加")
    x, y = L(2, 2); x2, y2 = R(1, 2)
    b += arrow(x, y, x2, y2, "统一候选", lx=(x + x2) / 2, ly=y - 8)
    b += rb(1, 3, "2b 下载媒体", "图片 · 视频封面与关键帧 · 哈希")
    b += rb(2, 3, "OSS · 原站", "缩略 w_768 · 原图按需 · 视频")
    x, y = L(2, 3); x2, y2 = R(1, 3)
    b += arrow(x, y, x2, y2, "文件", lx=(x + x2) / 2, ly=y - 8)
    b += rb(1, 4, "2c 识别图片视频", "每个媒体 × 该条正文 · 存描述")
    b += rb(2, 4, "模型（视觉）", "对应正文哪点 · 画面 · 图上没有的")
    x, y = L(2, 4); x2, y2 = R(1, 4)
    b += arrow(x, y, x2, y2, "描述", accent=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 5：合并
    b += rb(1, 5, "3 合并与窗口", "候选 → 事件 · 窗口外标注不删")
    b += rb(2, 5, "模型", "是否同一事件", dashed=True)
    x, y = L(2, 5); x2, y2 = R(1, 5)
    b += arrow(x, y, x2, y2, "判定", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 6：查重
    b += rb(1, 6, "4 查重", "有知识库则查 · 否则标「未查」")
    b += rb(0, 6, "知识库 · 选题记忆", "正式已发布 · 案例与准则", dashed=True)
    b += rb(2, 6, "模型", "同一事实？同一角度？", dashed=True)
    x, y = R(0, 6); x2, y2 = L(1, 6)
    b += arrow(x, y, x2, y2, "旧文 · 案例", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 6); x2, y2 = R(1, 6)
    b += arrow(x, y, x2, y2, "结论", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 7：打分
    b += rb(1, 7, "5 打分", "正文 + 媒体描述 + 实图 · 每事件一条")
    b += rb(2, 7, "模型", "六维度 · 依据 · 三句话 · 缺口")
    x, y = L(2, 7); x2, y2 = R(1, 7)
    b += arrow(x, y, x2, y2, "打分结果", lx=(x + x2) / 2, ly=y - 8)
    # 行 8：深核
    b += rb(1, 8, "6 深核", "入围 + 待核 · 可打断 · 线程落盘")
    b += rb(2, 8, "Codex App Server", "只读工具 · 完整图 · 原始来源")
    x, y = L(2, 8); x2, y2 = R(1, 8)
    b += arrow(x, y, x2, y2, "条目卡", lx=(x + x2) / 2, ly=y - 8)
    # 行 9：登记与交付
    b += rb(1, 9, "7 登记与交付", "有任务才写引擎 · 否则只出本地报告")
    b += rb(0, 9, "引擎落库", "条目 · 采集轮 · 台账 · 交付物")
    x, y = L(1, 9); x2, y2 = R(0, 9)
    b += arrow(x, y, x2, y2, "PUT · submit", lx=(x + x2) / 2, ly=y - 8)
    # 工作台主线的纵向连线
    for r in range(1, 9):
        x, y = B(1, r); x2, y2 = T(1, r + 1)
        b += arrow(x, y, x2, y2)
    # 行 10：首批校准
    b += rb(0, 10, "8 首批校准闸", "待主编审 · 群播报")
    b += rb(3, 10, "主编", "看台账 · 通过 / 退回")
    x, y = R(0, 10); x2, y2 = L(3, 10)
    b += arrow(x, y - 8, x2, y2 - 8, "待审", lx=(x + x2) / 2, ly=y - 16)
    b += arrow(x2, y2 + 8, x, y + 8, "通过 / 退回（方向 + 位置）", lx=(x + x2) / 2, ly=y + 24)
    # 行 11：补件
    b += rb(1, 11, "9 返工或补件", "退回则返工 · 通过后其余候选以补件追加")
    b += rb(0, 11, "补件挂在已通过任务", "下游按需返工")
    x, y = B(3, 10); tx, ty = R(1, 11)
    b += arrow(x, y, tx, ty, via=[(x, ty)])
    x, y = L(1, 11); x2, y2 = R(0, 11)
    b += arrow(x, y, x2, y2, "kind=补件", lx=(x + x2) / 2, ly=y - 8)
    # 行 12：下游与 Van
    b += rb(0, 12, "02 → 03 → Van 选题", "研究员评估 · 主编方案 · 03 闸")
    b += rb(1, 12, "台账 · Van 模式", "研究员读台账 · Van 勾选 + 原话")
    b += rb(3, 12, "Van · 主编", "勾选 + 原话 · 主编代录")
    x, y = L(3, 12); x2, y2 = R(1, 12)
    b += arrow(x, y, x2, y2, "勾选", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(1, 12); x2, y2 = R(0, 12)
    b += arrow(x, y, x2, y2, "03 决定", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(0, 12); x2, y2 = L(0, 6)
    b += arrow(x, y + 10, x2, y2 + 10, dashed=True, accent=True, via=[(16, y + 10), (16, y2 + 10)])
    b += f'<text x="30" y="{(y + y2) / 2 + 4}" font-size="11" fill="{ACC}" transform="rotate(-90 30 {(y + y2) / 2 + 4})" text-anchor="middle">决定 + 维度分入记忆，下一期用</text>'
    # 行 13：05 / 11
    b += rb(0, 13, "05 配图 · 11 选图包", "逐条派工 · 同起点")
    b += rb(1, 13, "05 / 11 接单", "采集媒体信息（单条 · 原图）· 核素材 · 交付")
    b += rb(2, 13, "agent.campsomewhere.com", "GET posts/{id}?include-media")
    x, y = R(0, 13); x2, y2 = L(1, 13)
    b += arrow(x, y, x2, y2, "派工单", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 13); x2, y2 = R(1, 13)
    b += arrow(x, y, x2, y2, "单条 + 媒体", lx=(x + x2) / 2, ly=y - 8)
    return fig(svg(W, H, b, "完整流程：引擎派单或人在工作台手动开启，工作台开工后采集媒体信息（取候选、下载、识别一体）、合并、查重、打分、深核；关联任务时登记交付，主编首批校准，补件，下游与 Van 决定回流"),
               "图 A · 完整流程。每一行是一步，列是谁在做。两种入口汇到同一条线：引擎派单，或人在工作台手动开启一轮。工作台这一列是主线：开工之后的每一步都在它上面可见；7 登记与交付、8 首批校准、9 补件和下游只在关联任务时发生，手动开启不关联任务的只出本地台账与报告。强调色框出与 9/20 设计稿不同之处：第 2 步把取候选、下载媒体、识别图片视频合成一个整体，并按任务类型跑可配置的采集方案，取 csw-agent 贴文只是其中一个采集器。虚线是「有则用」：知识库与记忆缺时这一步照跑，只是结果标「未查」。")

# ---------- 图 B：采集媒体信息的采集器模型 ----------
def fig_collectors():
    b = ""
    b += box(20, 40, 180, 48, "任务类型", "intake · material · xhs_pick …")
    b += box(260, 40, 180, 48, "采集方案", "有序采集器清单 · 版本", accent=True)
    b += box(20, 190, 180, 48, "工作台 · 设置页", "增删 · 排序 · 参数 · 回放")
    b += box(260, 190, 180, 48, "来源台账", "必扫 / 启停 / 实例参数")
    b += '<rect x="520" y="20" width="270" height="322" rx="6" fill="none" stroke="currentColor" stroke-opacity=".35" stroke-dasharray="6 4"/>'
    b += '<text x="655" y="40" text-anchor="middle" font-size="12" font-weight="700" fill="currentColor" opacity=".7">采集器（每个各跑一轮，记采集轮）</text>'
    names = [("csw 贴文窗口", "GET posts/window"), ("csw 单条", "GET posts/{id}"), ("小红书", "opencli 受控调用"), ("网页 / RSS", "域名白名单抓取"), ("Van 链接", "解析后交对应采集器")]
    y = 52
    for t, sub in names:
        b += box(540, y, 230, 40, t, sub, fs=12)
        y += 46
    b += box(540, y, 230, 40, "自定义采集器", "外部命令 / HTTP 端点 / MCP 工具", accent=True, dashed=True, fs=12)
    b += box(850, 40, 170, 52, "统一候选格式", "来源 · id · 正文 · 时间 · 媒体")
    b += box(850, 130, 170, 52, "下载媒体", "图片 · 视频封面与关键帧 · 哈希")
    b += box(850, 220, 170, 52, "识别图片视频", "每个媒体 × 正文 · 未识别标记", accent=True)
    b += box(850, 310, 170, 52, "暂存库 + 采集轮", "去重 · discovered_via · 计数")
    b += arrow(200, 64, 260, 64, "选方案", lx=230, ly=56)
    b += arrow(440, 64, 520, 64, "按序跑", lx=480, ly=56)
    b += arrow(790, 180, 850, 66, "各自返回", via=[(820, 180), (820, 66)], lx=826, ly=150, anchor="start")
    b += arrow(935, 92, 935, 130, "每条候选", lx=943, ly=115, anchor="start")
    b += arrow(935, 182, 935, 220, "每个媒体", lx=943, ly=205, anchor="start")
    b += arrow(935, 272, 935, 310, "带描述入库", lx=943, ly=295, anchor="start")
    b += arrow(200, 214, 260, 214, "改方案", lx=230, ly=206)
    b += arrow(350, 190, 350, 88, "实例参数", lx=358, ly=144, anchor="start")
    return fig(svg(1040, 384, b, "采集媒体信息：任务类型选采集方案，采集器各跑一轮，结果归一后逐条下载媒体、逐个识别，带描述入暂存库并记采集轮；方案可在工作台改，也可挂自定义采集器"),
               "图 B · 采集媒体信息。任务类型决定跑哪套采集方案；方案是一份有序的采集器清单，每个采集器各跑一轮、各记一条采集轮；结果归一成同一种候选格式后，不分来源地逐条下载媒体、逐个识别，带着描述进暂存库。取 csw-agent 贴文只是清单里的一项，加来源是改清单，加取法是加一个采集器；改方案在工作台设置页操作，每次改动写审计并存版本。")

# ---------- 流程带示意（HTML） ----------
STRIP = [
    ("0 派单", "05:30", "ok"), ("1 接单", "05:30 · 心跳中", "ok"), ("2 采集媒体信息", "3 采集器 · 438 条 · 媒体 2,311 / 2,361 · 识别 2,311", "warn"),
    ("3 合并", "96 事件 · 窗口外 12", "ok"), ("4 查重", "96 · 重复 9", "ok"), ("5 打分", "96 / 96 · 18 分", "ok"), ("6 深核", "6 / 24", "doing"),
    ("7 登记与交付", "未开始", "todo"), ("8 主编校准", "未开始", "todo"), ("9 补件", "未开始", "todo"),
]
strip_html = "".join(f'<li class="{c}"><b>{esc(t)}</b><span>{esc(s)}</span></li>' for t, s, c in STRIP)

# ---------- 逐步说明 ----------
STEPS = [
    ("0 开工入口", "引擎 或 人", "两种入口。任务驱动：引擎 05:30 定时触发 run（周一窗口到上周五），01 任务派给「情报收集员」角色，群播报由引擎单写；05 / 11 逐条派工同理。手动开启：主编、研究员或开发在工作台点「开始一轮」，选任务类型或采集方案、窗口、采集器参数，并选是否关联一个已接单的任务；发起人、参数、时间写审计。", "任务驱动：run · 任务 · 派工单。手动：一条本地开工记录（标「手动」）", "总览出现新一轮：任务驱动显示期次、任务号、派工单、作业标准与首批时限；手动显示发起人、参数与是否关联任务。", "主编可在管理后台照旧手动触发引擎；在工作台可手动开启一轮。", "引擎触发失败照旧走引擎告警；手动开启参数不合法当场拒绝。"),
    ("1 接单 / 手动开工", "工作台（采集服务）", "任务驱动：每 30 秒查 my-tasks，发现 open 的任务即 ack 并开始心跳，从 GET /tasks/:id 读作业内容、自检、验收，原样显示并注入后面的打分前言。手动开启：直接开工，作业标准取当前定义里对应阶段的版本（只读，不算接单）；两种方式之后走同一条线。", "任务状态：已接单；或本地开工记录", "流程带全部步置「待开始」；作业标准可读；开工方式标明。", "主编可「中止本轮」或「重新开始」（记录人与理由）。", "ack 失败重试；超过 ack 时限由引擎升级中枢，照旧。"),
    ("2 采集媒体信息", "工作台 → 来源 · OSS · 模型", "一步三段，对每条候选连着做完才算采集完成，不分来源。a 取候选：读任务 stage_code 选采集方案，方案里的采集器逐个跑一轮（csw 贴文窗口 GET /api/v1/posts/window；单条 GET /api/v1/posts/{id}?include-media=true；小红书 opencli 受控调用；网页按域名白名单抓取；Van 链接按 id 取单条），结果归一成统一候选格式，按（来源，id）去重，记 discovered_via。b 下载媒体：每条候选的全部媒体，图片取 w_768 缩略（05 / 11 取原图），视频取封面与关键帧，按哈希存本地，失败记原因。c 识别：每张图、每段视频的关键帧连同该条正文送模型，结构化输出对应正文哪一点、画面里的产品 / 场景 / 文字、正文提到但画面没有的、媒体类型；存 media_descriptions（候选 id、媒体哈希、描述、模型与提示词版本）。全部直接调接口，不经 csw MCP。", "完整候选：正文 + 本地媒体 + 每个媒体一条描述；每个采集器一条采集轮（found / in_window 由代码算）", "跑了哪些采集器、各自数量与耗时；下载成功率与读不到的清单；识别进度与未识别的；候选卡上每个媒体下一行描述。", "改采集方案（增删采集器、排序、参数、启停，写审计）；改窗口重取；对某个媒体重下、重识别或手改描述（留痕）。", "必扫采集器重试三次仍失败：本步标失败、停在这里告警；非必扫失败记采集轮 failed 继续。单个媒体下载或识别失败重试一次，仍失败标「未识别」，不算看过，不阻塞。不用旧接口兜底。"),
    ("3 合并与窗口", "工作台（代码为主）", "按发布时间精确判窗口；同账号、同文案开头先粗并；模型判是否同一事件。窗口外的标「硬性排除 · 窗口外」但保留。", "events 表；事件 ↔ 候选映射", "事件数、每个事件合并了哪几条、窗口外分组。", "拆开或合并事件（留痕）。", "模型判定失败时按粗并结果继续，标「未经模型判定」。"),
    ("4 查重", "工作台 + 知识库 + 模型", "代码按品牌、产品、别名到知识库找候选，只对照「正式已发布」；模型判是否同一事实、同一角度。知识库未同步到当天的如实标「已同步到 X 日」；没有知识库就标「未查」，不阻塞。", "每个事件的查重结论与对照旧文", "事件卡的查重标记与对照旧文；知识库同步到哪天。", "改判查重结论（留痕）。", "检索失败标「未查」继续。"),
    ("5 打分", "工作台 → 模型", "固定前言 = 任务下发的作业标准 + 判断框架与各级锚点 +（有则）本批品牌的知识库命中与案例；输入 = 正文 + 第 2 步的媒体描述 + 实图缩略；每批若干事件、每批新会话、并行。输出六维度分、依据、三句话、缺口、优先 / 降低命中、是否看到实图。", "打分台账：每个事件一行", "台账全量；综合分与六维度；权重版本即时重排；缺口。", "捞回、改判、标待核、加入首批（都留痕）。", "某批失败只重跑该批；重试后仍失败的事件标「未打分」并计数。"),
    ("6 深核", "工作台 → Codex App Server", "排名前 K 与待核的事件各起一个线程：核原始披露时间、找原始来源核事实、核比较参照是否准确、看完整图片；工具只挂只读白名单；线程事件流落盘。", "条目卡：披露时间、证据链接、查重说明、事实、配图、缺口", "详情页深核结果与证据；线程记录可下载；进度 K 之几。", "打断某个线程、改 K、对单个事件重跑。", "单线程超时或失败标「深核未完成」，条目仍可登记并写明缺口。"),
    ("7 登记与交付", "工作台 → 引擎", "只在本轮关联了任务时做：PUT items（带 origin）、PUT sweeps、PUT intake-scores；生成 index.md + images/ + trace/ 打 zip；提交前本地跑 intake-check；submit（kind=产出；关联到已通过任务的用 kind=补件）。登记为条目的是排名靠前的、待核的、Van 链接的；其余只在台账。手动开启且未关联任务：不写引擎、不群播报，只出本地台账与报告，事后可挑选结果关联到某个任务以补件提交。", "关联任务：引擎里的条目、采集轮、打分台账、交付物，任务状态待审。未关联：本地报告", "校验结果逐条、提交状态、交付物链接；未关联时显示「本地」并给「关联到任务」入口。", "主编可在提交前先看一眼台账并调整首批（可选，不改闸的位置）；把手动轮的结果关联到任务。", "intake-check 不过则不提交，列出不过项；submit 失败重试，幂等键固定。"),
    ("8 首批校准", "引擎闸 · 主编", "引擎群播报「待审」；主编在工作台看台账、媒体与描述、缺口，回引擎「通过」或「退回」（退回必填方向与位置）。", "任务：已交付 / 需返工", "闸状态、退回原因。", "通过、退回、定点意见。", "超时由引擎照旧提醒。"),
    ("9 返工或补件", "工作台 → 引擎", "退回：按 latest_review 的方向与位置返工，重新交付。通过：其余候选与后续补证以 kind=补件 追加到已通过任务，只让已开工的直接下游返工。", "补件记录", "补件清单与状态。", "主编触发「追加补件」。", "同 7。"),
    ("下游", "引擎 · 研究员 · 主编 · Van", "02 研究员读台账与 intake-trace 评估；03 主编合成选题方案；Van 在工作台 Van 模式勾选并留原话，主编代录 03 闸决定。决定连同当时的维度分入选题记忆，下一期第 4、5 步读得到。", "03 决定；记忆案例", "Van 模式；每条决定与原话；指标页三项。", "Van：采用 / 否决 / 暂缓 + 原话。", "——"),
    ("05 / 11", "引擎 → 同一工作台", "05 配图与素材核（material，逐条自动派工）与 11 小红书选图包（xhs_pick，逐条手动派工）都派给收集员角色，工作台以同样方式接单；按任务类型走「条目取单条」方案：采集媒体信息在同一步完成（取该贴文、下原图不缩略、识别），再核素材与版权、逐条交付（0 闸）。不走合并、查重、打分、深核。", "逐条交付物", "05 / 11 任务清单与进度、每条的原图、描述与核验结果。", "重跑某条。", "同 1、2。"),
]
rows = "".join("<tr>" + "".join(f"<td>{esc(c)}</td>" for c in r) + "</tr>" for r in STEPS)

CHANGES = [
    ("身份", "Hermes agent 上的一个 profile，被飞书 @ 唤醒", "采集服务持「情报收集员」角色的 token，30 秒轮询 my-tasks 接单；工作台前端是它的界面"),
    ("开工方式", "只能被引擎派单或群里 @ 唤醒", "任务驱动照旧；也能在工作台手动开启一轮（选方案、窗口、是否关联任务）；不关联任务的只出本地报告，不写引擎"),
    ("能看到什么", "群里的几条消息与最后的 zip", "整条流程带：每一步的状态、计数、耗时、失败；每一步的产物都能点开"),
    ("采集媒体信息", "模型调 csw MCP 搜索 Instagram 贴文，一次最多 100 条；不下图", "按任务类型跑可配置的采集方案，取候选、下载媒体、识别图片视频一步完成；取 csw-agent 贴文只是其中一个采集器；数量由代码算"),
    ("图片与视频", "不看图", "每张图、每段视频在采集时就按正文识别并保存描述；打分时再看实图"),
    ("判断", "模型自由挑选", "按 Van 的判断框架六维度打分、写依据与三句话；不因口味淘汰"),
    ("过程", "只在群聊里", "采集轮、台账、trace 都在引擎；人的每次操作留痕"),
    ("人的介入", "在群里 @ 收集员", "在工作台捞回、改判、重跑某步、首批校准"),
    ("交付", "手写 index.md", "代码生成 index.md、images/、trace/，提交前跑 intake-check"),
    ("05 配图", "同一个 Hermes profile", "同一个工作台接单"),
]
changes_rows = "".join(f"<tr><td>{esc(a)}</td><td>{esc(b)}</td><td>{esc(c)}</td></tr>" for a, b, c in CHANGES)

DIFF = [
    "起点写清楚了：0 派单 → 1 接单，之前的设计图从「取全量」起画，看不出接单在前。",
    "开工有两种方式：任务驱动与手动开启，之后走同一条线；手动开启不关联任务时只出本地台账与报告，不写引擎、不群播报，用于回放历史窗口、并行验证、试新采集器或来源、白天临时补采；关联任务时与任务驱动一样登记交付。",
    "第 2 步从「取全量」改为「采集媒体信息」：取候选、下载媒体、识别图片视频合成一个整体，每条候选出这一步时已带本地媒体与每个媒体的描述；之前下缩略图和看图分散在 B、D 两步，视频只取封面。",
    "第 2 步按任务类型跑一套可配置的采集方案，采集器直接调来源接口、不经 csw MCP；取 csw-agent 贴文只是其中一个采集器，以后加来源是改清单、加取法是加采集器；MCP 只留给深核线程按需查。",
    "知识库与选题记忆改为「有则用」：第 4、5 步缺它们照跑，结果标「未查」；不再是开工前置。",
    "整条流程做成工作台顶部的流程带，每一步可点开看产物、失败与重跑；之前总览只有一条进度条。",
    "05 配图与素材核、11 选图包明确为同一工作台接单，同一起点，采集媒体信息走「条目取单条」方案。",
]
diff_html = "".join(f"<li>{esc(d)}</li>" for d in DIFF)

KEEP = [
    "引擎仍是唯一真相：任务、条目、采集轮、打分台账、交付物、闸、授权都在引擎；工作台读引擎、写引擎，不另存状态。",
    "群播报仍由引擎单写；工作台不发群消息。",
    "01 的主编首批校准闸位置不变；其余候选以补件追加，不重跑已批内容。",
    "Van 的决定仍经 03 闸生效；工作台上的勾选先记「Van 选择」，由主编代录并附原话。",
    "作业标准、自检、验收仍以引擎定义为准，工作台原样显示，不另抄一份。",
]
keep_html = "".join(f"<li>{esc(k)}</li>" for k in KEEP)

CONFIRM = [
    "媒体描述每个媒体一条，存采集服务本地并随台账显示；是否同时回写 csw 库的 detailed_alt，请定。",
    "视频识别到什么程度：封面 + 关键帧画面描述（建议），是否加语音转写。",
    "必扫采集器失败时停在第 2 步告警、不用旧接口兜底，是否同意。",
    "第 7 步提交前是否允许主编先在工作台调整首批再提交（可选项，不改闸的位置）。",
    "Van 模式的勾选是否直接等同 03 闸决定，还是只记「Van 选择」由主编代录（沿用设计说明第 10 节的问题）。",
    "第 2 步的自定义先做到哪一层：只做「改方案」（第一期建议），还是同时做外部命令 / HTTP 适配器与 MCP 工具两种自定义采集器。",
    "手动开启的权限：operator（主编、研究员）与 superadmin 可开，Van 模式只给链接不开轮；手动轮的结果关联任务时一律以补件提交，是否同意。",
    "是否同意按这版流程重画设计说明第 3 到 6 节与开发版第 3、6 节，并把界面设计稿的总览页改成流程带。",
]
confirm_html = "".join(f"<li>{esc(c)}</li>" for c in CONFIRM)

page = f'''<title>收集员工作台完整流程</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&family=Noto+Serif+SC:wght@600;700&display=swap">
<style>
:root {{ --ground:#F4F5F7; --surface:#FFFFFF; --ink:#1B1F2A; --muted:#5F6673; --rule:#DCDFE6; --accent:#4F46E5; --accent-soft:#EEF0FD; --ok:#1F8A4C; --ok-soft:#E4F4EA; --warn:#B26A00; --warn-soft:#FBF0DC; --code-bg:#ECEEF3;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif; --serif:"Noto Serif SC","Songti SC",serif; --mono:"IBM Plex Mono",Menlo,monospace; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5FCF8A; --ok-soft:#1B3427; --warn:#F0B35A; --warn-soft:#3A2E17; --code-bg:#22263A; }} }}
:root[data-theme="dark"] {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5FCF8A; --ok-soft:#1B3427; --warn:#F0B35A; --warn-soft:#3A2E17; --code-bg:#22263A; }}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:16px; line-height:1.85; padding-inline:20px; padding-block:44px 96px; }}
.page {{ max-width:1080px; margin-inline:auto; display:flex; flex-direction:column; gap:28px; }}
.eyebrow {{ font-size:13px; color:var(--muted); letter-spacing:.08em; display:flex; flex-wrap:wrap; gap:4px 14px; }}
.chip {{ border:1px solid var(--accent); color:var(--accent); padding:0 8px; border-radius:3px; }}
h1 {{ font-family:var(--serif); font-weight:700; font-size:clamp(28px,5vw,38px); line-height:1.3; margin:12px 0 14px; text-wrap:balance; }}
h1 span {{ display:block; font-family:var(--sans); font-size:.5em; font-weight:500; color:var(--accent); margin-top:6px; }}
.lede {{ font-size:17px; line-height:1.9; margin:0; max-width:46em; }}
nav ol {{ list-style:none; margin:0; padding:0; display:flex; flex-wrap:wrap; gap:6px 8px; }}
nav a {{ display:block; font-size:13.5px; padding:3px 11px; border:1px solid var(--rule); border-radius:3px; background:var(--surface); color:var(--ink); text-decoration:none; }}
nav a:hover {{ border-color:var(--accent); color:var(--accent); }}
article {{ min-width:0; }}
article h2 {{ font-family:var(--serif); font-weight:700; font-size:24px; margin:56px 0 14px; padding-top:18px; border-top:2px solid var(--ink); text-wrap:balance; }}
article h2:first-child {{ margin-top:0; }}
article p {{ margin:12px 0; max-width:46em; }}
article ul, article ol {{ padding-left:1.4em; margin:12px 0; display:flex; flex-direction:column; gap:6px; max-width:52em; }}
code {{ font-family:var(--mono); font-size:.86em; background:var(--code-bg); padding:1px 5px; border-radius:3px; }}
.table-wrap {{ overflow-x:auto; margin:18px 0; border:1px solid var(--rule); border-radius:6px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; font-size:14px; line-height:1.65; }}
th, td {{ text-align:left; vertical-align:top; padding:10px 12px; border-bottom:1px solid var(--rule); min-width:7em; }}
th {{ font-size:13px; color:var(--muted); background:var(--ground); font-weight:700; white-space:nowrap; }}
tr:last-child td {{ border-bottom:0; }}
.steps td:first-child {{ white-space:nowrap; font-weight:700; }}
.steps td:nth-child(3) {{ min-width:22em; }}
.diagram {{ margin:22px 0; background:var(--surface); border:1px solid var(--rule); border-radius:8px; padding:18px 18px 12px; overflow-x:auto; }}
.diagram svg {{ display:block; min-width:760px; }}
.diagram figcaption {{ font-size:13.5px; color:var(--muted); line-height:1.6; margin-top:10px; max-width:60em; }}
.strip {{ list-style:none; margin:16px 0 6px; padding:14px; display:flex; flex-direction:row; max-width:none; gap:8px; overflow-x:auto; background:var(--surface); border:1px solid var(--rule); border-radius:8px; }}
.strip li {{ flex:0 0 auto; min-width:118px; padding:8px 10px; border-radius:6px; border:1px solid var(--rule); display:flex; flex-direction:column; gap:2px; position:relative; }}
.strip li b {{ font-size:13px; }}
.strip li span {{ font-size:12px; color:var(--muted); font-variant-numeric:tabular-nums; }}
.strip li::before {{ content:""; position:absolute; left:10px; right:10px; top:-1px; height:3px; border-radius:0 0 3px 3px; background:var(--rule); }}
.strip li.ok::before {{ background:var(--ok); }}
.strip li.warn {{ background:var(--warn-soft); }} .strip li.warn::before {{ background:var(--warn); }}
.strip li.doing {{ border-color:var(--accent); background:var(--accent-soft); }} .strip li.doing::before {{ background:var(--accent); }}
.strip li.todo {{ opacity:.6; }}
.strip-head {{ display:flex; flex-wrap:wrap; gap:6px 18px; font-size:13px; color:var(--muted); }}
.strip-head b {{ color:var(--ink); }}
.callout {{ border-left:3px solid var(--accent); background:var(--accent-soft); padding:10px 16px; border-radius:0 6px 6px 0; margin:16px 0; max-width:52em; }}
footer {{ font-size:13.5px; color:var(--muted); border-top:1px solid var(--rule); padding-top:16px; }}
@media (max-width:560px) {{ body {{ font-size:15.5px; padding-inline:16px; }} }}
</style>
<div class="page">
<header>
  <div class="eyebrow"><span>营事编集室 · 情报收集员改造</span><span>流程确认稿 · 2026-09-20</span><span class="chip">待确认</span></div>
  <h1>情报收集员工作台<span>完整流程 · 任务驱动或手动开启</span></h1>
  <p class="lede">情报收集员这个角色从 Hermes agent 换成工作台：采集服务持这个角色的 token 接引擎派单，Web 界面把接单之后的每一步都摆出来。流程有两个入口：引擎 05:30 派单驱动，或人在工作台手动开启一轮；之后走同一条线。变的是开工之后按任务类型怎么采集媒体信息（取候选、下载、识别一步完成）、怎么判断，以及人在哪里介入。</p>
</header>
<nav aria-label="目录"><ol>
<li><a href="#s1">1 变了什么</a></li><li><a href="#s2">2 全流程图</a></li><li><a href="#s2-2">2.2 采集器、下载与识别</a></li><li><a href="#s2-3">2.3 任务驱动与手动开启</a></li><li><a href="#s3">3 工作台上的流程带</a></li><li><a href="#s4">4 逐步说明</a></li><li><a href="#s5">5 与 9/20 设计稿的差异</a></li><li><a href="#s6">6 不变的约定</a></li><li><a href="#s7">7 请确认</a></li>
</ol></nav>
<article>
<h2 id="s1">1 变了什么：Hermes agent → 工作台</h2>
<div class="table-wrap"><table><tr><th>项目</th><th>之前（Hermes agent）</th><th>现在（工作台）</th></tr>{changes_rows}</table></div>

<h2 id="s2">2 全流程图</h2>
<p>横向四列是谁在做，纵向是步骤顺序。最上面一行是两个入口：左边引擎派单，右边人在工作台手动开启，都汇到 1 开工。第 2 步「采集媒体信息」是一个整体：框里的 a 取候选、b 下载媒体、c 识别图片视频对每条候选连着做完才算采集完成，不分来源。工作台这一列从 1 接单起一路到 9 补件，都是同一个进程在跑、同一个界面在显示。</p>
{fig_full()}
<h3 id="s2-1">2.1 任务类型决定采集方案</h3>
<p>工作台接到的任务不止 01。先读任务的 <code>stage_code</code>，选对应的采集方案，再决定走哪几步。方案里的采集器都直接调来源接口，不经 csw MCP；不论哪套方案，取到的候选都在同一步里下载媒体并识别。</p>
<div class="table-wrap"><table><tr><th>任务类型</th><th>采集方案（默认）</th><th>取什么</th><th>后续走哪几步</th></tr>
<tr><td>01 情报逐条（intake）</td><td>「日更采集」：csw 贴文窗口 + Van 链接；来源台账里启用的小红书、网页采集器</td><td>窗口内全部贴文与媒体，缺省前一天到当天，周一到上周五；她给的链接按 id 取单条</td><td>3 合并 → 4 查重 → 5 打分 → 6 深核 → 7 交付 → 8 首批校准</td></tr>
<tr><td>05 配图与素材核（material，逐条）</td><td>「条目取单条」：csw 单条</td><td>已批条目的来源贴文与全部媒体</td><td>同一步里下原图并识别 → 核素材与版权 → 逐条交付（0 闸）</td></tr>
<tr><td>11 小红书选图包（xhs_pick，逐条、手动派工）</td><td>「条目取单条」</td><td>同上</td><td>按资讯分组原图 → 交付（0 闸）</td></tr>
<tr><td>补件 / 重开</td><td>「条目取单条」</td><td>已登记条目的来源贴文</td><td>只补缺的图或事实，挂到已通过任务</td></tr>
<tr><td>以后的新任务类型</td><td>新建一套方案</td><td>由方案决定</td><td>由方案决定</td></tr>
</table></div>

<h3 id="s2-2">2.2 采集媒体信息：采集器、下载与识别是一个整体，可自定义</h3>
{fig_collectors()}
<p>三个概念：<b>采集器</b>是一种取法（怎么访问一个来源、怎么翻页、返回什么）；<b>来源</b>是采集器的一个实例，参数在来源台账里（账号、地址、必扫、启停）；<b>采集方案</b>是按任务类型编的一份有序清单，写明跑哪些采集器、各带什么参数、哪些必扫。跑完把结果归一成同一种候选格式再合并，然后不分来源地对每条候选下载媒体、逐个识别，所以后面的合并、查重、打分不关心候选是从哪来的，也不用再碰原站。</p>
<p><b>下载与识别是这一步的一部分，不是后面的步。</b>每条候选的全部媒体都下到本地：图片取 w_768 缩略，05 / 11 取原图；视频取封面与关键帧（按时长等距，最多若干帧）。然后每张图、每段视频的关键帧连同该条正文送视觉模型，结构化输出：对应正文哪一点、画面里的产品 / 场景 / 文字、正文提到但画面没有的、媒体类型。描述随候选保存，候选卡上每个媒体下一行描述。下载或识别失败的媒体标「未识别」，不算看过，不阻塞其他候选；一条候选只有媒体全部处理完（成功或标记）才算采集完成。加新采集器不用管下载与识别，它们对所有来源通用。</p>
<div class="table-wrap"><table><tr><th>内置采集器</th><th>来源</th><th>取法</th><th>参数</th></tr>
<tr><td>csw 贴文窗口</td><td>agent.campsomewhere.com</td><td>GET /api/v1/posts/window，按 has_more 续取</td><td>窗口、是否含隐藏、limit</td></tr>
<tr><td>csw 单条</td><td>agent.campsomewhere.com</td><td>GET /api/v1/posts/{{id}}?include-media=true</td><td>贴文 id 清单（从条目 source_url 解析）</td></tr>
<tr><td>小红书</td><td>opencli，已登录 Chrome</td><td>按账号或关键词受控调用，随机间隔</td><td>账号 / 关键词、条数上限</td></tr>
<tr><td>网页 / RSS</td><td>品牌官网、媒体页</td><td>域名白名单抓取，体积上限</td><td>入口地址、解析规则</td></tr>
<tr><td>Van 链接</td><td>工作台勾选与分享的链接</td><td>解析平台与 id 后交给对应采集器</td><td>——</td></tr>
</table></div>
<p><b>自定义有三层，都不改主流程：</b></p>
<ol>
<li><b>改方案</b>：在工作台「运行与设置」里增删采集器、调顺序、改参数、标必扫或停用；每次改动存一个版本，记谁改的、从哪一期生效；可用历史窗口回放验证。这一层覆盖「加一个 Instagram 账号」「多扫一个网站」「某来源暂停」。</li>
<li><b>加适配器</b>：新取法不用改采集服务代码。写一个外部命令或一个 HTTP 端点，遵守同一份输入输出契约（输入：任务、窗口、参数；输出：统一候选格式的清单、计数、错误），在方案里以「自定义采集器」登记路径或地址即可。引擎的数据子系统已经用同样的外部命令方式接平台读取，沿用它。</li>
<li><b>挂 MCP 工具</b>：指定 MCP 服务器与工具名，加一段字段映射（哪个字段是 id、链接、正文、时间、媒体），采集服务以 MCP 客户端身份直接调工具、不经模型。适合来源方已经给了 MCP 的情形。</li>
</ol>
<p><b>叠加的规则</b>：多个采集器同时跑，结果按（来源平台，外部 id）去重；同一候选被几个采集器找到就记几条 discovered_via，来源之间不加权、不设配额。必扫采集器失败让本步标失败并告警，非必扫的失败只记采集轮继续。</p>
<p><b>统一候选格式</b>（每个采集器都得给到这些字段）：来源平台、来源键、外部 id、链接、账号、正文原文与语言、发布时间、抓取时间、媒体清单（类型、地址、哈希）、原始返回。缺发布时间或媒体的照收，但标出来。</p>

<h3 id="s2-3">2.3 两种开工方式：任务驱动与手动开启</h3>
<p>同一条流程，两个入口。任务驱动是日常：引擎派什么就做什么，结果写回引擎。手动开启是人要用这套流程做点别的：回放、验证、试新来源、临时补采；不关联任务时只在本地，关联了就和任务驱动一样交付。</p>
<div class="table-wrap"><table><tr><th>项目</th><th>任务驱动</th><th>手动开启</th></tr>
<tr><td>谁发起</td><td>引擎定时触发，或主编在管理后台触发</td><td>主编、研究员、开发在工作台点「开始一轮」</td></tr>
<tr><td>参数来自</td><td>任务：stage_code、run 的窗口、作业标准与自检</td><td>人选：任务类型或采集方案、窗口、采集器参数、是否关联某个已接单的任务</td></tr>
<tr><td>走哪几步</td><td>1 到 9 全走</td><td>2 到 6 全走；7 到 9 只在关联任务时走</td></tr>
<tr><td>写不写引擎</td><td>写：条目、采集轮、打分台账、交付物</td><td>不关联：不写，只有本地台账与报告。关联：与任务驱动相同，以补件或产出提交</td></tr>
<tr><td>群播报</td><td>引擎单写</td><td>无；关联任务并提交后由引擎照旧播报</td></tr>
<tr><td>典型用途</td><td>每天 05:30 那一期；05 / 11 逐条任务</td><td>并行验证期只出报告不写引擎；回放历史窗口调权重与准则；试新采集器或新来源；白天临时补采 Van 给的链接；某一步失败后只重跑那一步</td></tr>
<tr><td>审计</td><td>引擎的任务事件</td><td>发起人、参数、时间写审计；本地台账标「手动」，与任务驱动的轮次分开列</td></tr>
<tr><td>与任务的关系</td><td>一轮对一个任务</td><td>可以不关联；也可以先跑后关联，把挑出的结果以补件挂到任务上</td></tr>
</table></div>

<h2 id="s3">3 工作台上的流程带</h2>
<p>每个页面顶部都有这一条，主编打开工作台第一眼看到本轮走到哪一步、是任务驱动还是手动开启。每格显示状态、计数、耗时；点开看这一步的产物、失败清单与重跑按钮。下面的数字是示意，取自 9/18 那一期的量级。</p>
<div class="strip-head"><span>开工方式 <b>任务驱动</b></span><span>期次 <b>r48</b></span><span>任务 <b>#611</b></span><span>窗口 <b>09-17 → 09-18（UTC）</b></span><span>距首批时限 <b>12 分钟</b></span></div>
<ol class="strip">{strip_html}</ol>
<p class="callout">绿条已完成，黄底有需要人看的失败，强调色是正在跑的一步，灰的未开始。每一步的计数都由代码算，不由模型报。</p>

<h2 id="s4">4 逐步说明</h2>
<div class="table-wrap"><table class="steps"><tr><th>步</th><th>谁</th><th>做什么</th><th>产出</th><th>工作台上看到</th><th>人能做什么</th><th>失败时</th></tr>{rows}</table></div>

<h2 id="s5">5 与 9/20 设计稿的差异</h2>
<ol>{diff_html}</ol>

<h2 id="s6">6 不变的约定</h2>
<ul>{keep_html}</ul>

<h2 id="s7">7 请确认</h2>
<ol>{confirm_html}</ol>
</article>
<footer>配套：《情报收集员工作台 · 设计说明》《情报收集员工作台 · 界面设计稿》《情报收集员改造方案》概要版与开发版。确认后按第 7 节第 5 条回改。</footer>
</div>'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "figs", page.count("<figure"))
