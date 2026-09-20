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
    b += box(300, 20, 300, 230, "", None)  # 容器
    b += f'<text x="450" y="40" text-anchor="middle" font-size="12" font-weight="700" fill="currentColor" opacity=".7">采集服务（Go，agent 主机）</text>'
    b += box(320, 52, 120, 44, "取全量", "posts/window")
    b += box(460, 52, 120, 44, "事件合并", "硬事实 · 查重候选")
    b += box(320, 108, 120, 44, "打分器", "文字 + 缩略图", accent=True)
    b += box(460, 108, 120, 44, "深核", "Codex App Server")
    b += box(320, 164, 120, 44, "本地 MCP", "kb · memory · fetch")
    b += box(460, 164, 120, 44, "工作台接口", "暂存库 · 线程记录")
    b += box(700, 40, 190, 56, "任务流转引擎", "任务 · 条目 · 打分台账 · 交付物")
    b += box(700, 150, 190, 56, "知识库 · 选题记忆", "引擎内 · FTS · 反馈记录")
    b += box(300, 300, 140, 50, "csw 后端", "贴文库 · OSS 图片")
    b += box(460, 300, 140, 50, "sub2api", "gpt-6-astra")
    b += box(620, 300, 140, 50, "opencli", "小红书 · 网页")
    b += arrow(200, 68, 300, 68, "JWT", lx=250, ly=60)
    b += arrow(115, 206, 795, 206, "读写定义与监控（不变）", dashed=True, via=[(115, 280), (795, 280)], lx=450, ly=274)
    b += arrow(600, 74, 700, 68, "任务 · 台账 · 交付物", lx=650, ly=52)
    b += arrow(600, 186, 700, 178, "检索", lx=650, ly=172)
    b += arrow(380, 300, 380, 208, "posts/window", lx=392, ly=232, anchor="start")
    b += arrow(530, 300, 530, 208, "Responses 接口", lx=542, ly=232, anchor="start")
    b += arrow(690, 300, 620, 208, "受控调用", lx=668, ly=232, anchor="start")
    return fig(svg(920, 370, b, "系统位置：工作台只经采集服务访问引擎与外部服务"),
               "图 1 · 系统位置。工作台前端只和采集服务说话；采集服务对外接 csw 后端、模型服务与 opencli，对内读写引擎。管理后台保持只管定义与监控。")

# ---------- 图 2：每日流程泳道 ----------
def fig_flow():
    W = 1180; laneH = 96; lanes = ["数据源", "采集服务（代码）", "模型", "引擎（真相）", "人：主编 · 研究员 · Van"]
    b = ""
    for i, t in enumerate(lanes):
        b += lane(10, 20 + i * laneH, W - 20, laneH, t)
    Y = [20 + i * laneH + 34 for i in range(5)]  # 每泳道盒子 y
    # 列 x 位置
    X = [90, 250, 410, 570, 730, 890, 1030]
    bw, bh = 130, 44
    # 数据源
    b += box(X[0]-bw/2, Y[0], bw, bh, "csw 后端", "前一天到当天全量")
    b += box(X[4]-bw/2, Y[0], bw, bh, "原始来源", "官方页 · OSS 图")
    # 代码
    b += box(X[0]-bw/2, Y[1], bw, bh, "A 取全量", "分页取完 · 计数")
    b += box(X[1]-bw/2, Y[1], bw, bh, "B 合并与硬事实", "窗口 · 事件 · 缩略图")
    b += box(X[5]-bw/2, Y[1], bw, bh, "F 登记与交付", "条目 · 采集轮 · 台账")
    # 模型
    b += box(X[2]-bw/2, Y[2], bw, bh, "C 查重判断", "同一事实或角度？")
    b += box(X[3]-bw/2, Y[2], bw, bh, "D 打分", "六维度 · 依据 · 三句话", accent=True)
    b += box(X[4]-bw/2, Y[2], bw, bh, "E 深核", "证据 · 参照 · 完整图")
    # 引擎
    b += box(X[2]-bw/2, Y[3], bw, bh, "知识库", "正式已发布")
    b += box(X[3]-bw/2, Y[3], bw, bh, "打分台账", "全部事件")
    b += box(X[5]-bw/2, Y[3], bw, bh, "条目 · 交付物", "首批校准闸")
    b += box(X[6]-bw/2, Y[3], bw, bh, "选题记忆", "决定 + 维度分")
    # 人
    b += box(X[3]-bw/2, Y[4], bw, bh, "工作台 · 台账", "捞回 · 改判 · 首批")
    b += box(X[6]-bw/2, Y[4], bw, bh, "Van 选择", "采用 / 否决 / 暂缓 + 原话")
    # 箭头
    c = lambda i, l: (X[i], Y[l] + bh/2)
    b += arrow(X[0], Y[0]+bh, X[0], Y[1], "posts/window")
    b += arrow(X[0]+bw/2, Y[1]+bh/2, X[1]-bw/2, Y[1]+bh/2)
    b += arrow(X[1]+bw/2, Y[1]+bh/2, X[2]-bw/2, Y[2]+bh/2, "事件 + 候选旧文", lx=330, ly=Y[1]+bh+14, anchor="start")
    b += arrow(X[2], Y[3], X[2], Y[2]+bh, "检索候选", lx=X[2]+8, ly=Y[2]+bh+30, anchor="start")
    b += arrow(X[2]+bw/2, Y[2]+bh/2, X[3]-bw/2, Y[2]+bh/2, "查重结论", lx=(X[2]+X[3])/2, ly=Y[2]-4)
    b += arrow(X[3], Y[2]+bh, X[3], Y[3], "每个事件一条", accent=True, lx=X[3]+8, ly=Y[2]+bh+30, anchor="start")
    b += arrow(X[3]+bw/2, Y[2]+bh/2, X[4]-bw/2, Y[2]+bh/2, "入围 + 待核", lx=(X[3]+X[4])/2, ly=Y[2]-4)
    b += arrow(X[4], Y[0]+bh, X[4], Y[2], "核事实 · 看图", lx=X[4]+8, ly=Y[1]+bh/2, anchor="start")
    b += arrow(X[4]+bw/2, Y[2]+bh/2, X[5]-bw/2, Y[1]+bh/2, "条目卡", lx=850, ly=Y[1]+bh+14, anchor="start")
    b += arrow(X[5], Y[1]+bh, X[5], Y[3], "PUT items · sweeps · scores", lx=X[5]+8, ly=Y[2]+bh+18, anchor="start")
    b += arrow(X[3], Y[3]+bh, X[3], Y[4], "读台账")
    b += arrow(X[3]+bw/2, Y[4]+bh/2, X[6]-bw/2, Y[4]+bh/2, "07:15 前送排序表")
    b += arrow(X[6], Y[4], X[6], Y[3]+bh, "决定连同维度分入库", accent=True, lx=X[6]-8, ly=Y[4]-10, anchor="end")
    b += arrow(X[6]-bw/2, Y[3]+bh/2, X[4]+bw/2, Y[2]+bh/2, "下一期：案例 · 准则", dashed=True, via=[(X[6]-bw/2-20, Y[3]+bh/2), (X[6]-bw/2-20, Y[2]+bh/2)], lx=(X[6]-bw/2-20+X[4]+bw/2)/2, ly=Y[2]+bh/2-8)
    return fig(svg(W, 20 + 5*laneH + 10, b, "每日流程：代码取全量与合并，模型查重打分深核，引擎存台账与条目，人看台账并做决定，决定回流记忆"),
               "图 2 · 每日 01 流程。横向是六步，纵向是谁在做。强调色标出打分：每个事件都在引擎留下一行台账；Van 的每次决定连同当时的维度分入库，下一期的查重与打分读得到。")

# ---------- 图 3：事件状态 ----------
def fig_state():
    b = ""
    bw, bh = 130, 40
    P = {"merged": (20, 125), "scored": (180, 125), "short": (420, 30), "pending": (420, 125), "excluded": (420, 220),
         "deep": (600, 30), "registered": (770, 30), "van": (940, 30), "carry": (600, 125), "closed": (770, 125)}
    L = {"merged": ("已合并", "贴文 → 事件"), "scored": ("已打分", "六维度 · 依据"), "short": ("入围", "排序或人"),
         "pending": ("待核", "写明缺口"), "excluded": ("硬性排除", "窗口外 · 重复 · 刚被否"), "deep": ("深核中", "证据 · 参照"),
         "registered": ("已登记", "引擎条目"), "van": ("Van 决定", "采用 / 否决 / 暂缓"), "carry": ("结转", "后续期次重评"), "closed": ("关闭", "到期 · 写原因")}
    for k, (x, y) in P.items():
        b += box(x, y, bw, bh, L[k][0], L[k][1], accent=(k == "scored"), dashed=(k == "excluded"))
    R = lambda k: (P[k][0] + bw, P[k][1] + bh/2)   # 右边中点
    Lf = lambda k: (P[k][0], P[k][1] + bh/2)       # 左边中点
    b += arrow(*R("merged"), *Lf("scored"), "模型")
    b += arrow(*R("scored"), *Lf("short"), "排名靠前", lx=352, ly=84)
    b += arrow(*R("scored"), *Lf("pending"), "缺资料 · 缺图", lx=365, ly=138)
    b += arrow(*R("scored"), *Lf("excluded"), "硬事实", lx=352, ly=204)
    b += arrow(*R("short"), *Lf("deep")); b += arrow(*R("deep"), *Lf("registered"), "条目卡"); b += arrow(*R("registered"), *Lf("van"), "台账上")
    b += arrow(413, 240, 413, 50, "捞回", dashed=True, lx=405, ly=108, anchor="end")
    b += arrow(*R("pending"), *Lf("carry"), "当期未补齐", lx=575, ly=118)
    b += arrow(*R("carry"), *Lf("closed"), "到期", lx=750, ly=118)
    b += arrow(665, 125, 665, 70, "补齐 → 重评", dashed=True, lx=673, ly=104, anchor="start")
    b += arrow(1005, 70, 1005, 284, None, accent=True); b += arrow(1005, 284, 245, 284, "决定 + 维度分 → 记忆库，下一期可查", accent=True, lx=625, ly=278)
    b += arrow(245, 284, 245, 165, None, accent=True)
    return fig(svg(1090, 300, b, "事件状态：已合并、已打分，然后入围、待核或硬性排除；硬性排除可捞回，待核可补齐重评或结转关闭；Van 的决定回流记忆"),
               "图 3 · 一个事件的状态。没有「因口味淘汰」这一档：排除只用硬事实且可捞回，待核不永久出局。")
