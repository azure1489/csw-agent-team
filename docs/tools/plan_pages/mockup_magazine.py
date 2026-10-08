# -*- coding: utf-8 -*-
"""界面设计稿 · 杂志背景库部分（方案 §8 工作台六项，2026-10-09 步骤 7）。

数据：`magazine_sample.json` 取自 GO OUT 2016.08 的真实清单（译文、商品、裁图与整页 OSS 地址）；
统计数字、相似度、耗时为示意。由 build_mockup.py 引入。
"""
import html
import json
import os

E = lambda s: html.escape(str(s))

CSS = """
.seg { display:inline-flex; border:1px solid var(--rule); border-radius:6px; overflow:hidden; }
.seg span { padding:4px 10px; font-size:12.5px; color:var(--muted); border-right:1px solid var(--rule); } .seg span:last-child { border-right:0; }
.seg span.on { background:var(--accent-soft); color:var(--accent); font-weight:600; }
.mag { display:grid; grid-template-columns:96px minmax(0,1fr); gap:12px; border:1px solid var(--rule); border-radius:8px; padding:10px; margin-bottom:8px; }
.mag img, .mag .ph { width:96px; height:96px; object-fit:cover; border-radius:6px; background:var(--ground); }
.mag .meta { font-size:11.5px; color:var(--muted); } .mag b { font-size:13px; display:block; margin:2px 0; }
.mag .price { font-family:var(--mono); font-size:12px; }
.mag details { font-size:12px; color:var(--muted); margin-top:4px; } .mag details summary { cursor:pointer; color:var(--accent); }
.drop { border:1.5px dashed var(--rule); border-radius:8px; padding:22px 12px; text-align:center; color:var(--muted); font-size:12.5px; }
.drop b { display:block; color:var(--ink); font-size:13px; margin-bottom:4px; }
.states { display:grid; grid-template-columns:repeat(auto-fit,minmax(220px,1fr)); gap:10px; }
.state h4 { margin:0 0 6px; }
.qimg { display:flex; gap:10px; align-items:center; border:1px solid var(--rule); border-radius:8px; padding:8px; }
.qimg img { width:64px; height:64px; object-fit:cover; border-radius:6px; }
.bar { height:6px; border-radius:3px; background:var(--ground); overflow:hidden; margin:6px 0 2px; } .bar i { display:block; height:100%; background:var(--accent); }
.igrid { display:grid; grid-template-columns:repeat(auto-fill,minmax(110px,1fr)); gap:8px; }
.igrid figure { margin:0; border:1px solid var(--rule); border-radius:6px; overflow:hidden; }
.igrid img { width:100%; aspect-ratio:1; object-fit:cover; display:block; background:var(--ground); }
.igrid figcaption { font-size:11px; padding:4px 6px; color:var(--muted); line-height:1.4; } .igrid figcaption b { color:var(--ink); font-size:11.5px; }
.pagebox { position:relative; display:inline-block; max-width:100%; } .pagebox img { display:block; max-width:100%; border-radius:6px; border:1px solid var(--rule); }
.pagebox i { position:absolute; border:2px solid var(--accent); background:rgba(79,70,229,.10); border-radius:2px; }
.crop { max-width:100%; border-radius:6px; border:1px solid var(--rule); display:block; }
.kv { display:grid; grid-template-columns:6.5em minmax(0,1fr); gap:4px 10px; font-size:12.5px; } .kv dt { color:var(--muted); } .kv dd { margin:0; word-break:break-all; }
.md { font-size:12.5px; white-space:pre-line; }
"""


def load(path):
    if not os.path.exists(path):
        return None
    return json.load(open(path, encoding="utf-8"))


def label(x):
    p = x.get("printed_page")
    page = f"P.{p}" if p else f"PDF 第 {x['pdf_index'] + 1} 页"
    return f"{x['magazine']} {x['issue']} {page}"


def md(t):
    """译文是 Markdown：设计稿里只把标题与粗体渲染出来（前端用正式的渲染）。"""
    import re
    out = []
    for line in E(t).split("\n"):
        m = re.match(r"#+\s*(.*)", line)
        line = f"<b>{m.group(1)}</b>" if m else line
        out.append(re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", line))
    return "\n".join(out)


def price(p):
    y = p.get("price_jpy")
    if not y:
        return ""
    tax = {True: "含税", False: "不含税"}.get(p.get("tax_included"), "")
    return f"¥{y:,}{'（' + tax + '）' if tax else ''}"


def card(x, routes, sim=None):
    p = (x.get("products") or [{}])[0]
    head = " ".join(v for v in (p.get("brand"), p.get("name")) if v) or x.get("section_title_zh") or "杂志"
    more = len(x.get("products") or []) - 1
    tags = "".join(f'<span class="tag">{E(r)}</span>' for r in routes)
    if sim is not None:
        tags += f'<span class="tag pri">相似 {sim:.2f}</span>'
    return f'''<div class="mag"><img loading="lazy" src="{E(x['image_url'])}" alt="{E(head)}">
<div><span class="meta">{E(label(x))} · {E(x.get('section_title_zh') or '')} {tags}</span>
<b>{E(head)}</b>
<span class="price">{E(price(p))}</span>{f' <span class="meta">另 {more} 件商品</span>' if more > 0 else ''}
<details><summary>描述与译文</summary>{E(x.get('description') or '')}<br>{E((x.get('md_zh') or '')[:240])}</details>
<span class="meta"><a href="#" style="color:var(--accent)">条目详情</a> · <a href="{E(x['image_url'])}" style="color:var(--accent)">裁图</a> · <a href="{E(x['page_url'])}" style="color:var(--accent)">整页</a></span></div></div>'''


def kb_screen(s):
    """知识库页：统计区（加杂志背景卡）、检索区（范围切换 + 以图搜三态）、结果卡、品牌页两节、同步状态。"""
    snow, cool, tnf = s["snow"], s["cool"], s["tnf"]
    q = cool[0]
    text_hits = "".join(card(x, r) for x, r in zip(snow[:3], (["全文", "向量"], ["全文"], ["向量"])))
    sims = [0.97, 0.83, 0.79, 0.74]
    img_grid = "".join(
        f'<figure><img loading="lazy" src="{E(x["image_url"])}" alt=""><figcaption><b>{E((x["products"][0].get("brand") or "")[:22])}</b><br>{E(label(x))} · {sims[i]:.2f}</figcaption></figure>'
        for i, x in enumerate(cool[:4])
    )
    brand_mag = "".join(
        f'<tr><td>{E(x["magazine"])} {E(x["issue"])}</td><td>{E(("P." + str(x["printed_page"])) if x.get("printed_page") else "")}</td><td>{E(x["products"][0].get("name") or "")}</td><td class="num">{E(price(x["products"][0]))}</td><td><img loading="lazy" src="{E(x["image_url"])}" alt="" width="40" height="40" style="object-fit:cover;border-radius:4px"></td></tr>'
        for x in tnf[:4]
    )
    return f'''
<div class="cards">
 <div class="card"><span class="k">参考库条目（四类）</span><b>22,140</b><span class="s">判断只看这四类</span></div>
 <div class="card"><span class="k">待算向量</span><b>0</b><span class="s">只算参考库四类</span></div>
 <div class="card"><span class="k">品牌</span><b>918</b><span class="s">别名 2,304</span></div>
 <div class="card"><span class="k">杂志背景</span><b>5 本 · 2,960 条</b><span class="s">融合向量 1,204 / 2,710 · 纯图 0 / 2,488<br>预计约 3 小时算完（按 0.6 秒 / 图）· <span class="tag warn">正式轮期间暂停</span></span></div>
</div>
<div class="toolbar"><div class="filters"><span class="seg"><span class="on">文字搜</span><span>以图搜</span></span> <span class="seg"><span>判断参考库（四类）</span><span class="on">杂志背景</span></span></div><div class="right"><input class="search" value="snow peak" placeholder="品牌、产品、关键词"><select><option>全部杂志</option><option>GO OUT 2016.08</option></select><button class="primary">搜</button></div></div>
<p class="muted" style="margin:-6px 0 0">范围默认「判断参考库」。切到「杂志背景」只搜杂志：全文 ∪ 向量，不重排；杂志是背景参照，不进判断。</p>
<div class="grid2">
<div class="panel"><h3>杂志背景 · 「snow peak」· 3 条（示意排序）</h3>{text_hits}
<p class="muted">同一本杂志可以出多条；每条带刊名期号页码、品牌商品价格，描述与译文折叠；缩略图走 OSS 公开地址、懒加载。</p></div>
<div class="panel"><h3>以图搜图 · 三态</h3><div class="states">
<div class="state"><h4>上传前</h4><div class="drop"><b>拖一张图进来</b>或粘贴 · 或 <button>选文件</button><br>jpg / png / webp，≤20MB；缩到 768 宽再算</div><p class="muted">只有主编那一档能用：一次占一块 GPU 约一秒</p></div>
<div class="state"><h4>计算中</h4><div class="qimg"><img src="{E(q['image_url'])}" alt=""><div><b style="font-size:12.5px">ig_cooler.jpg · 1.8 MB</b><div class="bar"><i style="width:60%"></i></div><span class="muted" style="font-size:11.5px">算向量中…（正式轮在跑时排队）</span></div></div></div>
<div class="state"><h4>出结果</h4><p class="muted" style="margin:0 0 6px;font-size:12px">算向量 0.9 秒 · 共 1.1 秒 · 回 4 条</p><div class="igrid">{img_grid}</div></div>
</div><p class="muted">命中经 <code>kb_doc_images</code> 回到条目；同一张图出现在几期就给几条；纯图向量还没算完的书暂时搜不到，统计卡上写着进度。</p></div>
</div>
<div class="grid2">
<div class="panel"><h3>品牌页 · THE NORTH FACE PURPLE LABEL</h3>
<h4>CSW 历史覆盖 · 3</h4><div class="ref"><b>TNF 紫标 2026 秋冬（示意）</b><span class="sub">2026-09-05 · <span class="tag ok">正式已发布</span></span></div><p class="muted">近 30 天 1 篇 · 近 90 天 3 篇（示意）</p>
<h4>杂志里出现过 · 按刊期新的在前</h4><table class="plain"><tr><th>刊期</th><th>页</th><th>商品</th><th>价格</th><th></th></tr>{brand_mag}</table>
<p class="muted">两节分开、不混排：杂志不是编辑部的口味证据。品牌按品牌键比（大小写、空格、标点不计）；「THE NORTH FACE」与「THE NORTH FACE PURPLE LABEL」是两个品牌。</p></div>
<div class="panel"><h3>同步与索引</h3><table class="plain">
<tr><td>刊发记录</td><td>同步到 2026-10-08 · 正式已发布 291 篇</td></tr>
<tr><td>生成过文章的贴文</td><td>2,140 条 · 10-09 01:20 同步</td></tr>
<tr><td>范例 · 03 决定</td><td>10 篇 41 条 · 决定 52 条</td></tr>
<tr><td>杂志同步</td><td>10-09 02:30 · 5 本 · 最近入库 GO-OUT-2026.02（873 张图 → 612 条）</td></tr>
<tr><td>杂志回填</td><td class="warn">融合剩 1,506 · 纯图剩 2,488 · 正式轮 05:25–06:30 暂停</td></tr>
<tr><td>坏图</td><td>2 张读不出来（本进程跳过，重启后重试）</td></tr>
<tr><td>LanceDB</td><td>docs 23,344 · images 9,120</td></tr>
</table>
<h4>总览告警措辞</h4><ul class="alerts">
<li>杂志背景：GO-OUT-2026.03 清单已写 3 小时仍未入库（看刊译台 ingested.json 与工作台日志）</li>
<li class="warn">杂志回填：向量服务连续失败，已暂停 10 分钟（不影响判断）</li>
<li>杂志回填进度只作提示，不进告警：正常的「还剩多少」不是要人处理的事</li></ul></div>
</div>'''


def detail_screen(s):
    """杂志条目详情（新页）：大图、整页图上标 bbox、商品表、复制地址。"""
    x = s["cool"][0]
    w, h = x["page_size"]
    x0, y0, x1, y1 = x["bbox_pt"]
    box = f"left:{x0 / w * 100:.2f}%;top:{y0 / h * 100:.2f}%;width:{(x1 - x0) / w * 100:.2f}%;height:{(y1 - y0) / h * 100:.2f}%"
    rows = "".join(
        f'<tr><td>{E(p.get("brand") or "")}</td><td>{E(p.get("name") or "")}</td><td class="num">{E(price(p))}</td><td>{E(p.get("specs") or "")}</td></tr>'
        for p in x["products"]
    )
    p = x["products"][0]
    return f'''
<div class="toolbar"><div class="filters"><button class="link">← 返回检索</button><span class="tag">{E(label(x))}</span><span class="tag">{E(x.get("section_title_zh") or "")}</span><span class="tag">{E(x.get("category") or "")}</span><span class="tag ok">融合向量已算</span><span class="tag ok">纯图向量已算</span></div>
<div class="right"><button>复制裁图地址</button><button>复制整页地址</button><button class="primary">以这张图搜图</button></div></div>
<div class="detail">
<div class="col"><h3>{E(p.get("brand") or "")} · {E(p.get("name") or "")}</h3><img class="crop" src="{E(x["image_url"])}" alt="裁图">
<h3>商品</h3><table class="plain"><tr><th>品牌</th><th>商品</th><th>价格</th><th>规格</th></tr>{rows}</table></div>
<div class="col"><h3>整页 · 框出这张图的位置</h3><div class="pagebox"><img src="{E(x["page_url"])}" alt="整页"><i style="{box}"></i></div>
<p class="muted">框按清单的 bbox（PDF 点）与 page_size 换算成百分比，整页图缩放后照样对得上。</p></div>
<div class="col"><h3>描述</h3><p class="md">{E(x.get("description") or "")}</p>
<h3>译文</h3><p class="md">{md(x.get("md_zh") or "")}</p>
<h3>出处</h3><dl class="kv"><dt>刊物</dt><dd>{E(x["magazine"])} {E(x["issue"])}（{E(x.get("issue_date") or "")}）</dd><dt>页码</dt><dd>P.{E(x.get("printed_page"))} · PDF 第 {x["pdf_index"] + 1} 页</dd><dt>书</dt><dd class="mono">{E(x["book_key"])}</dd><dt>裁图</dt><dd><a href="{E(x["image_url"])}" style="color:var(--accent)">{E(x["image_url"])}</a></dd></dl>
<p class="muted">只读。要删整本走 <code>kb purge --book</code>（运维）；刊译台重新解析后按本对账，旧条目自动清掉。</p></div>
</div>'''
