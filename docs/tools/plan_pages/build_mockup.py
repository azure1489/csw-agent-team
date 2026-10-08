# -*- coding: utf-8 -*-
"""情报收集员工作台 · 界面设计稿（十一个页面，静态原型）。
用法：python build_mockup.py events_scored.json events_meta.json out.html
9/21 按流程确认稿第 14 版重做：流程带、结论档 + 逐维判定（不打分、无权重）、热度与标签、对照材料五类、采集方案与手动开启。
六维度判定由早期打分实验的 0–10 分换算（≥7 成立、4–6 不明、≤3 不成立），属示意；贴文、热度、标签、范例命中、Van 决定为真实数据。"""
import json, sys, html, os
import mockup_magazine as MZ
src, meta_src, out = sys.argv[1], sys.argv[2], sys.argv[3]
EV = [e for e in json.load(open(src, encoding="utf-8")) if e.get("scores")]
META = json.load(open(meta_src, encoding="utf-8"))
MAG = MZ.load(os.path.join(os.path.dirname(os.path.abspath(meta_src)), "magazine_sample.json"))
DIMS = [("change", "具体变化"), ("use", "使用关联"), ("gain", "信息增量"), ("compare", "比较参照"), ("explain", "值得解释"), ("csw", "调性适配")]
E = lambda s: html.escape(str(s))

def verdict(v): return "成立" if v >= 7 else ("不明" if v >= 4 else "不成立")
def tier_of(e):
    s = e["scores"]; d = s["dims"]
    if s["unanswered"] == "missing_material": return "待核"
    ok = sum(1 for k, _ in DIMS if verdict(d[k]) == "成立")
    if ok >= 5 and verdict(d["change"]) == "成立": return "推荐"
    if ok >= 3: return "备选"
    return "不推荐"
TIER_CLS = {"推荐": "rec", "备选": "alt", "待核": "pending", "不推荐": "no"}
TIER_ORDER = {"推荐": 0, "备选": 1, "待核": 2, "不推荐": 3}
for e in EV:
    e["tier"] = tier_of(e)
    e["ok"] = sum(1 for k, _ in DIMS if verdict(e["scores"]["dims"][k]) == "成立")
    m = META.get(e["shortcode"], {})
    e["meta"] = m
    e["heat"] = (m.get("likes", 0) / m["acct_median"]) if m.get("acct_median") else None
EV.sort(key=lambda e: (TIER_ORDER[e["tier"]], -e["ok"], -(e["heat"] or 0), e["date_posted"]), reverse=False)

# 真实对照材料（五类）
PUB_HIT = {"brompton": ("and wander × Brompton：骑小布，有了一套官方穿法 · vol.248", "2026-09-02", "同品牌同产品：本条是 ROBIN 门店视角与骑行实拍，需判是否有新增信息"),
           "omnium": ("Carhartt WIP × OMNIUM：把工装骑上街 · vol.260", "2026-09-18", "同一事件：本期已成稿（Van 指定链接），本条为媒体号视角")}
EXAMPLE_HIT = {"brompton": "范例 vol.248 条目 1「and wander × Brompton｜骑小布，有了一套官方穿法」（09-02）：联名骑行装、通勤与山野切换",
               "omnium": "范例 vol.260 条目 2「Carhartt WIP × OMNIUM｜把工装骑上街」（09-18）：工装文化落到可骑行的城市工具",
               "gnuhr": "范例 vol.260 条目 5「gnuhr」（09-18）：面料送检数据说话"}
DECISION = {"DdZwiM9EnWL": ("采用", "Van 9/18 亲选链接，当期成稿"), "DdYgHUNE6ux": ("否决", "Van 9/18 退回两条联名之一（未给理由）"), "DdYmJ-ahq34": ("否决", "Van 9/18 退回两条联名之一（未给理由）")}

def tag(t, cls=""): return f'<span class="tag {cls}">{E(t)}</span>'
def tier_tag(t): return tag(t, "tier " + TIER_CLS[t])
def verds(d):
    return '<span class="verds">' + "".join(f'<i class="{ {"成立":"y","不明":"u","不成立":"n"}[verdict(d[k])] }" title="{n}：{verdict(d[k])}"></i>' for k, n in DIMS) + "</span>"
def heat_html(e):
    m = e["meta"]
    if not m: return '<span class="muted">—</span>'
    if not m.get("likes"): return f'<span class="muted">账号不公开点赞</span>'
    r = f' · 常态 {m["acct_median"]} 的 {e["heat"]:.1f}×' if e["heat"] else ""
    return f'{m["likes"]} 赞 · {m.get("comments",0)} 评{E(r)}'
def tags_html(e, n=4):
    m = e["meta"]; ts = m.get("tags") or []
    low = {"营业时间", "选品", "举办日", "报名日", "开票日"}; pri = {"新品", "事件", "装备"}
    h = "".join(tag(t, "low" if t in low else "pri" if t in pri else "") for t in ts[:n])
    if len(ts) > n: h += tag(f"+{len(ts)-n}", "muted")
    if not ts: h = tag("无标签", "muted")
    return h
def thumbs(e, n=2, size=48):
    return "".join(f'<img src="{t}" width="{size}" height="{size}" alt="" loading="lazy">' for t in e["thumbs"][:n]) or f'<span class="noimg" style="width:{size}px;height:{size}px">无图</span>'
def materials(e):
    """对照材料五类"""
    pub = PUB_HIT.get(e["name"]); ex = EXAMPLE_HIT.get(e["name"]); dec = DECISION.get(e["shortcode"])
    return f'''<div><h4>对照材料 · 五类</h4><table class="mat">
<tr><th>正式已发布</th><td>{E(pub[0] + "（" + pub[1] + "）— " + pub[2]) if pub else "90 天内无同一事实"}</td></tr>
<tr><th>范例</th><td>{E(ex) if ex else "无相似范例"}</td></tr>
<tr><th>生成过文章的贴文</th><td>{"同 post_id 已生成过文章" if dec and dec[0]=="采用" else "无相似（示意）"}</td></tr>
<tr><th>03 决定</th><td>{(tag(dec[0], "ok" if dec[0]=="采用" else "warn") + " " + E(dec[1])) if dec else "无同事实决定"}</td></tr>
<tr><th>上一轮台账</th><td>昨日未判</td></tr></table></div>'''

# ---------- 判断台账 ----------
rows = ""; last_tier = None; i = 0
for e in EV:
    s = e["scores"]; i += 1
    if e["tier"] != last_tier:
        cnt = sum(1 for x in EV if x["tier"] == e["tier"])
        rows += f'<tr class="group"><td colspan="14">{E(e["tier"])} · {cnt} 条</td></tr>'; last_tier = e["tier"]
    pub = PUB_HIT.get(e["name"]); dec = DECISION.get(e["shortcode"])
    dedup = tag("同品牌 · 需判新增", "warn") if pub else tag("未发过", "ok")
    three = {"none": tag("三句话已答", "ok"), "missing_material": tag("缺资料", "warn"), "angle_not_formed": tag("角度未成立", "warn"), "low_value": tag("价值不足", "muted")}[s["unanswered"]]
    origin = tag("Van 链接", "van") if e["name"] == "gnuhr" else tag("系统", "muted")
    st = tag(f"03 已{dec[0]}", "ok" if dec[0] == "采用" else "warn") if dec else tag("已判", "muted")
    rows += f'''<tr class="row" data-i="{i}">
<td class="num">{i}</td>
<td class="th">{thumbs(e)}</td>
<td class="title"><b>{E(s["title"])}</b><span class="sub">@{E(e["account"])} · {E(str(e["date_posted"])[5:16].replace("T"," "))} · {e["n_images"]} 图</span></td>
<td>{tier_tag(e["tier"])}</td>
<td>{verds(s["dims"])}<span class="sub">{e["ok"]} / 6 成立</span></td>
<td class="heat">{heat_html(e)}</td>
<td class="tags">{tags_html(e)}</td>
<td>{tag(s["kind"])}</td>
<td>{dedup}</td>
<td>{three}</td>
<td class="num">{len(s["gaps"])}</td>
<td>{st}</td>
<td>{origin}</td>
<td class="act"><button>首批</button><button>待核</button><button>改档</button></td>
</tr>'''
    if i == 1:
        rows += f'''<tr class="expand"><td colspan="14"><div class="expand-grid">
<div><h4>六维度判定与依据</h4><ul>{"".join(f"<li><b>{n} · {verdict(s['dims'][k])}</b> · {E(s['evidence'][k])}</li>" for k,n in DIMS)}</ul></div>
<div><h4>三句话</h4><ol><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
<h4>热度与标签</h4><p>{heat_html(e)}<br>{tags_html(e, 12)}<br><span class="muted">话题：{E(" ".join(e["meta"].get("hashtags") or []) or "无")}</span></p><h4>缺口</h4><p>{E("、".join(s["gaps"]) or "无")}</p></div>
{materials(e)}
</div></td></tr>'''
hard = '''<tr class="group"><td colspan="14">硬性排除 · 2 条（示意，可捞回）<button class="link">展开</button></td></tr>
<tr class="row dim"><td class="num">—</td><td class="th"><span class="noimg">图</span></td><td class="title"><b>示意｜窗口外的贴文</b><span class="sub">首次入库早于上次运行 · 代码判定</span></td><td>{}</td><td></td><td></td><td></td><td>{}</td><td></td><td></td><td class="num">0</td><td>{}</td><td>{}</td><td class="act"><button>捞回</button></td></tr>
<tr class="row dim"><td class="num">—</td><td class="th"><span class="noimg">图</span></td><td class="title"><b>示意｜与 03 否决的同一事实同角度</b><span class="sub">对照 9/16 否决 · 模型判定</span></td><td>{}</td><td></td><td></td><td></td><td>{}</td><td></td><td></td><td class="num">0</td><td>{}</td><td>{}</td><td class="act"><button>捞回</button></td></tr>'''.format(
    tag("排除", "dim"), tag("其他"), tag("硬性排除", "dim"), tag("系统", "muted"), tag("排除", "dim"), tag("新品发布"), tag("硬性排除", "dim"), tag("系统", "muted"))
n_rec = sum(1 for e in EV if e["tier"] == "推荐")
board = f'''
<div class="toolbar">
  <div class="filters">{tag("档：全部")}{tag("类型：全部")}{tag("对照：全部")}{tag("三句话：全部")}{tag("来源：全部")}{tag("标签：全部")}{tag("只看有图")}</div>
  <div class="right"><label>分组 <select><option>按档</option><option>按类型</option><option>按账号</option></select></label><label>排序 <select><option>档 → 成立维度数 → 热度</option><option>热度</option><option>发布时间</option></select></label><button class="primary">提交首批（{n_rec} 条推荐）</button></div>
</div>
<div class="table-wrap"><table class="board">
<thead><tr><th>#</th><th>图</th><th>事件</th><th>档</th><th>六维度</th><th>热度</th><th>标签</th><th>类型</th><th>对照</th><th>三句话</th><th>缺口</th><th>状态</th><th>来源</th><th>操作</th></tr></thead>
<tbody>{rows}{hard}</tbody></table></div>
<p class="foot">窗口 09-17 → 09-18 · 图文贴文 305 · 已判断 305（100%）· 推荐 14 · 备选 41 · 待核 9 · 不推荐 241 · 硬性排除 6 · 本页显示实测过的 {len(EV)} 条与 2 条示意；六维度判定由早期打分实验换算，属示意</p>'''

# ---------- Van 模式 ----------
van_cards = ""
for e in [x for x in EV if x["tier"] in ("推荐", "备选")][:6]:
    s = e["scores"]
    van_cards += f'''<div class="vcard">{thumbs(e, 2, 120)}<div class="vbody"><b>{E(s["title"])}</b><span class="sub">@{E(e["account"])} · {tier_tag(e["tier"])} · {heat_html(e)}</span>
<ol class="three"><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
<div class="vact"><button class="ok">采用</button><button class="no">否决</button><button>暂缓</button><input placeholder="一句话原话（选填）"></div></div></div>'''

# ---------- 事件详情 ----------
d = EV[0]; s = d["scores"]; dm = d["meta"]
detail = f'''
<div class="detail">
<div class="col">
  <h3>图集 · {d["n_images"]} 张（每张带描述与用途）</h3><div class="gallery">{thumbs(d, 2, 160)}<span class="noimg" style="width:160px;height:160px">其余 {max(d["n_images"]-2,0)} 张懒加载</span></div>
  <table class="plain"><tr><td>图 1</td><td>示意：产品正面全貌，对应正文的产品名 · 产品图 · 可作配图</td></tr><tr><td>图 2</td><td>示意：面料细节，对应正文「实验室检测」· 细节图 · 可作配图</td></tr></table>
  <h3>合并的贴文 · 1 条</h3><div class="post"><b>@{E(d["account"])}</b> <span class="sub">{E(str(d["date_posted"])[:16].replace("T"," "))} · 粉丝 {dm.get("followers","—")} · {heat_html(d)}</span><p>{E(d["caption"][:400])}</p><a>{E(d["url"])}</a></div>
  <p>标签：{tags_html(d, 12)}<br><span class="muted">话题：{E(" ".join(dm.get("hashtags") or []) or "无")}</span></p>
  <h3>披露时间</h3><p>原始披露：{E(str(d["date_posted"])[:10])}（品牌官方贴）· 转载：无</p>
</div>
<div class="col">
  <h3>结论 {tier_tag(d["tier"])} · {d["ok"]} / 6 成立</h3>
  <table class="dims">{"".join(f'<tr><th>{n}</th><td>{tag(verdict(s["dims"][k]), {"成立":"ok","不明":"warn","不成立":"muted"}[verdict(s["dims"][k])])}</td><td class="ev">{E(s["evidence"][k])}</td></tr>' for k,n in DIMS)}</table>
  <h3>三句话</h3><ol class="three"><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
  <h3>热度与标签</h3><p>{heat_html(d)}<br>{tags_html(d, 12)}</p>
  <h3>命中 · 缺口</h3><p>{"".join(tag(h,"pri") for h in s["priority_hits"])}{"".join(tag(h,"low") for h in s["lower_hits"])}</p><p>缺口：{E("、".join(s["gaps"]) or "无")}</p>
</div>
<div class="col">
  {materials(d).replace('<div><h4>', '<h3>').replace('</h4>', '</h3>').replace('</table></div>', '</table>')}
  <h3>深核结果</h3><div class="ref"><b>待深核</b><span class="sub">推荐档由 Codex App Server 核原始来源、比较参照与完整图集；首批限 4–8 条、10 分钟</span></div>
  <h3>审计</h3><table class="audit"><tr><td>识别</td><td>视觉模型 · {d["n_images"]} 张</td><td>—</td><td>—</td></tr><tr><td>判断</td><td>gpt-6-astra · 准则 v0</td><td>{d["secs"]} 秒</td><td>{(d.get("usage") or {}).get("inputTokens","—")} tok</td></tr><tr><td>对照材料</td><td>LanceDB 召回 38 · 重排 8</td><td>2.1 秒</td><td>—</td></tr></table>
  <div class="actions"><button class="primary">登记条目</button><button>标待核</button><button>改档</button></div>
</div></div>'''

# ---------- 今日总览 ----------
overview = '''
<div class="strip-head"><span>开工方式 <b>任务驱动</b></span><span>期次 <b>r48</b> · 任务 <b>#611</b></span><span>窗口 <b>09-17 → 09-18（UTC）</b></span><span>距首批时限 <b>12 分钟</b></span><span class="right"><button>手动开启一轮</button><button class="link">轮次列表（2）</button></span></div>
<div class="steps strip"><span class="done">0 派单 · 05:30</span><span class="done">1 接单 · 心跳中</span><span class="done">2 采集媒体信息 · 3 采集器 · 383 → 图文 305 · 图 1,727 识别 1,727</span><span class="done">3 合并 · 新入库 182 · 事件 160</span><span class="done">4 对照材料 · 160 / 160 · 知识库到 09-20</span><span class="done">5 逐条判断 · 160 / 160 · 推荐 14 · 待核 9</span><span class="doing">6 深核 · 6 / 8（时间盒 10 分）</span><span>7 登记与交付</span><span>8 主编校准</span><span>9 补件</span></div>
<div class="cards">
 <div class="card"><span class="k">贴文全量</span><b>383</b><span class="s">库内 383 · 一致</span></div>
 <div class="card"><span class="k">只图文</span><b>305</b><span class="s">过滤含视频的 78 条</span></div>
 <div class="card"><span class="k">图片识别</span><b>1,727 / 1,727</b><span class="s">未识别 0 · 由代码计数</span></div>
 <div class="card"><span class="k">已判断</span><b>100%</b><span class="s">160 / 160 事件 · 每条都判</span></div>
</div>
<div class="grid2">
 <div class="panel"><h3>告警</h3><ul class="alerts"><li class="warn">小红书采集器：opencli 超时 2 次，已重试成功（非必扫）</li><li class="warn">对照材料缺：0 条 · 知识库同步到 09-20，生成过文章的贴文同步到 09-21 01:20</li><li>向量服务：入库 1,204 个向量 · 8 分 40 秒 · 无失败</li><li class="warn">杂志背景：GO-OUT-2026.03 清单已写 3 小时仍未入库</li><li>模型调用失败 1 次（429），重试成功</li></ul></div>
 <div class="panel"><h3>值得主编看一眼</h3><ul class="alerts"><li>不推荐档里热度是账号常态 3 倍以上的：2 条</li><li>Van 链接：1 条已进推荐档并优先深核</li><li>与 03 否决同事实被硬性排除的：1 条（可捞回）</li><li>待补录账号：4 个（来自本期候选与 Van 链接）</li></ul></div>
</div>'''

# ---------- 采集与覆盖 ----------
harvest = '''
<div class="panel"><h3>采集方案 · 日更采集（v3，本轮）</h3><div class="table-wrap"><table>
<thead><tr><th>采集器</th><th>来源</th><th>必扫</th><th>取到</th><th>只图文</th><th>图片</th><th>已识别</th><th>耗时</th><th>结果</th></tr></thead>
<tbody>
<tr><td>csw 贴文窗口</td><td>agent.campsomewhere.com · posts/window</td><td>是</td><td>383</td><td>305（过滤 78）</td><td>1,727</td><td><b>1,727</b></td><td>取 2.1 秒 · 下 3 分 · 识别 6 分</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>小红书</td><td>opencli · 关键词 12 个</td><td>否</td><td>37</td><td>37</td><td>112</td><td><b>112</b></td><td>4 分</td><td><span class="tag warn">partial · 超时 2 次后成功</span></td></tr>
<tr><td>网页 / RSS</td><td>GO OUT</td><td>否</td><td>14</td><td>14</td><td>31</td><td><b>31</b></td><td>40 秒</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>网页 / RSS</td><td>CAMP HACK</td><td>否</td><td>11</td><td>11</td><td>26</td><td><b>26</b></td><td>35 秒</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>Van 链接</td><td>工作台粘贴 · 转 csw 单条</td><td>—</td><td>1</td><td>1</td><td>5</td><td><b>5</b></td><td>6 秒</td><td><span class="tag van">Van 链接 · 单独统计</span></td></tr>
</tbody></table></div><p class="muted">数量都由代码计数；含视频的贴文在采集器里过滤，数量单列；每条候选带点赞、评论、粉丝、标签、话题。</p></div>
<div class="grid2">
<div class="panel"><h3>来源台账</h3><table class="plain"><tr><td>Instagram 贴文库</td><td>必扫</td><td class="ok">01:00 已抓取 · 182 条入库（01:22）</td></tr><tr><td>小红书</td><td>普通</td><td class="ok">09-21 05:41</td></tr><tr><td>品牌官网 / 媒体</td><td>普通</td><td class="ok">09-21 05:44</td></tr><tr><td>GO OUT · CAMP HACK</td><td>普通（Van 补充，不加权）</td><td class="ok">09-21 05:44</td></tr></table></div>
<div class="panel"><h3>待补录账号 · 4</h3><table class="plain"><tr><td>@moss_tents</td><td>Van 9/18 指定链接 · MOSS THE EDEN</td><td><button>导出</button></td></tr><tr><td>@carharttwip</td><td>Van 9/18 指定 · 刊发库 vol.260</td><td></td></tr><tr><td>@wearebraindead</td><td>Van 9/18 指定 · Brain Dead × FIVE TEN</td><td></td></tr><tr><td>@heimplanet（全球号）</td><td>在册的是 heimplanet_japan · Van 9/18 指定</td><td></td></tr></table><p class="muted">补录在 csw 后台完成；清单每周汇总一次。</p></div>
</div>
<div class="panel"><h3>图片</h3><table class="plain"><tr><td>下载</td><td class="ok">1,901 / 1,901 成功</td></tr><tr><td>识别</td><td class="ok">1,901 / 1,901 · 可作配图 1,612</td></tr><tr><td>向量</td><td class="ok">图文融合 338 · 图片 1,901 · 入库 8 分 40 秒</td></tr><tr><td>未识别</td><td>0（失败标「未识别」，不算看过）</td></tr></table></div>'''

pending = '''
<div class="panel"><h3>待核结转 · 9</h3><div class="table-wrap"><table>
<thead><tr><th>事件</th><th>缺口</th><th>首次出现</th><th>剩余期数</th><th>上次重判</th><th>操作</th></tr></thead>
<tbody>
<tr><td><b>Helinox｜复写七八十年代露营风格</b><span class="sub">@helinox_kr</span></td><td>缺发售信息与具体产品清单；文案只有背景</td><td>r48</td><td>5</td><td>—</td><td><button>补证后重判</button><button>关闭</button></td></tr>
<tr><td><b>FUTURE FOX｜炉顶附件 10 月下旬新型</b><span class="sub">@futurefox</span></td><td>缺官方页与价格</td><td>r47</td><td>3</td><td>r48：仍缺价格</td><td><button>补证后重判</button><button>关闭</button></td></tr>
<tr><td><b>New Balance｜Flying 77 与石</b><span class="sub">@newbalance</span></td><td>图不足（只有 1 张海报）</td><td>r47</td><td>3</td><td>r48：无新图</td><td><button>补证后重判</button><button>关闭</button></td></tr>
<tr><td><b>示意｜未识别的图</b><span class="sub">下载失败</span></td><td>未读到实图，不算看过</td><td>r48</td><td>7</td><td>—</td><td><button>重试下载</button></td></tr>
</tbody></table></div><p class="muted">当期截止前未补齐不阻塞已批准选题；到期自动关闭并写明原因，不自动入选。</p></div>'''

kb = '''
<div class="toolbar"><input class="search" value="Brompton" placeholder="输入品牌、产品、关键词，或拖一张图"><span class="tag">向量 + 全文（jieba）召回 38 · 重排 8</span><span class="tag">品牌别名：小布 · ブロンプトン</span></div>
<div class="grid2">
<div class="panel"><h3>检索结果 · 按五类分组</h3>
<h4>正式已发布 · 2</h4>
<div class="ref"><b>and wander × Brompton：骑小布，有了一套官方穿法 · vol.248</b><span class="sub">2026-09-02 · <span class="tag ok">正式已发布</span> · 拆条 5 · 本条：联名骑行服与折叠车</span></div>
<div class="ref"><b>Carhartt WIP × OMNIUM：把工装骑上街 · vol.260</b><span class="sub">2026-09-18 · <span class="tag ok">正式已发布</span> · 货运自行车联名</span></div>
<h4>范例 · 1</h4><div class="ref"><b>vol.248 条目 1 · and wander × Brompton</b><span class="sub">选题对象：联名骑行装与折叠车 · 切口：通勤与山野切换 · 六维度全部成立</span></div>
<h4>生成过文章的贴文 · 0</h4><p class="muted">1,911 条里无 Brompton</p>
<h4>03 决定 · 0</h4><p class="muted">无同品牌决定</p>
<h4>上一轮台账 · 1</h4><p class="muted">r47：ROBIN 门店 Brompton 陈列 · 备选</p>
<h3>品牌页 · Brompton</h3><p class="muted">近 30 天：1 篇 · 近 90 天：1 篇 · 角度：联名骑行装、通勤与户外切换</p></div>
<div class="panel"><h3>同步与索引</h3><table class="plain"><tr><td>刊发记录</td><td>同步到 2026-09-20 · 正式已发布 261 篇 · 拆条 1,412</td></tr><tr><td>生成过文章的贴文</td><td>1,911 条 · 9,373 图 · 09-21 01:20 同步</td></tr><tr><td>范例</td><td>10 篇 · 41 条 · 225 图 · 拆解与图片说明完成</td></tr><tr><td>03 决定</td><td>37 条（含否决 21）· 实时</td></tr><tr><td>LanceDB 索引</td><td>向量 22,140 · 176 MB · 全文 jieba</td></tr><tr><td>向量服务</td><td class="ok">Qwen3-VL · 健康 · 队列 0</td></tr><tr><td>缺口</td><td>群发类 3 篇公开页抓取失败 · 已列清单</td></tr></table>
<h3>十篇范例</h3><table class="plain"><tr><td>09-18</td><td>Carhartt WIP × OMNIUM：把工装骑上街 vol.260</td><td><span class="tag ok">5 条 · 25 图</span></td></tr><tr><td>09-16</td><td>BRUNT：用装备，唱情歌 专题 Vol.12</td><td><span class="tag ok">4 章 · 16 图</span></td></tr><tr><td>09-11</td><td>Apple Ultra 4：够不够让户外人换表？vol.255</td><td><span class="tag ok">5 条 · 18 图</span></td></tr><tr><td>09-03</td><td>1983年，数字游民OG把办公室装上单车（贴图文）</td><td><span class="tag ok">1 条 · 22 图</span></td></tr><tr><td colspan="3" class="muted">…共 10 篇、41 条、225 图；待核推断 65 条单独标出</td></tr></table></div>
</div>'''

memory = '''
<div class="toolbar"><div class="filters"><span class="tag">身份：全部</span><span class="tag">线：选题偏好</span><span class="tag">状态：生效</span></div><div class="right"><button class="primary">进入 Van 校准模式</button></div></div>
<div class="panel"><h3>选题准则卡</h3><div class="table-wrap"><table>
<thead><tr><th>准则</th><th>身份</th><th>原话 · 日期</th><th>对象与适用条件</th><th>校准</th></tr></thead>
<tbody>
<tr><td>这件事有没有足够具体的变化，值得 CSW 帮读者解释一次</td><td><span class="tag ok">本人确认</span></td><td>9/19 反馈</td><td>全部选题 · 核心判断</td><td>—</td></tr>
<tr><td>品牌知名度不是首要标准；小品牌解决具体问题可能更值得报</td><td><span class="tag ok">本人确认</span></td><td>9/19 反馈</td><td>全部选题</td><td>—</td></tr>
<tr><td>非新品不自动淘汰；新品不自动入选</td><td><span class="tag ok">本人确认</span></td><td>9/19 反馈 二·2</td><td>全部选题</td><td>—</td></tr>
<tr><td>国外新开店铺，除非非常特别，对国内读者价值不足</td><td><span class="tag">具体案例</span></td><td>「4国外新开店铺除非非常特别否则对于国内读者来说价值不足」· 9/16</td><td>当时第 4 条 · 门店类；9/18 她自选 SATISFY 巴黎旗舰店可对照</td><td><button>对</button><button>不对</button><button>改</button></td></tr>
<tr><td>被打回的同一事实同一角度不再重复提报</td><td><span class="tag ok">本人确认</span></td><td>「其他全部打回并不要在之后的选题里重复出现」· 9/16</td><td>被否条目 · 落为硬性排除</td><td>—</td></tr>
<tr><td>图片不足的先待核，核实够用再上</td><td><span class="tag">具体案例</span></td><td>「5可保留但需要验证图片是否足够」· 9/16</td><td>当时第 5 条 Springbar</td><td><button>对</button><button>不对</button><button>改</button></td></tr>
<tr><td>联名若只有 logo 叠加、无使用信息，优先级低</td><td><span class="tag warn">模型推断</span></td><td>从 9/16、9/18 两次否决归纳</td><td>联名类 · 待校准</td><td><button>对</button><button>不对</button><button>改</button></td></tr>
</tbody></table></div></div>
<div class="grid2">
<div class="panel"><h3>案例库 · 最近</h3><div class="ref"><b>9/18 · 退回两条联名，改用自选 8 条</b><span class="sub">White Mountaineering × MOONSTAR、SOUTH2 WEST8 × BEN MILLER → 否决（未给理由）</span></div><div class="ref"><b>9/16 · 七条退六条</b><span class="sub">1 非新品；2、3、6、7 产品价值不足；4 国外新店；5 保留待核图片</span></div><div class="ref"><b>9/15 · 三条全退</b><span class="sub">「三个产品的报道价值都不高；选题数目太少」</span></div></div>
<div class="panel"><h3>回收覆盖清单</h3><table class="plain"><tr><td>主编会话库</td><td>私聊 62 · 群 247</td><td class="ok">已回收</td></tr><tr><td>文案会话库</td><td>私聊 91 · 群 29</td><td class="ok">已回收（写作表达线）</td></tr><tr><td>其余七个会话库</td><td>私聊 0–6 · 群 2–49</td><td class="ok">已回收</td></tr><tr><td>飞书群完整记录</td><td>—</td><td class="warn">待权限</td></tr><tr><td>身份映射</td><td>九个应用下的 open_id</td><td class="warn">待做</td></tr></table></div>
</div>'''

backtest = "".join(f'<tr><td>{E(s["scores"]["title"])}</td><td>{E(DECISION[s["shortcode"]][0])}</td><td>{tier_tag(s["tier"])} · {s["ok"]} / 6 成立</td></tr>' for s in EV if s["shortcode"] in DECISION)
rubric = f'''
<div class="grid2">
<div class="panel"><h3>六个维度与锚点（引用范例与被否案例）</h3><table class="plain rub">
<tr><th>具体变化</th><td>成立：Carhartt WIP × OMNIUM 车架由铬钼钢改为透明涂层不锈钢、加联名图形与特别版脚踏（范例 vol.260）· 不明：只有配色 · 不成立：补货、再贩、无变化</td></tr>
<tr><th>使用关联</th><td>成立：and wander × Brompton 骑行与山野的切换（范例 vol.248）· 不成立：与读者怎么用、穿、去无关</td></tr>
<tr><th>信息增量</th><td>成立：限量 150 台、9 月 24 日发售、含税 4,500 欧元（范例 vol.260）· 不成立：只有名字</td></tr>
<tr><th>比较参照</th><td>成立：联名车架与标准版材质对照（范例 vol.260）· 不明：无参照但可补 · 不成立：编造参照</td></tr>
<tr><th>值得解释</th><td>成立：裸露焊道、酸性配色、货架与脚踏各自表达什么（范例 vol.260）· 不成立：只能说好看</td></tr>
<tr><th>调性适配</th><td>成立：工装、城市货运骑行、BMX 细节构成城市户外选题 · 不成立：与户外生活方式无关；被否：S2W8 × BEN MILLER（9/18）</td></tr>
</table><p class="muted">每级锚点引用范例条目或被否案例；新增或修改写审计。品牌知名度不是维度；热度与标签是输入信号，不是维度。</p></div>
<div class="panel"><h3>准则版本</h3><table class="plain"><tr><th>版本</th><th>内容</th><th>生效</th><th>谁 · 为什么</th></tr><tr><td>v0</td><td>Van 9/19 框架六维度 + 三句话 + 优先 / 降低七类</td><td>r49 起</td><td>开发 · 初始</td></tr><tr><td>v1 草案</td><td>锚点补入十篇范例的 41 条拆解</td><td>—</td><td>开发 · 范例建成后</td></tr></table>
<h3>回测 · Van 有态度的条目落在哪一档</h3><table class="plain"><tr><th>条目</th><th>Van</th><th>当前准则</th></tr>{backtest}</table><p class="muted">样本只有三条，只用于提示维度或锚点可能还没对上她的取舍；两条否决若落在推荐 / 备选，说明「联名」的锚点要收紧。重复判断一致性：待 M3 检验。</p></div>
</div>'''

metrics = '''
<div class="cards">
 <div class="card"><span class="k">自主发现情况</span><b>0 / 7</b><span class="s">r48 基线：采用条目里系统先于 Van 发现的</span></div>
 <div class="card"><span class="k">第一轮推荐质量</span><b>2 → 0</b><span class="s">r48：首轮推荐 2 条，采用 0，随后她给 8 条链接</span></div>
 <div class="card"><span class="k">Van 的实际投入</span><b>约 45 分钟</b><span class="s">r48：审题 + 补链接 + 说明（估算）</span></div>
 <div class="card"><span class="k">审阅覆盖</span><b>12% → 100%</b><span class="s">r48 → 并行期目标：每条都判</span></div>
</div>
<div class="panel"><h3>按期次</h3><div class="chart"><div class="axis"></div><svg viewBox="0 0 720 200" role="img" aria-label="自主发现比例按期次折线，r48 为 0，之后为目标示意" style="max-width:100%;height:auto"><polyline points="40,170 200,170 360,120 520,90 680,60" fill="none" stroke="#4F46E5" stroke-width="2"/><g font-size="12" fill="currentColor" opacity=".7"><text x="40" y="192">r48</text><text x="200" y="192">r49</text><text x="360" y="192">r50</text><text x="520" y="192">r51</text><text x="680" y="192">r52</text><text x="8" y="24">100%</text><text x="8" y="174">0%</text></g><line x1="40" y1="20" x2="40" y2="176" stroke="currentColor" opacity=".3"/><line x1="40" y1="176" x2="700" y2="176" stroke="currentColor" opacity=".3"/></svg><p class="muted">r48 为真实基线；r49 起为目标走势示意。数值目标按实测再定。</p></div></div>'''

ops = '''
<div class="cards">
 <div class="card"><span class="k">采集服务</span><b class="ok">运行中</b><span class="s">Rust v0.1.0 · 上次任务轮 05:31 开工 · 手动轮 1</span></div>
 <div class="card"><span class="k">LanceDB</span><b class="ok">正常</b><span class="s">向量 22,140 · 176 MB · 最近写入 05:52</span></div>
 <div class="card"><span class="k">Qwen3-VL 向量 · 重排</span><b class="ok">健康</b><span class="s">embedder / reranker 已加载 · 串行队列 0</span></div>
 <div class="card"><span class="k">Codex App Server · csw 后端</span><b class="ok">就绪</b><span class="s">固定版本 · 独立 CODEX_HOME · posts/window 2.1 秒</span></div>
</div>
<div class="grid2">
<div class="panel"><h3>模型调用 · 本轮</h3><table class="plain"><tr><th>阶段</th><th>次数</th><th>中位耗时</th><th>token</th><th>失败 / 重试</th></tr><tr><td>图片识别</td><td>1,901 张（318 批）</td><td>4 秒 / 张</td><td>1.1k</td><td>0 / 0</td></tr><tr><td>向量化</td><td>2,239</td><td>0.42 秒 / 张</td><td>—</td><td>0 / 0</td></tr><tr><td>重排</td><td>160 × 38 候选</td><td>4.6 秒 / 条</td><td>—</td><td>0 / 0</td></tr><tr><td>逐条判断</td><td>160</td><td>18 秒</td><td>6.8k</td><td>1 / 1</td></tr><tr><td>深核</td><td>8</td><td>2 分 10 秒</td><td>31k</td><td>0 / 0</td></tr></table><p class="muted">实测参考：本机 8 条样本每回合 11–22 秒；图片识别 225 张 46 批约 20 分钟（3 并行）；生产用独立 CODEX_HOME 后底座上下文会明显低于本机的 2 万 token。</p></div>
<div class="panel"><h3>采集方案</h3><table class="plain"><tr><th>方案</th><th>采集器</th><th>版本</th><th></th></tr><tr><td>日更采集（01）</td><td>csw 贴文窗口（必扫，只图文）· 小红书 · GO OUT · CAMP HACK · Van 链接</td><td>v3 · r49 起 · 开发</td><td><button>编辑</button></td></tr><tr><td>条目取单条（05 / 11 / 补件）</td><td>csw 单条（原图）</td><td>v1</td><td><button>编辑</button></td></tr><tr><td>自定义</td><td>外部命令 / HTTP / MCP 工具</td><td>—</td><td><button>新建</button></td></tr></table>
<h3>手动开启一轮</h3><table class="plain"><tr><td>方案</td><td><select><option>日更采集</option><option>条目取单条</option></select></td></tr><tr><td>窗口</td><td><input class="search" value="2026-09-17 → 2026-09-18（UTC）"></td></tr><tr><td>关联任务</td><td><select><option>不关联（只出本地报告）</option><option>r48 · 任务 #611（已接单）</option></select></td></tr><tr><td></td><td><button class="primary">开始</button> <span class="muted">发起人、参数、时间写审计</span></td></tr></table>
<h3>设置</h3><table class="plain"><tr><td>窗口判定</td><td>首次入库时间在上次运行之后 · 发布时间 3 天内</td></tr><tr><td>并行</td><td>识别 3 · 判断 3 · 深核 2</td></tr><tr><td>首批深核</td><td>前 8 条 · 10 分钟时间盒</td></tr><tr><td>待核结转期限</td><td>7 期</td></tr><tr><td>准则版本</td><td>v0（改动写审计）</td></tr><tr><td>回放</td><td><button>选历史窗口重跑</button> <span class="muted">可指定知识库版本</span></td></tr></table></div>
</div>'''

NAV = [("overview", "今日总览"), ("board", "判断台账"), ("van", "Van 模式"), ("detail", "事件详情"), ("harvest", "采集与覆盖"), ("pending", "待核结转"), ("kb", "知识库"), ("magazine", "杂志条目"), ("memory", "选题记忆"), ("rubric", "判断框架与锚点"), ("metrics", "指标"), ("ops", "运行与设置")]
NOTES = {
 "overview": "主编开工后第一眼看的页。顶部流程带十格，每格状态、计数、耗时，点开看产物与重跑；四个数字都由代码计数；告警只列需要人处理的；可手动开启一轮。",
 "board": "核心页。窗口内每条图文贴文都判，按档分组，不推荐照样列出、随时捞回；没有综合分和权重；展开行看逐维依据、三句话、热度与标签、缺口、对照材料五类。",
 "van": "只给 Van 看的精简视图：图、标题、三句话、结论档与三个按钮。她的标记即时进入记忆库并在主编视图显示；正式决定仍经引擎 03 闸。",
 "detail": "三栏：左原料（图集带描述与用途、贴文、热度、标签、披露时间），中判断（结论档、逐维判定、三句话、缺口），右对照材料五类、深核与审计。",
 "harvest": "回答「跑了哪些采集器、取到几条、识别了几张」。只图文与过滤数单列；数量由代码计数；Van 补充的来源标为普通来源；待补录账号带依据。",
 "pending": "证据或图片不足的不永久淘汰：写明缺口、结转期数、上次重判结果；到期关闭要写原因。",
 "kb": "向量 + 全文混合召回；范围切换「判断参考库（四类）/ 杂志背景」，默认参考库；文字搜与以图搜两入口；参考库结果按类分组，杂志结果带缩略图与刊期页码；品牌页分「CSW 历史覆盖」与「杂志里出现过」两节；同步与回填状态常驻。",
 "magazine": "杂志条目详情（新页）：裁图大图、整页图上框出位置、商品表、描述与译文、出处，复制 OSS 地址，以这张图再搜。杂志数据为 GO OUT 2016.08 真实清单。",
 "memory": "每条准则标明身份（具体案例 / 模型推断 / 本人确认）与原话；Van 校准模式逐条对不对；回收覆盖清单如实列缺口。",
 "rubric": "维度定义直接取自 Van 的框架；每级锚点引用范例条目与被否案例；准则带版本；用她的决定回测落在哪一档。没有权重。",
 "metrics": "先看 Van 的三项目标指标，再看技术指标；她提供链接后的处理结果单独统计。",
 "ops": "开发用：服务与依赖状态（LanceDB、向量服务、Codex、csw 后端）、模型调用与费用、采集方案、手动开启、设置与回放。",
}
SCREENS = {"overview": overview, "board": board, "van": f'<div class="vgrid">{van_cards}</div>', "detail": detail, "harvest": harvest, "pending": pending, "kb": (MZ.kb_screen(MAG) if MAG else kb), "magazine": (MZ.detail_screen(MAG) if MAG else "<p>缺 magazine_sample.json</p>"), "memory": memory, "rubric": rubric, "metrics": metrics, "ops": ops}
TITLES = dict(NAV)
sections = "".join(f'<section class="screen" id="s-{k}" {"" if k=="overview" else "hidden"}><div class="shead"><div><h2>{TITLES[k]}</h2><p class="note">{E(NOTES[k])}</p></div><span class="crumb">r48 · 任务驱动 · 窗口 09-17 → 09-18（UTC）</span></div>{SCREENS[k]}</section>' for k in SCREENS)
navs = "".join(f'<a href="#" data-s="{k}" class="{"on" if k=="overview" else ""}">{n}</a>' for k, n in NAV)

page = f'''<title>收集员工作台界面设计稿</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400;500;700&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root {{ --ground:#F5F6F8; --surface:#FFFFFF; --ink:#1B1F2A; --muted:#6B7280; --rule:#E5E7EB; --accent:#4F46E5; --accent-soft:#EEF0FD; --ok:#1F8A5B; --ok-soft:#E4F5EC; --warn:#B45309; --warn-soft:#FDF1DC; --van:#8A3FA8; --van-soft:#F3E8F9; --dim:#9CA3AF;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif; --mono:"IBM Plex Mono",Menlo,monospace; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5CCB93; --ok-soft:#173327; --warn:#F2B96B; --warn-soft:#3A2A12; --van:#C48BE0; --van-soft:#33203D; --dim:#6B7280; }} }}
:root[data-theme="dark"] {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5CCB93; --ok-soft:#173327; --warn:#F2B96B; --warn-soft:#3A2A12; --van:#C48BE0; --van-soft:#33203D; --dim:#6B7280; }}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:13.5px; line-height:1.6; padding-inline:16px; padding-block:16px 60px; }}
.frame {{ max-width:1480px; margin-inline:auto; background:var(--surface); border:1px solid var(--rule); border-radius:10px; overflow:hidden; display:grid; grid-template-columns:212px minmax(0,1fr); min-height:900px; }}
aside {{ border-right:1px solid var(--rule); background:var(--surface); display:flex; flex-direction:column; }}
.brand {{ display:flex; align-items:center; gap:10px; padding:16px 18px; border-bottom:1px solid var(--rule); }}
.brand i {{ width:30px; height:30px; border-radius:8px; background:var(--accent); display:inline-block; }}
.brand b {{ display:block; font-size:14px; }} .brand span {{ font-size:11.5px; color:var(--muted); }}
.navh {{ font-size:11px; color:var(--muted); letter-spacing:.08em; padding:14px 18px 6px; }}
nav.side a {{ display:block; padding:8px 14px; margin:2px 8px; border-radius:6px; color:var(--ink); text-decoration:none; font-size:13.5px; }}
nav.side a.on {{ background:var(--accent-soft); color:var(--accent); font-weight:600; }}
nav.side a:hover {{ background:var(--ground); }}
.role {{ margin-top:auto; padding:14px 18px; border-top:1px solid var(--rule); font-size:12px; color:var(--muted); }}
main {{ min-width:0; }}
.topbar {{ display:flex; justify-content:space-between; align-items:center; padding:10px 22px; border-bottom:1px solid var(--rule); font-size:13px; gap:12px; flex-wrap:wrap; }}
.topbar .pill {{ border:1px solid var(--rule); border-radius:999px; padding:2px 10px; margin-left:8px; color:var(--muted); }}
.screen[hidden] {{ display:none; }}
.screen {{ padding:20px 22px 36px; display:flex; flex-direction:column; gap:16px; }}
.shead {{ display:flex; justify-content:space-between; gap:16px; align-items:flex-start; flex-wrap:wrap; }}
.shead h2 {{ margin:0; font-size:20px; }} .note {{ margin:4px 0 0; color:var(--muted); max-width:62em; }}
.crumb {{ font-family:var(--mono); font-size:12px; color:var(--muted); white-space:nowrap; }}
.cards {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(200px,1fr)); gap:12px; }}
.card {{ border:1px solid var(--rule); border-radius:8px; padding:14px 16px; display:flex; flex-direction:column; gap:2px; background:var(--surface); }}
.card .k {{ font-size:12.5px; color:var(--muted); }} .card b {{ font-size:26px; font-weight:700; line-height:1.2; }} .card .s {{ font-size:12px; color:var(--muted); }} .card b.ok {{ color:var(--ok); font-size:18px; }}
.panel {{ border:1px solid var(--rule); border-radius:8px; padding:14px 16px; background:var(--surface); min-width:0; }}
.panel h3 {{ margin:0 0 10px; font-size:14px; }} .panel h3:not(:first-child) {{ margin-top:16px; }} .panel h4 {{ margin:12px 0 6px; font-size:12.5px; color:var(--accent); }}
.grid2 {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(360px,1fr)); gap:12px; }}
.strip-head {{ display:flex; flex-wrap:wrap; gap:6px 18px; font-size:13px; color:var(--muted); align-items:center; }} .strip-head b {{ color:var(--ink); }} .strip-head .right {{ margin-left:auto; display:flex; gap:8px; }}
.steps {{ display:flex; flex-wrap:wrap; gap:6px; }} .steps span {{ padding:5px 10px; border-radius:6px; border:1px solid var(--rule); font-size:12.5px; color:var(--muted); }}
.steps .done {{ background:var(--ok-soft); color:var(--ok); border-color:transparent; }} .steps .doing {{ background:var(--accent-soft); color:var(--accent); border-color:transparent; font-weight:600; }}
.alerts {{ margin:0; padding-left:1.2em; display:flex; flex-direction:column; gap:6px; }} .alerts .warn {{ color:var(--warn); }}
.toolbar {{ display:flex; justify-content:space-between; gap:12px; flex-wrap:wrap; align-items:center; }}
.filters {{ display:flex; flex-wrap:wrap; gap:6px; }} .right {{ display:flex; gap:10px; align-items:center; flex-wrap:wrap; }}
label {{ font-size:12.5px; color:var(--muted); }} select, input.search, .vact input {{ font:inherit; font-size:12.5px; border:1px solid var(--rule); border-radius:6px; padding:4px 8px; background:var(--surface); color:var(--ink); }}
input.search {{ min-width:280px; }}
button {{ font:inherit; font-size:12px; border:1px solid var(--rule); background:var(--surface); color:var(--ink); border-radius:6px; padding:4px 9px; cursor:pointer; }}
button.primary {{ background:var(--accent); border-color:var(--accent); color:#fff; font-weight:600; }} button.link {{ border:0; color:var(--accent); background:none; }}
button.ok {{ background:var(--ok-soft); color:var(--ok); border-color:transparent; }} button.no {{ background:var(--warn-soft); color:var(--warn); border-color:transparent; }}
.tag {{ display:inline-block; font-size:11.5px; padding:1px 7px; border-radius:999px; border:1px solid var(--rule); color:var(--muted); margin:1px 2px 1px 0; white-space:nowrap; }}
.tag.ok {{ background:var(--ok-soft); color:var(--ok); border-color:transparent; }} .tag.warn {{ background:var(--warn-soft); color:var(--warn); border-color:transparent; }}
.tag.van {{ background:var(--van-soft); color:var(--van); border-color:transparent; }} .tag.pri {{ background:var(--accent-soft); color:var(--accent); border-color:transparent; }}
.tag.low {{ color:var(--warn); border-color:var(--warn-soft); }} .tag.pending {{ background:var(--warn-soft); color:var(--warn); border-color:transparent; }} .tag.dim, .tag.muted {{ color:var(--dim); }}
.tag.tier {{ font-weight:700; }} .tag.tier.rec {{ background:var(--accent); color:#fff; border-color:transparent; }} .tag.tier.alt {{ background:var(--accent-soft); color:var(--accent); border-color:transparent; }} .tag.tier.no {{ color:var(--dim); }}
.verds {{ display:inline-flex; gap:3px; }} .verds i {{ width:12px; height:12px; border-radius:3px; display:inline-block; background:var(--ground); border:1px solid var(--rule); }} .verds i.y {{ background:var(--ok); border-color:var(--ok); }} .verds i.u {{ background:var(--warn-soft); border-color:var(--warn); }} .verds i.n {{ background:var(--ground); }}
.table-wrap {{ overflow-x:auto; border:1px solid var(--rule); border-radius:8px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; }} th, td {{ text-align:left; vertical-align:top; padding:8px 10px; border-bottom:1px solid var(--rule); }}
th {{ font-size:12px; color:var(--muted); background:var(--ground); white-space:nowrap; font-weight:600; }} tr:last-child td {{ border-bottom:0; }}
table.plain th, table.plain td {{ padding:7px 8px; font-size:13px; }} table.plain td.ok {{ color:var(--ok); }} table.plain td.warn {{ color:var(--warn); }}
table.board td {{ font-size:12.5px; }} td.num {{ font-family:var(--mono); color:var(--muted); text-align:right; }} td.heat {{ font-size:12px; white-space:nowrap; }} td.tags {{ max-width:170px; }}
td.th {{ white-space:nowrap; }} td.th img, .noimg {{ border-radius:4px; object-fit:cover; margin-right:3px; vertical-align:middle; background:var(--ground); }}
.noimg {{ display:inline-flex; align-items:center; justify-content:center; font-size:11px; color:var(--dim); border:1px dashed var(--rule); }}
td.title b {{ display:block; font-size:13px; }} .sub {{ display:block; font-size:11.5px; color:var(--muted); }}
td.act {{ white-space:nowrap; }} td.act button {{ margin-right:4px; }}
tr.expand td {{ background:var(--ground); }} .expand-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(280px,1fr)); gap:14px; padding:6px 4px; }}
.expand-grid h4 {{ margin:0 0 6px; font-size:12.5px; color:var(--accent); }} .expand-grid ul, .expand-grid ol {{ margin:0; padding-left:1.2em; font-size:12.5px; }} .expand-grid p {{ margin:4px 0; font-size:12.5px; }}
table.mat th {{ background:none; width:7.5em; font-size:12px; color:var(--ink); white-space:normal; }} table.mat td {{ font-size:12px; }}
tr.group td {{ background:var(--ground); color:var(--ink); font-weight:600; font-size:12.5px; }} tr.dim td {{ opacity:.65; }}
.foot {{ color:var(--muted); font-size:12.5px; margin:0; }} .muted {{ color:var(--muted); }}
.vgrid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(420px,1fr)); gap:12px; }}
.vcard {{ border:1px solid var(--rule); border-radius:8px; padding:12px; display:flex; gap:12px; background:var(--surface); }} .vcard img {{ border-radius:6px; object-fit:cover; }} .vbody {{ min-width:0; flex:1; }} .vbody b {{ font-size:14px; }}
.three {{ margin:8px 0; padding-left:1.2em; font-size:12.5px; display:flex; flex-direction:column; gap:3px; }}
.vact {{ display:flex; gap:6px; flex-wrap:wrap; align-items:center; }} .vact input {{ flex:1; min-width:160px; }}
.detail {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(320px,1fr)); gap:14px; }} .col {{ border:1px solid var(--rule); border-radius:8px; padding:14px; background:var(--surface); min-width:0; }} .col h3 {{ margin:14px 0 8px; font-size:13.5px; }} .col h3:first-child {{ margin-top:0; }}
.gallery {{ display:flex; gap:6px; flex-wrap:wrap; }} .gallery img {{ border-radius:6px; object-fit:cover; }}
.post p {{ font-size:12.5px; white-space:pre-line; margin:6px 0; }} .post a {{ font-size:11.5px; color:var(--accent); word-break:break-all; }}
table.dims th {{ background:none; width:6em; font-weight:600; color:var(--ink); }} table.dims td.ev {{ font-size:12px; color:var(--muted); }}
.ref {{ border:1px solid var(--rule); border-radius:6px; padding:8px 10px; margin-bottom:8px; }} .ref b {{ font-size:12.5px; }} .ref p {{ margin:4px 0 0; font-size:12px; }}
table.audit td {{ font-size:12px; }} .actions {{ display:flex; gap:6px; margin-top:12px; }}
table.rub th {{ width:6em; white-space:normal; }}
.chart svg {{ display:block; }}
{MZ.CSS}
@media (max-width:900px) {{ .frame {{ grid-template-columns:1fr; }} aside {{ border-right:0; border-bottom:1px solid var(--rule); }} nav.side {{ display:flex; flex-wrap:wrap; }} .role {{ display:none; }} .crumb {{ white-space:normal; }} }}
</style>
<div class="frame">
<aside>
  <div class="brand"><i></i><div><b>情报收集员工作台</b><span>营事编集室 · 设计稿</span></div></div>
  <div class="navh">导航</div>
  <nav class="side">{navs}</nav>
  <div class="role">当前身份：主编（operator）<br>切换到 Van 视图只保留「Van 模式 · 选题记忆 · 知识库 · 指标」</div>
</aside>
<main>
  <div class="topbar"><span>环境 · 生产 <span class="pill">采集服务 Rust · 运行中</span><span class="pill">LanceDB 正常</span><span class="pill">引擎 已连接</span></span><span class="muted">设计稿：贴文、热度、标签、范例命中与 Van 决定为真实数据；六维度判定由早期打分实验换算，属示意；标「示意」的为示例</span></div>
  {sections}
</main>
</div>
<script>
document.querySelectorAll('nav.side a').forEach(a=>a.addEventListener('click',ev=>{{ev.preventDefault();document.querySelectorAll('nav.side a').forEach(x=>x.classList.toggle('on',x===a));document.querySelectorAll('.screen').forEach(s=>s.hidden=(s.id!=='s-'+a.dataset.s));window.scrollTo({{top:0}});}}));
</script>'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "screens", page.count('class="screen"'), "tiers", {t: sum(1 for e in EV if e["tier"] == t) for t in TIER_ORDER})
