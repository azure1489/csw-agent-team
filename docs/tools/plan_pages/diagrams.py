# -*- coding: utf-8 -*-
"""流程设计图：用代码排版生成内联 SVG（currentColor 随主题，单一强调色）。"""
ACC = "#4F46E5"

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

def box(x, y, w, h, title, sub=None, accent=False, dashed=False, fs=13):
    stroke = ACC if accent else "currentColor"
    sw = 2 if accent else 1.2
    dash = ' stroke-dasharray="6 4"' if dashed else ""
    t = f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="var(--surface,#fff)" stroke="{stroke}" stroke-width="{sw}"{dash}/>'
    cx = x + w / 2
    if sub:
        t += f'<text x="{cx}" y="{y + h/2 - 3}" text-anchor="middle" font-size="{fs}" font-weight="600" fill="currentColor">{esc(title)}</text>'
        t += f'<text x="{cx}" y="{y + h/2 + 13}" text-anchor="middle" font-size="11" fill="currentColor" opacity=".75">{esc(sub)}</text>'
    else:
        t += f'<text x="{cx}" y="{y + h/2 + 5}" text-anchor="middle" font-size="{fs}" font-weight="600" fill="currentColor">{esc(title)}</text>'
    return t

def arrow(x1, y1, x2, y2, label=None, accent=False, dashed=False, lx=None, ly=None, anchor="middle", via=None):
    stroke = ACC if accent else "currentColor"
    dash = ' stroke-dasharray="5 4"' if dashed else ""
    pts = f"{x1},{y1} " + (" ".join(f"{px},{py}" for px, py in via) + " " if via else "") + f"{x2},{y2}"
    t = f'<polyline points="{pts}" fill="none" stroke="{stroke}" stroke-width="1.4"{dash} marker-end="url(#{"arrA" if accent else "arr"})"/>'
    if label:
        if lx is None:
            if via:
                mx, my = via[0]
                lx, ly = (mx + x2) / 2 if len(via) == 1 and my == y2 else mx, my - 6
            else:
                lx, ly = (x1 + x2) / 2, (y1 + y2) / 2 - 6
        t += f'<text x="{lx}" y="{ly}" text-anchor="{anchor}" font-size="11" fill="{stroke}">{esc(label)}</text>'
    return t

def lane(x, y, w, h, title):
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="none" stroke="currentColor" stroke-opacity=".25" stroke-width="1"/>'
            f'<text x="{x+12}" y="{y+18}" font-size="12" font-weight="700" fill="currentColor" opacity=".7">{esc(title)}</text>')

def svg(w, h, body, label):
    defs = ('<defs><marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">'
            '<path d="M0,0 L10,5 L0,10 z" fill="currentColor"/></marker>'
            f'<marker id="arrA" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">'
            f'<path d="M0,0 L10,5 L0,10 z" fill="{ACC}"/></marker></defs>')
    return (f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="{esc(label)}" style="max-width:100%;height:auto;font-family:inherit">'
            + defs + body + "</svg>")

def fig(inner, caption):
    return f'<figure class="diagram">{inner}<figcaption>{caption}</figcaption></figure>'

# ---------- 图 1：系统位置 ----------
def fig_system():
    b = ""
    b += box(30, 40, 170, 56, "工作台前端", "浏览器 · React")
    b += box(30, 150, 170, 56, "管理后台", "定义 · 成员 · 监控（不变）")
    b += box(300, 20, 360, 230, "", None)  # 容器
    b += f'<text x="480" y="40" text-anchor="middle" font-size="12" font-weight="700" fill="currentColor" opacity=".7">采集服务（Rust，agent 主机）</text>'
    b += box(320, 52, 140, 44, "采集媒体信息", "采集器 · 下载 · 识别")
    b += box(500, 52, 140, 44, "合并 · 对照材料", "窗口 · 召回 · 重排")
    b += box(320, 108, 140, 44, "逐条判断", "正文 + 图 + 对照", accent=True)
    b += box(500, 108, 140, 44, "深核", "Codex App Server")
    b += box(320, 164, 140, 44, "工作台接口", "台账 · 线程记录")
    b += box(500, 164, 140, 44, "LanceDB", "知识库索引 · 候选 · 图片")
    b += box(740, 40, 190, 56, "任务流转引擎", "任务 · 条目 · 判断台账 · 交付物")
    b += box(740, 150, 190, 56, "刊发记录 · 决定 · 记忆", "引擎内 · 真相")
    b += box(300, 300, 130, 50, "csw 后端", "贴文库 · OSS 图片")
    b += box(445, 300, 130, 50, "sub2api", "gpt-6-astra")
    b += box(590, 300, 130, 50, "Qwen3-VL", "向量 · 重排")
    b += box(735, 300, 130, 50, "opencli", "小红书 · 网页")
    b += arrow(200, 68, 300, 68, "JWT", lx=250, ly=60)
    b += arrow(30, 178, 930, 68, "读写定义与监控（不变）", dashed=True, via=[(18, 178), (18, 380), (945, 380), (945, 68)], lx=480, ly=374)
    b += arrow(660, 74, 740, 68, "任务 · 台账 · 交付物", lx=700, ly=52)
    b += arrow(740, 178, 660, 186, "同步为索引", lx=700, ly=172)
    b += arrow(365, 300, 365, 250, "posts/window", lx=373, ly=278, anchor="start")
    b += arrow(510, 300, 510, 250, "Responses", lx=518, ly=278, anchor="start")
    b += arrow(655, 300, 655, 250, "向量 · 重排", lx=647, ly=278, anchor="end")
    b += arrow(800, 300, 660, 240, "受控调用", via=[(800, 268), (680, 268), (680, 240)], lx=740, ly=262)
    return fig(svg(960, 400, b, "系统位置：工作台只经采集服务访问引擎与外部服务；采集服务用 Rust，内嵌 LanceDB，向量与重排调 Qwen3-VL"),
               "图 1 · 系统位置。工作台前端只和采集服务说话；采集服务用 Rust 写，内嵌 LanceDB 存知识库索引与候选，对外接 csw 后端、模型服务、Qwen3-VL 向量与重排服务、opencli，对内读写引擎；刊发记录、决定与记忆的真相在引擎，同步进 LanceDB 作索引。管理后台保持只管定义与监控。")

# ---------- 图 2：每日流程泳道 ----------
def fig_flow():
    W = 1170; laneH = 96
    lanes = ["来源", "采集服务（Rust · LanceDB）", "模型 · 向量服务", "引擎（真相）", "人：主编 · 研究员 · Van"]
    b = ""
    for i, t in enumerate(lanes):
        b += lane(10, 20 + i * laneH, W - 20, laneH, t)
    Y = [20 + i * laneH + 34 for i in range(5)]
    X = [90 + i * 140 for i in range(8)]
    bw, bh = 124, 44
    def bx(col, l, t, sub=None, **kw): return box(X[col] - bw / 2, Y[l], bw, bh, t, sub, fs=12, **kw)
    C = lambda l: Y[l] + bh / 2
    # 来源
    b += bx(2, 0, "csw 后端 · OSS", "posts/window · 只图文")
    b += bx(3, 0, "csw 后台", "生成过文章的贴文")
    # 采集服务
    b += bx(1, 1, "1 开工", "ack · 心跳 / 手动")
    b += bx(2, 1, "2 采集媒体信息", "候选 · 下载 · 识别 · 向量", accent=True)
    b += bx(3, 1, "3 合并 · 4 对照", "窗口 · 召回 · 重排 · 五类")
    b += bx(4, 1, "5 逐条判断", "每条都判 · 结论档", accent=True)
    b += bx(5, 1, "6 深核", "推荐 + 待核 · 时间盒")
    b += bx(6, 1, "7 登记与交付", "有任务才写引擎")
    b += bx(7, 1, "9 返工或补件", "每期合并一次")
    # 模型
    b += bx(2, 2, "视觉模型", "每张图 × 正文")
    b += bx(3, 2, "Qwen3-VL", "向量 · 重排")
    b += bx(4, 2, "判断模型", "逐维依据 · 三句话")
    b += bx(5, 2, "Codex App Server", "线程 · 只读工具")
    # 引擎
    b += bx(0, 3, "0 派单 05:30", "run · 01 任务")
    b += bx(3, 3, "刊发记录 · 决定", "记忆 · 同步为索引")
    b += bx(6, 3, "条目 · 台账 · 交付", "intake-check")
    b += bx(7, 3, "8 首批校准闸", "→ 02 · 03 · Van 决定")
    # 人
    b += bx(0, 4, "0 手动开启", "方案 · 窗口 · 关联任务")
    b += bx(7, 4, "主编 · Van", "通过 / 退回 · 勾选")
    # 两个入口汇到 1 开工
    jx = X[1] - bw / 2 - 16
    b += arrow(X[0] + bw / 2, C(3), X[1] - bw / 2, C(1), "派工单", via=[(jx, C(3)), (jx, C(1))], lx=jx - 6, ly=C(2) + 4, anchor="end")
    b += arrow(X[0] + bw / 2, C(4), jx, C(3) + 6, "点开始", via=[(jx, C(4))], lx=(X[0] + bw / 2 + jx) / 2, ly=C(4) - 6)
    # 采集主线
    for c in range(1, 6):
        b += arrow(X[c] + bw / 2, C(1), X[c + 1] - bw / 2, C(1))
    # 来源 / 模型 ↔ 采集
    b += arrow(X[2], Y[0] + bh, X[2], Y[1], "贴文 + 图", lx=X[2] + 6, ly=Y[0] + bh + 14, anchor="start")
    b += arrow(X[3], Y[0] + bh, X[3], Y[1], "已生成贴文", lx=X[3] + 6, ly=Y[0] + bh + 14, anchor="start")
    b += arrow(X[2], Y[2], X[2], Y[1] + bh, "描述 · 用途", lx=X[2] + 6, ly=Y[1] + bh + 14, anchor="start")
    b += arrow(X[3], Y[2], X[3], Y[1] + bh, "向量 · 精排", lx=X[3] + 6, ly=Y[1] + bh + 14, anchor="start")
    b += arrow(X[4], Y[2], X[4], Y[1] + bh, "判断结果", accent=True, lx=X[4] + 6, ly=Y[1] + bh + 14, anchor="start")
    b += arrow(X[5], Y[2], X[5], Y[1] + bh, "条目卡", lx=X[5] + 6, ly=Y[1] + bh + 14, anchor="start")
    # 引擎 ↔ 其余
    b += arrow(X[3], Y[3], X[3], Y[2] + bh, "同步为索引", dashed=True, lx=X[3] + 6, ly=Y[2] + bh + 14, anchor="start")
    b += arrow(X[6], Y[1] + bh, X[6], Y[3], "PUT · submit", lx=X[6] + 6, ly=C(2), anchor="start")
    b += arrow(X[6] + bw / 2, C(3), X[7] - bw / 2, C(3))
    b += arrow(X[7], Y[3] + bh, X[7], Y[4], "待审", lx=X[7] + 6, ly=Y[3] + bh + 14, anchor="start")
    b += arrow(X[7], Y[1] + bh, X[7], Y[3], "kind=补件", lx=X[7] + 6, ly=C(2), anchor="start")
    rx = X[7] + bw / 2 + 14
    b += arrow(X[7] + bw / 2, C(4), X[7] + bw / 2, C(1), via=[(rx, C(4)), (rx, C(1))])
    b += f'<text x="{rx + 4}" y="{(C(1) + C(4)) / 2}" font-size="11" fill="currentColor" transform="rotate(-90 {rx + 4} {(C(1) + C(4)) / 2})" text-anchor="middle">退回 → 返工 · 通过 → 补件</text>'
    # 决定回流记忆
    yb = Y[4] + bh + 22
    b += arrow(X[7] - 20, Y[4] + bh, X[3], Y[3] + bh, "决定 + 判断结论入记忆，下一期用", dashed=True, accent=True, via=[(X[7] - 20, yb), (X[3], yb)], lx=(X[3] + X[7]) / 2, ly=yb - 6)
    return fig(svg(W, yb + 16, b, "每日流程：引擎派单或人手动开启，采集服务采集媒体信息、合并与对照材料、逐条判断、深核、登记交付，主编首批校准，补件，Van 决定回流记忆"),
               "图 2 · 每日流程。两个入口汇到 1 开工；采集服务这一行是主线，每一步在工作台流程带上可见。强调色是第 2 步（只取图文，取候选、下载、识别、向量化一体）和第 5 步（每条都判，结论档加逐维依据，不打分）。对照材料五类：正式已发布、范例、生成过文章的贴文、03 决定、上一轮台账，刊发记录与决定经向量化同步成 LanceDB 索引。7 登记与交付之后只在关联任务时发生；Van 的每次决定连同判断结论入记忆。")

# ---------- 图 3：候选状态 ----------
def fig_state():
    b = ""
    bw, bh = 124, 40
    P = {"got": (20, 150), "merged": (170, 150), "mat": (320, 150),
         "rec": (490, 30), "alt": (490, 110), "no": (490, 190), "pend": (490, 270), "excl": (170, 260),
         "deep": (660, 30), "reg": (820, 30), "van": (980, 30), "carry": (660, 270), "closed": (820, 270)}
    L = {"got": ("已采集", "图 · 描述 · 向量"), "merged": ("已合并", "候选 → 事件"), "mat": ("有对照材料", "五类备齐"),
         "rec": ("推荐", "结论档"), "alt": ("备选", "可捞回进首批"), "no": ("不推荐", "保留 · 可捞回"), "pend": ("待核", "写明缺口"),
         "excl": ("硬性排除", "窗口外 · 03 否决同事实"), "deep": ("深核中", "证据 · 参照 · 时间盒"),
         "reg": ("已登记", "引擎条目"), "van": ("Van 决定", "采用 / 否决 / 暂缓"), "carry": ("结转", "后续期次重评"), "closed": ("关闭", "到期 · 写原因")}
    for k, (x, y) in P.items():
        b += box(x, y, bw, bh, L[k][0], L[k][1], accent=(k in ("rec", "mat")), dashed=(k == "excl"))
    R = lambda k: (P[k][0] + bw, P[k][1] + bh / 2)
    Lf = lambda k: (P[k][0], P[k][1] + bh / 2)
    b += arrow(*R("got"), *Lf("merged")); b += arrow(*R("merged"), *Lf("mat"), "缺一不判", lx=382, ly=164)
    for k, lab in [("rec", "档"), ("alt", ""), ("no", ""), ("pend", "缺资料 · 缺图")]:
        b += arrow(*R("mat"), *Lf(k), lab, lx=470, ly=P[k][1] + 14, anchor="end")
    b += arrow(P["merged"][0] + bw / 2, P["merged"][1] + bh, P["excl"][0] + bw / 2, P["excl"][1], "硬事实", lx=240, ly=248, anchor="start")
    b += arrow(*R("rec"), *Lf("deep")); b += arrow(*R("deep"), *Lf("reg"), "条目卡"); b += arrow(*R("reg"), *Lf("van"), "台账上")
    b += arrow(P["alt"][0] + bw / 2, P["alt"][1], P["alt"][0] + bw / 2, P["rec"][1] + bh, "捞回", dashed=True, lx=P["alt"][0] + bw / 2 + 6, ly=P["rec"][1] + bh + 26, anchor="start")
    b += arrow(P["no"][0] + bw / 2, P["no"][1], P["no"][0] + bw / 2, P["alt"][1] + bh, "捞回", dashed=True, lx=P["no"][0] + bw / 2 + 6, ly=P["alt"][1] + bh + 26, anchor="start")
    b += arrow(*R("pend"), *Lf("carry"), "当期未补齐", lx=637, ly=P["pend"][1] + bh + 12)
    b += arrow(*R("carry"), *Lf("closed"), "到期", lx=797, ly=P["carry"][1] + bh + 12)
    b += arrow(P["deep"][0] + bw / 2, P["carry"][1], P["deep"][0] + bw / 2, P["deep"][1] + bh, "补齐 → 重判", dashed=True, lx=P["deep"][0] + bw / 2 + 6, ly=160, anchor="start")
    b += arrow(P["van"][0] + bw / 2, P["van"][1] + bh, P["van"][0] + bw / 2, 340, None, accent=True)
    b += arrow(P["van"][0] + bw / 2, 340, P["mat"][0] + bw / 2, 340, "决定 + 判断结论 → 03 决定类参考，下一期对照材料读得到", accent=True, lx=700, ly=334)
    b += arrow(P["mat"][0] + bw / 2, 340, P["mat"][0] + bw / 2, P["mat"][1] + bh, None, accent=True)
    return fig(svg(1130, 356, b, "候选状态：已采集、已合并、有对照材料，然后判成推荐、备选、不推荐或待核；硬性排除只用硬事实且可捞回；推荐进深核、登记、Van 决定；待核可补齐重判或结转关闭；决定回流记忆"),
               "图 3 · 一条候选的状态。没有「因口味淘汰」这一档：不推荐照样保留、随时捞回；排除只用硬事实（窗口外、与 03 否决的同一事实同角度）；待核不永久出局。")

# ---------- 图 4：部署拓扑 ----------
def fig_deploy():
    b = ""
    def host(x, y, w, h, title, sub):
        return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="none" stroke="currentColor" stroke-opacity=".5" stroke-width="1.2"/>'
                f'<text x="{x + 12}" y="{y + 19}" font-size="12.5" font-weight="700" fill="currentColor">{esc(title)}</text>'
                f'<text x="{x + 12}" y="{y + 36}" font-size="10.5" fill="currentColor" opacity=".65">{esc(sub)}</text>')
    def note(x, y, t): return f'<text x="{x}" y="{y}" font-size="11" fill="currentColor" opacity=".8">{esc(t)}</text>'
    # agent 主机
    b += host(20, 20, 580, 340, "agent 主机 centos9 · 8.138.23.218（ssh 9318）", "8 vCPU · 30 GB（可用 9）· 866 GB（用 85%）· P40 24 GB · CentOS 9 · Rust 1.95 · bwrap · Docker")
    b += box(40, 72, 240, 44, "csw-collector（新，Rust）", "systemd · :8090 · LanceDB · 图片", accent=True)
    b += box(40, 128, 240, 40, "codex app-server（新装）", "子进程 · stdio · 独立 CODEX_HOME")
    b += box(40, 180, 240, 40, "opencli + Chrome（已在）", "小红书 · 网页")
    b += box(40, 232, 240, 40, "Hermes 网关 · 收集员（已在）", "切换时停；其余角色照常", dashed=True)
    b += box(340, 72, 240, 44, "qwen3-vl-embedding（已在）", "Docker · :8022 · 单 GPU 串行", accent=True)
    b += box(340, 128, 240, 40, "base-nginx（已在）", "80 / 443 · *.aworld.ltd 证书")
    b += box(340, 180, 240, 40, "csw-collector-web（新）", "静态文件 · 由 collector 托管")
    b += box(340, 232, 240, 40, "无关负载（已在）", "jellyfin · es · llm-wiki · open-webui")
    b += arrow(280, 94, 340, 94, "本机 :8022", lx=310, ly=86)
    b += arrow(160, 116, 160, 128); b += arrow(160, 168, 160, 180)
    b += arrow(460, 180, 460, 168)
    b += note(40, 296, "入口：collector.aworld.ltd → base-nginx :443 → 127.0.0.1:8090（JWT 复用引擎密钥）")
    b += note(40, 316, "目录：/opt/csw-collector/{bin, collector.env, data/lancedb, data/images, data/threads, logs}")
    b += note(40, 336, "出站：csw-agent API · 引擎 API · sub2api · OSS 图片直链 · 本机向量服务")
    # 引擎主机 / csw-agent 主机
    b += host(640, 20, 300, 160, "引擎主机 · 8.138.43.109 · tasks.aworld.ltd", "2 vCPU · 3 GB · 30 GB 可用 · Go + SQLite")
    b += box(660, 72, 260, 40, "server :8080 · adminsrv :8081", "nginx: /api/v1 · /admin · SPA")
    b += box(660, 122, 260, 40, "csw-daily-trigger.timer 05:30", "run · 派单 · 群播报")
    b += host(640, 200, 300, 160, "csw-agent 主机 · 47.245.60.56", "agent.campsomewhere.com · Docker")
    b += box(660, 252, 260, 40, "csw-agent-prod :8000 · pgvector", "posts/window · posts/{id} · generated")
    b += box(660, 302, 260, 40, "instagram-data 抓取器（screen）", "01:00 触发 Bright Data → webhook 入库")
    # 外部
    b += box(980, 72, 170, 40, "sub2api · 38.49.38.56", "gpt-6-astra · Responses")
    b += box(980, 132, 170, 40, "阿里云 OSS", "cws-file · 贴文图片")
    b += box(980, 212, 170, 40, "飞书", "群播报 · Hermes 网关")
    b += box(980, 272, 170, 40, "Bright Data", "Instagram 采集")
    # 连线
    b += arrow(600, 92, 660, 92, "bearer · HTTPS", lx=630, ly=84)
    b += arrow(600, 272, 660, 272, "API key · HTTPS", lx=630, ly=264)
    b += arrow(600, 190, 980, 92, "HTTPS：模型调用 · 图片直链", via=[(955, 190), (955, 92)], lx=780, ly=184)
    b += arrow(955, 152, 980, 152)
    b += arrow(940, 142, 980, 232, "播报", via=[(966, 142), (966, 232)], lx=972, ly=200, anchor="start")
    b += arrow(940, 322, 980, 292, "触发 · 回写", via=[(966, 322), (966, 292)], lx=972, ly=340, anchor="start")
    return fig(svg(1170, 372, b, "部署拓扑：新采集服务与工作台放在 agent 主机，与向量服务同机；引擎与 csw-agent 各在自己的主机；模型网关、OSS、飞书、Bright Data 在外部"),
               "图 4 · 部署拓扑。新增的只有 agent 主机上的 csw-collector（含工作台静态文件）和 codex 子进程；向量服务本来就在这台机上，走本机端口不经 nginx。引擎主机与 csw-agent 主机不动，只各加一个接口。")


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
    b += rb(1, 2, "2a 取候选", "只取图文 · 按任务类型跑采集方案")
    b += rb(2, 2, "来源（采集器各跑一轮）", "csw 贴文 · 小红书 · 网页 · 链接 · 可加")
    x, y = L(2, 2); x2, y2 = R(1, 2)
    b += arrow(x, y, x2, y2, "统一候选", lx=(x + x2) / 2, ly=y - 8)
    b += rb(1, 3, "2b 下载图片", "缩略 · 05 / 11 取原图 · 哈希")
    b += rb(2, 3, "OSS", "缩略 w_768 · 原图按需")
    x, y = L(2, 3); x2, y2 = R(1, 3)
    b += arrow(x, y, x2, y2, "文件", lx=(x + x2) / 2, ly=y - 8)
    b += rb(1, 4, "2c 识别图片", "每张 × 该条正文 · 描述 + 用途判断")
    b += rb(2, 4, "模型（视觉）", "对应正文哪点 · 画面 · 可否作配图")
    x, y = L(2, 4); x2, y2 = R(1, 4)
    b += arrow(x, y, x2, y2, "描述", accent=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 5：合并
    b += rb(1, 5, "3 合并与窗口", "候选 → 事件 · 窗口外标注不删")
    b += rb(2, 5, "Jev 窄判断", "是否同一事件 · 昨日已判", dashed=True)
    x, y = L(2, 5); x2, y2 = R(1, 5)
    b += arrow(x, y, x2, y2, "判定", dashed=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 6：对照材料（三样必备）
    b += rb(1, 6, "4 对照材料", "知识库 · 已挑选记录 · 框架，缺一不判")
    b += rb(0, 6, "知识库 · 决定 · 记忆", "正式已发布 · 03 决定 · 案例")
    b += rb(2, 6, "csw 后台", "Van 的挑选记录")
    x, y = R(0, 6); x2, y2 = L(1, 6)
    b += arrow(x, y, x2, y2, "旧文 · 决定", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 6); x2, y2 = R(1, 6)
    b += arrow(x, y, x2, y2, "已选贴文", lx=(x + x2) / 2, ly=y - 8)
    # 行 7：逐条判断
    b += rb(1, 7, "5 逐条综合判断", "Jev 初评 → 生成模型 → Jev 核对")
    b += rb(2, 7, "Jev + 判断模型", "六维概率 · 结论档 · 依据 · 三句话")
    x, y = L(2, 7); x2, y2 = R(1, 7)
    b += arrow(x, y, x2, y2, "判断结果", accent=True, lx=(x + x2) / 2, ly=y - 8)
    # 行 8：深核
    b += rb(1, 8, "6 深核", "推荐 + 待核 · 可打断 · 线程落盘")
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
    b += f'<text x="30" y="{(y + y2) / 2 + 4}" font-size="11" fill="{ACC}" transform="rotate(-90 30 {(y + y2) / 2 + 4})" text-anchor="middle">决定 + 判断结论入记忆，下一期用</text>'
    # 行 13：05 / 11
    b += rb(0, 13, "05 配图 · 11 选图包", "逐条派工 · 同起点")
    b += rb(1, 13, "05 / 11 接单", "采集媒体信息（单条 · 原图）· 核素材 · 交付")
    b += rb(2, 13, "agent.campsomewhere.com", "GET posts/{id}?include-media")
    x, y = R(0, 13); x2, y2 = L(1, 13)
    b += arrow(x, y, x2, y2, "派工单", lx=(x + x2) / 2, ly=y - 8)
    x, y = L(2, 13); x2, y2 = R(1, 13)
    b += arrow(x, y, x2, y2, "单条 + 媒体", lx=(x + x2) / 2, ly=y - 8)
    return fig(svg(W, H, b, "完整流程：引擎派单或人在工作台手动开启，工作台开工后采集媒体信息（只取图文，取候选、下载、识别一体）、合并、取对照材料、逐条综合判断、深核；关联任务时登记交付，主编首批校准，补件，下游与 Van 决定回流"),
               "图 A · 完整流程。每一行是一步，列是谁在做。两种入口汇到同一条线：引擎派单，或人在工作台手动开启一轮。工作台这一列是主线：开工之后的每一步都在它上面可见；7 登记与交付、8 首批校准、9 补件和下游只在关联任务时发生，手动开启不关联任务的只出本地台账与报告。强调色框出与 9/20 设计稿不同之处：第 2 步只取图文，把取候选、下载、识别合成一个整体；第 5 步每条候选都判，判断的输入是正文、图片与第 4 步的三样对照材料（知识库、已挑选记录、Van 判断框架），缺一样不判。虚线只剩第 3 步的模型判定，是可选的。")

# ---------- 图 B：采集媒体信息的采集器模型 ----------
def fig_collectors():
    b = ""
    b += box(20, 40, 180, 48, "任务类型", "intake · material · xhs_pick …")
    b += box(260, 40, 180, 48, "采集方案", "有序采集器清单 · 版本", accent=True)
    b += box(20, 190, 180, 48, "工作台 · 设置页", "增删 · 排序 · 参数 · 回放")
    b += box(260, 190, 180, 48, "来源台账", "必扫 / 启停 / 实例参数")
    b += '<rect x="520" y="20" width="270" height="322" rx="6" fill="none" stroke="currentColor" stroke-opacity=".35" stroke-dasharray="6 4"/>'
    b += '<text x="655" y="40" text-anchor="middle" font-size="12" font-weight="700" fill="currentColor" opacity=".7">采集器（每个各跑一轮，记采集轮）</text>'
    names = [("csw 贴文窗口", "posts/window · 只图文"), ("csw 单条", "GET posts/{id}"), ("小红书", "opencli 受控调用"), ("网页 / RSS", "域名白名单抓取"), ("Van 链接", "解析后交对应采集器")]
    y = 52
    for t, sub in names:
        b += box(540, y, 230, 40, t, sub, fs=12)
        y += 46
    b += box(540, y, 230, 40, "自定义采集器", "外部命令 / HTTP 端点 / MCP 工具", accent=True, dashed=True, fs=12)
    b += box(850, 40, 170, 52, "统一候选格式", "来源 · id · 正文 · 时间 · 媒体")
    b += box(850, 130, 170, 52, "下载图片", "缩略 / 原图 · 哈希 · 失败标记")
    b += box(850, 220, 170, 52, "识别图片", "每张 × 正文 · 描述 + 用途", accent=True)
    b += box(850, 310, 170, 52, "暂存库 + 采集轮", "去重 · discovered_via · 计数")
    b += arrow(200, 64, 260, 64, "选方案", lx=230, ly=56)
    b += arrow(440, 64, 520, 64, "按序跑", lx=480, ly=56)
    b += arrow(790, 180, 850, 66, "各自返回", via=[(820, 180), (820, 66)], lx=826, ly=150, anchor="start")
    b += arrow(935, 92, 935, 130, "每条候选", lx=943, ly=115, anchor="start")
    b += arrow(935, 182, 935, 220, "每张图", lx=943, ly=205, anchor="start")
    b += arrow(935, 272, 935, 310, "带描述入库", lx=943, ly=295, anchor="start")
    b += arrow(200, 214, 260, 214, "改方案", lx=230, ly=206)
    b += arrow(350, 190, 350, 88, "实例参数", lx=358, ly=144, anchor="start")
    return fig(svg(1040, 384, b, "采集媒体信息：任务类型选采集方案，采集器各跑一轮，只取图文，结果归一后逐条下载图片、逐张识别，带描述入暂存库并记采集轮；方案可在工作台改，也可挂自定义采集器"),
               "图 B · 采集媒体信息。任务类型决定跑哪套采集方案；方案是一份有序的采集器清单，每个采集器各跑一轮、各记一条采集轮；结果归一成同一种候选格式后，不分来源地逐条下载图片、逐张识别，带着描述进暂存库；含视频的贴文在采集器里就过滤掉。取 csw-agent 贴文只是清单里的一项，加来源是改清单，加取法是加一个采集器；改方案在工作台设置页操作，每次改动写审计并存版本。")
