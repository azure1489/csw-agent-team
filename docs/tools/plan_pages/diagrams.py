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

