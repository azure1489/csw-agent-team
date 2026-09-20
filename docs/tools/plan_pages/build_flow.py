# -*- coding: utf-8 -*-
"""情报收集员工作台 · 完整流程（派单为起点）确认页。用法：python build_flow.py out.html"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from diagrams import box, arrow, svg, fig, esc, ACC

out = sys.argv[1]

# ---------- 图 A：全流程（纵向，行 = 步，列 = 谁在做） ----------
def fig_full():
    W = 1180
    lanes = [("引擎（真相）", 40, 260), ("情报收集员工作台（采集服务 + Web）", 300, 340), ("csw-agent · 模型", 640, 310), ("人", 950, 190)]
    y0, RH, bh = 56, 66, 44
    b = ""
    for t, x, w in lanes:
        b += f'<rect x="{x}" y="14" width="{w}" height="{y0 - 22 + 14 * RH + 8}" fill="none" stroke="currentColor" stroke-opacity=".22"/>'
        b += f'<text x="{x + w / 2}" y="36" text-anchor="middle" font-size="12.5" font-weight="700" fill="currentColor" opacity=".75">{esc(t)}</text>'
    pos = {}
    def rb(lane, row, title, sub=None, accent=False, dashed=False, w=None):
        _, lx, lw = lanes[lane]
        bw = w or {0: 170, 1: 240, 2: 220, 3: 130}[lane]
        x = lx + (lw - bw) / 2
        y = y0 + row * RH
        pos[(lane, row)] = (x, y, bw)
        return box(x, y, bw, bh, title, sub, accent=accent, dashed=dashed, fs=12.5)
    def L(lane, row): x, y, w = pos[(lane, row)]; return x, y + bh / 2
    def R(lane, row): x, y, w = pos[(lane, row)]; return x + w, y + bh / 2
    def T(lane, row): x, y, w = pos[(lane, row)]; return x + w / 2, y
    def B(lane, row): x, y, w = pos[(lane, row)]; return x + w / 2, y + bh
    def cy(row): return y0 + row * RH + bh / 2

    # 行 0：派单
    b += rb(0, 0, "0 派单 05:30", "引擎触发 run · 01 派给收集员")
    # 行 1：接单
    b += rb(1, 1, "1 接单", "查 my-tasks · ack · 心跳 · 读作业标准")
    b += rb(0, 1, "任务：已接单", "派工单 · 作业标准 · 首批时限")
    x, y = R(0, 0); tx, ty = T(1, 1)
    b += arrow(x, y, tx, ty, "派工单 · 群播报", via=[(tx, y)], lx=x + 8, ly=y - 8, anchor="start")
    x, y = L(1, 1); x2, y2 = R(0, 1)
    b += arrow(x, y, x2, y2, "ack · 心跳", lx=(x + x2) / 2, ly=y - 8)
    # 行 2：取贴文
    b += rb(1, 2, "2 取贴文（按任务类型）", "01 → 窗口全量 · 05 / 11 → 按条目取单条", accent=True)
    b += rb(2, 2, "agent.campsomewhere.com", "posts/window · posts/{id}")
    x, y = L(2, 2); x2, y2 = R(1, 2)
    b += arrow(x, y, x2, y2, "贴文 + 媒体", lx=(x + x2) / 2, ly=y - 8)
    # 行 3：下图
    b += rb(1, 3, "3 下图", "全部媒体 · 视频取封面 · 失败标记")
    b += rb(2, 3, "OSS 图片", "缩略 w_768 · 直链")
    x, y = L(2, 3); x2, y2 = R(1, 3)
    b += arrow(x, y, x2, y2, "缩略图", lx=(x + x2) / 2, ly=y - 8)
    # 行 4：图片描述
    b += rb(1, 4, "4 图片描述", "每张图 × 该条正文 · 存描述", accent=True)
    b += rb(2, 4, "模型", "对应正文哪点 · 画面 · 图上没有的")
    x, y = L(2, 4); x2, y2 = R(1, 4)
    b += arrow(x, y, x2, y2, "逐图描述", accent=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 5：合并
    b += rb(1, 5, "5 合并与窗口", "贴文 → 事件 · 窗口外标注不删")
    b += rb(2, 5, "模型", "是否同一事件", dashed=True)
    x, y = L(2, 5); x2, y2 = R(1, 5)
    b += arrow(x, y, x2, y2, "判定", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 6：查重
    b += rb(1, 6, "6 查重", "有知识库则查 · 否则标「未查」")
    b += rb(0, 6, "知识库 · 选题记忆", "正式已发布 · 案例与准则", dashed=True)
    b += rb(2, 6, "模型", "同一事实？同一角度？", dashed=True)
    x, y = R(0, 6); x2, y2 = L(1, 6)
    b += arrow(x, y, x2, y2, "旧文 · 案例", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 6); x2, y2 = R(1, 6)
    b += arrow(x, y, x2, y2, "结论", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 7：打分
    b += rb(1, 7, "7 打分", "正文 + 图描述 + 实图 · 每事件一条")
    b += rb(2, 7, "模型", "六维度 · 依据 · 三句话 · 缺口")
    x, y = L(2, 7); x2, y2 = R(1, 7)
    b += arrow(x, y, x2, y2, "打分结果", lx=(x + x2) / 2, ly=y - 8)
    # 行 8：深核
    b += rb(1, 8, "8 深核", "入围 + 待核 · 可打断 · 线程落盘")
    b += rb(2, 8, "Codex App Server", "只读工具 · 完整图 · 原始来源")
    x, y = L(2, 8); x2, y2 = R(1, 8)
    b += arrow(x, y, x2, y2, "条目卡", lx=(x + x2) / 2, ly=y - 8)
    # 行 9：登记与交付
    b += rb(1, 9, "9 登记与交付", "intake-check · zip · submit")
    b += rb(0, 9, "引擎落库", "条目 · 采集轮 · 台账 · 交付物")
    x, y = L(1, 9); x2, y2 = R(0, 9)
    b += arrow(x, y, x2, y2, "PUT · submit", lx=(x + x2) / 2, ly=y - 8)
    # 行 10：首批校准
    b += rb(0, 10, "10 首批校准闸", "待主编审 · 群播报")
    b += rb(3, 10, "主编", "看台账 · 通过 / 退回")
    x, y = R(0, 10); x2, y2 = L(3, 10)
    b += arrow(x, y - 8, x2, y2 - 8, "待审", lx=(x + x2) / 2, ly=y - 16)
    b += arrow(x2, y2 + 8, x, y + 8, "通过 / 退回（方向 + 位置）", lx=(x + x2) / 2, ly=y + 24)
    # 行 11：补件
    b += rb(1, 11, "11 返工或补件", "退回则返工 · 通过后其余候选以补件追加")
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
    # 决定回流记忆（虚线，沿左侧边距回到行 6）
    x, y = L(0, 12); x2, y2 = L(0, 6)
    b += arrow(x, y + 10, x2, y2 + 10, dashed=True, accent=True, via=[(16, y + 10), (16, y2 + 10)])
    b += f'<text x="30" y="{(y + y2) / 2 + 4}" font-size="11" fill="{ACC}" transform="rotate(-90 30 {(y + y2) / 2 + 4})" text-anchor="middle">决定 + 维度分入记忆，下一期用</text>'
    # 行 13：05
    b += rb(0, 13, "05 配图 · 11 选图包", "逐条派工 · 同起点")
    b += rb(1, 13, "05 / 11 接单", "按条目取单条 · 下原图 · 逐条交付")
    b += rb(2, 13, "agent.campsomewhere.com", "GET posts/{id}?include-media")
    x, y = R(0, 13); x2, y2 = L(1, 13)
    b += arrow(x, y, x2, y2, "派工单", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 13); x2, y2 = R(1, 13)
    b += arrow(x, y, x2, y2, "单条 + 媒体", lx=(x + x2) / 2, ly=y - 8)
    H = y0 + 14 * RH + 8
    return fig(svg(W, H, b, "完整流程：引擎派单起点，工作台接单后取贴文、下图、逐图描述、合并、查重、打分、深核、登记交付，主编首批校准，补件，下游与 Van 决定回流"),
               "图 A · 完整流程。每一行是一步，列是谁在做。工作台这一列是主线：0 派单之后的每一步都在它上面可见。强调色标出与 9/20 设计稿不同的两处：取贴文直接调 csw-agent 接口并按任务类型决定取法，图片先逐图按正文生成描述。虚线是「有则用」：知识库与记忆缺时这一步照跑，只是结果标「未查」。")

# ---------- 流程带示意（HTML） ----------
STRIP = [
    ("0 派单", "05:30", "ok"), ("1 接单", "05:30 · 心跳中", "ok"), ("2 取贴文", "438 条 · 2.1 秒", "ok"),
    ("3 下图", "2,311 / 2,361 · 50 失败", "warn"), ("4 图片描述", "2,311 张 · 6 分", "ok"), ("5 合并", "96 事件 · 窗口外 12", "ok"),
    ("6 查重", "96 · 重复 9", "ok"), ("7 打分", "96 / 96 · 18 分", "ok"), ("8 深核", "6 / 24", "doing"),
    ("9 登记与交付", "未开始", "todo"), ("10 主编校准", "未开始", "todo"), ("11 补件", "未开始", "todo"),
]
strip_html = "".join(f'<li class="{c}"><b>{esc(t)}</b><span>{esc(s)}</span></li>' for t, s, c in STRIP)

# ---------- 逐步说明 ----------
STEPS = [
    ("0 派单", "引擎", "05:30 定时触发 run（周一窗口到上周五）；01 任务派给「情报收集员」角色；群播报由引擎单写。", "run · 01 任务 · 派工单（editor_note + upstreams）", "总览出现新期次、任务号、派工单、作业标准与自检、首批时限倒计时。", "主编可在管理后台照旧手动触发。", "触发失败照旧走引擎告警，与工作台无关。"),
    ("1 接单", "工作台（采集服务）", "每 30 秒查 my-tasks；发现 open 的 01 任务即 ack 并开始心跳；从 GET /tasks/:id 读作业内容、自检、验收，原样显示并注入后面的打分前言。", "任务状态：已接单", "流程带全部步置「待开始」；作业标准可读。", "主编可「中止本轮」或「重新开始」（记录人与理由）。", "ack 失败重试；超过 ack 时限由引擎升级中枢，照旧。"),
    ("2 取贴文", "工作台 → csw-agent", "先看任务的 stage_code，再决定取法（见 2.1 的表）。01 情报逐条：GET /api/v1/posts/window，缺省前一天到当天，周一 start 取上周五，include_media=true，limit=2000，按 has_more 用 offset 续取。05 / 11 逐条任务：从条目的 source_url 解析贴文 id，GET /api/v1/posts/{id}?include-media=true 取单条。Van 给的链接同样按 id 取单条并入候选，origin=van_link。都是直接 HTTP，不经 csw MCP；带重试与长超时（空闲后首个请求实测可到 40 秒）。存本地 posts 表，按 post_id 去重。来源台账里的小红书、网页源有则跑，无则记「未扫」。", "本地贴文；每次调用一条采集轮（found / in_window 由代码算）", "任务类型与取法、贴文数、与接口 total 是否一致、耗时、失败与重试、各来源状态。", "主编可改窗口重取（记审计）。", "接口失败：重试三次仍失败则本步标失败、流程停在这里并告警；不用旧接口兜底。"),
    ("3 下图", "工作台 → OSS", "每条贴文的全部媒体：图片用 ?x-oss-process=image/resize,w_768 下缩略；视频取封面；本地 media/ 按 media_hash 存；失败记原因。", "本地缩略图；下载记录", "下载成功率、读不到图的贴文清单、原因。", "重试下载。", "单张失败不阻塞；整体失败率超阈值告警。"),
    ("4 图片描述", "工作台 → 模型", "每张图连同该条正文（原文 + 译文若有）送模型，结构化输出：这张图对应正文哪一点、画面里的产品 / 场景 / 文字、正文提到但图上没有的、图片类型（产品图 / 海报 / 场景 / 截图 / 无关）。存 image_descriptions（post_id、media_hash、描述、模型与提示词版本）。", "每张图一条描述；未描述的标记原因", "贴文卡每张图下一行描述；详情页逐图对照正文；未描述的标记。", "主编可对某张图重跑或手改描述（留痕）。", "单张失败重试一次，仍失败标「未描述」，不算看过图。"),
    ("5 合并与窗口", "工作台（代码为主）", "按 date_posted 精确判窗口；同账号、同文案开头先粗并；模型判是否同一事件。窗口外的标「硬性排除 · 窗口外」但保留。", "events 表；事件 ↔ 贴文映射", "事件数、每个事件合并了哪几条、窗口外分组。", "拆开或合并事件（留痕）。", "模型判定失败时按粗并结果继续，标「未经模型判定」。"),
    ("6 查重", "工作台 + 知识库 + 模型", "代码按品牌、产品、别名到知识库找候选，只对照「正式已发布」；模型判是否同一事实、同一角度。知识库未同步到当天的如实标「已同步到 X 日」；没有知识库就标「未查」，不阻塞。", "每个事件的查重结论与对照旧文", "事件卡的查重标记与对照旧文；知识库同步到哪天。", "改判查重结论（留痕）。", "检索失败标「未查」继续。"),
    ("7 打分", "工作台 → 模型", "固定前言 = 任务下发的作业标准 + 判断框架与各级锚点 +（有则）本批品牌的知识库命中与案例；输入 = 正文 + 第 4 步的图片描述 + 实图缩略；每批若干事件、每批新会话、并行。输出六维度分、依据、三句话、缺口、优先 / 降低命中、是否看到实图。", "打分台账：每个事件一行", "台账全量；综合分与六维度；权重版本即时重排；缺口。", "捞回、改判、标待核、加入首批（都留痕）。", "某批失败只重跑该批；重试后仍失败的事件标「未打分」并计数。"),
    ("8 深核", "工作台 → Codex App Server", "排名前 K 与待核的事件各起一个线程：核原始披露时间、找原始来源核事实、核比较参照是否准确、看完整图片；工具只挂只读白名单；线程事件流落盘。", "条目卡：披露时间、证据链接、查重说明、事实、配图、缺口", "详情页深核结果与证据；线程记录可下载；进度 K 之几。", "打断某个线程、改 K、对单个事件重跑。", "单线程超时或失败标「深核未完成」，条目仍可登记并写明缺口。"),
    ("9 登记与交付", "工作台 → 引擎", "PUT items（带 origin）、PUT sweeps、PUT intake-scores；生成 index.md + images/ + trace/ 打 zip；提交前本地跑 intake-check；submit（kind=产出）。登记为条目的是排名靠前的、待核的、Van 链接的；其余只在台账。", "引擎里的条目、采集轮、打分台账、交付物；任务状态：待审", "校验结果逐条、提交状态、交付物链接。", "主编可在提交前先看一眼台账并调整首批（可选，不改闸的位置）。", "intake-check 不过则不提交，列出不过项；submit 失败重试，幂等键固定。"),
    ("10 首批校准", "引擎闸 · 主编", "引擎群播报「待审」；主编在工作台看台账、图与描述、缺口，回引擎「通过」或「退回」（退回必填方向与位置）。", "任务：已交付 / 需返工", "闸状态、退回原因。", "通过、退回、定点意见。", "超时由引擎照旧提醒。"),
    ("11 返工或补件", "工作台 → 引擎", "退回：按 latest_review 的方向与位置返工，重新交付。通过：其余候选与后续补证以 kind=补件 追加到已通过任务，只让已开工的直接下游返工。", "补件记录", "补件清单与状态。", "主编触发「追加补件」。", "同 9。"),
    ("下游", "引擎 · 研究员 · 主编 · Van", "02 研究员读台账与 intake-trace 评估；03 主编合成选题方案；Van 在工作台 Van 模式勾选并留原话，主编代录 03 闸决定。决定连同当时的维度分入选题记忆，下一期第 6、7 步读得到。", "03 决定；记忆案例", "Van 模式；每条决定与原话；指标页三项。", "Van：采用 / 否决 / 暂缓 + 原话。", "——"),
    ("05 / 11", "引擎 → 同一工作台", "05 配图与素材核（material，逐条自动派工）与 11 小红书选图包（xhs_pick，逐条手动派工）都派给收集员角色，工作台以同样方式接单；按任务类型走单条取法：从条目 source_url 取该贴文与媒体，下载原图（不缩略）、核素材与版权、逐条交付（0 闸）。不走描述、打分、深核。", "逐条交付物", "05 / 11 任务清单与进度、每条的原图与核验结果。", "重跑某条。", "同 1 至 3。"),
]
rows = "".join("<tr>" + "".join(f"<td>{esc(c)}</td>" for c in r) + "</tr>" for r in STEPS)

CHANGES = [
    ("身份", "Hermes agent 上的一个 profile，被飞书 @ 唤醒", "采集服务持「情报收集员」角色的 token，30 秒轮询 my-tasks 接单；工作台前端是它的界面"),
    ("能看到什么", "群里的几条消息与最后的 zip", "整条流程带：每一步的状态、计数、耗时、失败；每一步的产物都能点开"),
    ("取贴文", "模型调 csw MCP 搜索，一次最多 100 条", "代码按任务类型直接调 csw-agent 接口：01 取窗口全量，05 / 11 按条目取单条；数量由代码算"),
    ("图片", "不看图", "每张图先按正文生成描述并保存；打分时再看实图"),
    ("判断", "模型自由挑选", "按 Van 的判断框架六维度打分、写依据与三句话；不因口味淘汰"),
    ("过程", "只在群聊里", "采集轮、台账、trace 都在引擎；人的每次操作留痕"),
    ("人的介入", "在群里 @ 收集员", "在工作台捞回、改判、重跑某步、首批校准"),
    ("交付", "手写 index.md", "代码生成 index.md、images/、trace/，提交前跑 intake-check"),
    ("05 配图", "同一个 Hermes profile", "同一个工作台接单"),
]
changes_rows = "".join(f"<tr><td>{esc(a)}</td><td>{esc(b)}</td><td>{esc(c)}</td></tr>" for a, b, c in CHANGES)

DIFF = [
    "起点写清楚了：0 派单 → 1 接单，之前的设计图从「取全量」起画，看不出接单在前。",
    "取贴文直接调 csw-agent 的 HTTP 接口，不经 csw MCP，并按任务类型决定取法：01 取窗口全量，05 / 11 与 Van 链接按条目取单条；MCP 只留给深核线程按需查。",
    "新增第 4 步「图片描述」：每张图按该条正文生成描述并保存，随贴文显示；之前只在打分时顺带看缩略图。",
    "知识库与选题记忆改为「有则用」：第 6、7 步缺它们照跑，结果标「未查」；不再是开工前置。",
    "整条流程做成工作台顶部的流程带，每一步可点开看产物、失败与重跑；之前总览只有一条进度条。",
    "05 配图与素材核明确为同一工作台接单，同一起点。",
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
    "图片描述的粒度与去向：每张图一条（建议），存采集服务本地并随台账显示；是否同时回写 csw 库的 detailed_alt，请定。",
    "第 2 步接口失败时停在本步告警、不用旧接口兜底，是否同意。",
    "第 9 步提交前是否允许主编先在工作台调整首批再提交（可选项，不改闸的位置）。",
    "Van 模式的勾选是否直接等同 03 闸决定，还是只记「Van 选择」由主编代录（沿用设计说明第 10 节的问题）。",
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
  <h1>情报收集员工作台<span>完整流程 · 以引擎派单为起点</span></h1>
  <p class="lede">情报收集员这个角色从 Hermes agent 换成工作台：采集服务持这个角色的 token 接引擎派单，Web 界面把接单之后的每一步都摆出来。流程起点不变，仍是引擎 05:30 派工；变的是接单之后按任务类型怎么取贴文、怎么看图、怎么判断，以及人在哪里介入。</p>
</header>
<nav aria-label="目录"><ol>
<li><a href="#s1">1 变了什么</a></li><li><a href="#s2">2 全流程图</a></li><li><a href="#s3">3 工作台上的流程带</a></li><li><a href="#s4">4 逐步说明</a></li><li><a href="#s5">5 与 9/20 设计稿的差异</a></li><li><a href="#s6">6 不变的约定</a></li><li><a href="#s7">7 请确认</a></li>
</ol></nav>
<article>
<h2 id="s1">1 变了什么：Hermes agent → 工作台</h2>
<div class="table-wrap"><table><tr><th>项目</th><th>之前（Hermes agent）</th><th>现在（工作台）</th></tr>{changes_rows}</table></div>

<h2 id="s2">2 全流程图</h2>
<p>横向四列是谁在做，纵向十四行是步骤顺序。工作台这一列从 1 接单起一路到 11 补件，都是同一个进程在跑、同一个界面在显示。</p>
{fig_full()}
<h3 id="s2-1">2.1 取贴文按任务类型</h3>
<p>工作台接到的任务不止 01。先读任务的 <code>stage_code</code>，再决定取什么、走哪几步。都是直接调 csw-agent 的 HTTP 接口，不经 csw MCP。</p>
<div class="table-wrap"><table><tr><th>任务类型</th><th>取什么</th><th>接口</th><th>后续走哪几步</th></tr>
<tr><td>01 情报逐条（intake）</td><td>窗口内全部贴文与媒体列表；缺省前一天到当天，周一到上周五</td><td><code>GET /api/v1/posts/window</code></td><td>3 下图 → 4 图片描述 → … → 9 交付 → 10 首批校准</td></tr>
<tr><td>01 里 Van 给的链接</td><td>她指定的那条贴文</td><td><code>GET /api/v1/posts/{{id}}?include-media=true</code></td><td>并入候选，origin=van_link，单独统计，不计自主发现</td></tr>
<tr><td>05 配图与素材核（material，逐条）</td><td>已批条目的来源贴文与全部媒体</td><td>同上</td><td>下原图（不缩略）→ 核素材与版权 → 逐条交付（0 闸）</td></tr>
<tr><td>11 小红书选图包（xhs_pick，逐条、手动派工）</td><td>同上</td><td>同上</td><td>按资讯分组原图 → 交付（0 闸）</td></tr>
<tr><td>补件 / 重开</td><td>已登记条目的来源贴文</td><td>同上</td><td>只补缺的图或事实，挂到已通过任务</td></tr>
</table></div>

<h2 id="s3">3 工作台上的流程带</h2>
<p>每个页面顶部都有这一条，主编打开工作台第一眼看到本期走到哪一步。每格显示状态、计数、耗时；点开看这一步的产物、失败清单与重跑按钮。下面的数字是示意，取自 9/18 那一期的量级。</p>
<div class="strip-head"><span>期次 <b>r48</b></span><span>任务 <b>#611</b></span><span>窗口 <b>09-17 → 09-18（UTC）</b></span><span>距首批时限 <b>12 分钟</b></span></div>
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
