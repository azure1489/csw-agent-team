# -*- coding: utf-8 -*-
"""情报收集员工作台 · 界面设计稿（十个页面，静态原型）。用法：python build_mockup.py events_scored.json out.html"""
import json, sys, html
src, out = sys.argv[1], sys.argv[2]
EV = [e for e in json.load(open(src, encoding="utf-8")) if e.get("scores")]
DIMS = [("change", "具体变化"), ("use", "使用关联"), ("gain", "信息增量"), ("compare", "比较参照"), ("explain", "可解释性"), ("csw", "CSW 视角")]
W = {"change": .25, "use": .15, "gain": .20, "compare": .10, "explain": .15, "csw": .15}
def comp(d): return round(sum(W[k] * d[k] for k in W), 1)
for e in EV:
    e["composite"] = comp(e["scores"]["dims"])
EV.sort(key=lambda e: -e["composite"])
E = lambda s: html.escape(str(s))
KB_HIT = {"brompton": ("and wander × Brompton：骑小布，有了一套官方穿法 · vol.248", "2026-09-02", "正式已发布", "同品牌同产品：本条是 ROBIN 门店视角与骑行实拍，需判是否有新增信息"),
          "omnium": ("Carhartt WIP × OMNIUM：把工装骑上街 · vol.260", "2026-09-18", "正式已发布", "同一事件：本期已成稿（Van 指定链接），本条为媒体号视角")}
STATUS = {}
def status_of(e):
    s = e["scores"]
    if s["unanswered"] == "missing_material": return ("待核", "pending")
    if s["unanswered"] == "low_value": return ("已打分", "scored")
    return ("入围", "short")
def bars(d, small=True):
    h = "".join(f'<span class="bar" title="{n} {d[k]}"><i style="height:{d[k]*10}%"></i></span>' for k, n in DIMS)
    return f'<span class="bars">{h}</span>'
def tag(t, cls=""): return f'<span class="tag {cls}">{E(t)}</span>'
def thumbs(e, n=2, size=48):
    return "".join(f'<img src="{t}" width="{size}" height="{size}" alt="" loading="lazy">' for t in e["thumbs"][:n]) or f'<span class="noimg" style="width:{size}px;height:{size}px">无图</span>'

# ---------- 打分台账 ----------
rows = ""
for i, e in enumerate(EV, 1):
    s = e["scores"]; st, cls = status_of(e)
    kb = KB_HIT.get(e["name"])
    dedup = tag("同品牌 · 需判新增", "warn") if kb else tag("未发过", "ok")
    three = {"none": tag("三句话已答", "ok"), "missing_material": tag("缺资料", "warn"), "angle_not_formed": tag("角度未成立", "warn"), "low_value": tag("价值不足", "muted")}[s["unanswered"]]
    hits = "".join(tag(h, "pri") for h in s["priority_hits"][:2]) + "".join(tag(h, "low") for h in s["lower_hits"][:1])
    origin = tag("Van 链接", "van") if e["name"] == "gnuhr" else tag("系统", "muted")
    rows += f'''<tr class="row" data-i="{i}">
<td class="num">{i}</td>
<td class="th">{thumbs(e)}</td>
<td class="title"><b>{E(s["title"])}</b><span class="sub">@{E(e["account"])} · {E(str(e["date_posted"])[5:16].replace("T"," "))} · {e["n_images"]} 图 · 赞 {e["likes"]}</span></td>
<td class="score"><b>{e["composite"]}</b></td>
<td>{bars(s["dims"])}</td>
<td>{tag(s["kind"])}</td>
<td class="hits">{hits}</td>
<td>{dedup}</td>
<td>{three}</td>
<td class="num">{len(s["gaps"])}</td>
<td>{tag(st, cls)}</td>
<td>{origin}</td>
<td class="act"><button>首批</button><button>待核</button><button>改判</button></td>
</tr>'''
    if i == 1:
        rows += f'''<tr class="expand"><td colspan="13"><div class="expand-grid">
<div><h4>六维度依据</h4><ul>{"".join(f"<li><b>{n} {s['dims'][k]}</b> · {E(s['evidence'][k])}</li>" for k,n in DIMS)}</ul></div>
<div><h4>三句话</h4><ol><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
<h4>缺口</h4><p>{E("、".join(s["gaps"]) or "无")}</p><h4>外观与设计</h4><p>{E(s["look"])}</p></div>
<div><h4>知识库命中</h4><p class="muted">{E(KB_HIT.get(e["name"], ("无 90 天内相关报道","","",""))[0])}</p><h4>案例命中</h4><p class="muted">Van 9/16：「1非新品退回」— 本条为新品，不触发（案例，非规则）</p><h4>合并的贴文</h4><p class="muted">1 条 · @{E(e["account"])}</p></div>
</div></td></tr>'''
hard = '''<tr class="group"><td colspan="13">硬性排除 · 2 条（示意，可捞回）<button class="link">展开</button></td></tr>
<tr class="row dim"><td class="num">—</td><td class="th"><span class="noimg">图</span></td><td class="title"><b>示意｜窗口外的贴文</b><span class="sub">发布于窗口开始之前 · 代码判定</span></td><td class="score">—</td><td></td><td>{}</td><td></td><td>{}</td><td></td><td class="num">0</td><td>{}</td><td>{}</td><td class="act"><button>捞回</button></td></tr>
<tr class="row dim"><td class="num">—</td><td class="th"><span class="noimg">图</span></td><td class="title"><b>示意｜与正式已发布同一事实且无新增</b><span class="sub">对照 vol.255 · 2026-09-11 · 模型判定</span></td><td class="score">—</td><td></td><td>{}</td><td></td><td>{}</td><td></td><td class="num">0</td><td>{}</td><td>{}</td><td class="act"><button>捞回</button></td></tr>'''.format(tag("其他"), tag("窗口外","muted"), tag("硬性排除","dim"), tag("系统","muted"), tag("新品发布"), tag("重复 · 无新增","warn"), tag("硬性排除","dim"), tag("系统","muted"))

board = f'''
<div class="toolbar">
  <div class="filters">{tag("状态：全部")}{tag("类型：全部")}{tag("查重：全部")}{tag("三句话：全部")}{tag("来源：全部")}{tag("只看有图")}</div>
  <div class="right"><label>权重版本 <select><option>v1 · 变化 .25 使用 .15 增量 .20 参照 .10 解释 .15 CSW .15</option><option>v2 · 草案</option></select></label><label>排序 <select><option>综合分</option><option>具体变化</option><option>使用关联</option></select></label><button class="primary">提交首批（6 条）</button></div>
</div>
<div class="table-wrap"><table class="board">
<thead><tr><th>#</th><th>图</th><th>事件</th><th>综合</th><th>六维度</th><th>类型</th><th>命中</th><th>查重</th><th>三句话</th><th>缺口</th><th>状态</th><th>来源</th><th>操作</th></tr></thead>
<tbody>{rows}{hard}</tbody></table></div>
<p class="foot">共 422 个事件 · 已打分 422 · 看到实图 410 · 待核 9 · 硬性排除 31 · 本页显示实测过的 {len(EV)} 条与 2 条示意</p>'''

# ---------- Van 模式 ----------
van_cards = ""
for e in EV[:6]:
    s = e["scores"]
    van_cards += f'''<div class="vcard">{thumbs(e, 2, 120)}<div class="vbody"><b>{E(s["title"])}</b><span class="sub">@{E(e["account"])} · 综合 {e["composite"]}</span>
<ol class="three"><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
<div class="vact"><button class="ok">采用</button><button class="no">否决</button><button>暂缓</button><input placeholder="一句话原话（选填）"></div></div></div>'''

# ---------- 事件详情 ----------
d = EV[0]; s = d["scores"]; kb = KB_HIT.get(d["name"])
detail = f'''
<div class="detail">
<div class="col">
  <h3>图集 · {d["n_images"]} 张</h3><div class="gallery">{thumbs(d, 2, 160)}<span class="noimg" style="width:160px;height:160px">其余 {max(d["n_images"]-2,0)} 张懒加载</span></div>
  <h3>合并的贴文 · 1 条</h3><div class="post"><b>@{E(d["account"])}</b> <span class="sub">{E(str(d["date_posted"])[:16].replace("T"," "))} · 赞 {d["likes"]} · 粉丝 {d["followers"]}</span><p>{E(d["caption"][:400])}</p><a>{E(d["url"])}</a></div>
  <h3>披露时间</h3><p>原始披露：{E(str(d["date_posted"])[:10])}（品牌官方贴）· 转载：无</p>
</div>
<div class="col">
  <h3>六维度 · 综合 {d["composite"]}（权重 v1）</h3>
  <table class="dims">{"".join(f'<tr><th>{n}</th><td><span class="bar-h"><i style="width:{s["dims"][k]*10}%"></i></span> <b>{s["dims"][k]}</b></td><td class="ev">{E(s["evidence"][k])}</td></tr>' for k,n in DIMS)}</table>
  <h3>外观与设计</h3><p>{E(s["look"])}</p>
  <h3>三句话</h3><ol class="three"><li>{E(s["three_sentences"]["what_changed"])}</li><li>{E(s["three_sentences"]["why_it_matters"])}</li><li>{E(s["three_sentences"]["how_different"])}</li></ol>
  <h3>命中 · 缺口</h3><p>{"".join(tag(h,"pri") for h in s["priority_hits"])}{"".join(tag(h,"low") for h in s["lower_hits"])}</p><p>缺口：{E("、".join(s["gaps"]) or "无")}</p>
</div>
<div class="col">
  <h3>查重对照</h3>{f'<div class="ref"><b>{E(kb[0])}</b><span class="sub">{kb[1]} · {tag(kb[2],"ok")}</span><p>{E(kb[3])}</p></div>' if kb else '<p class="muted">正式已发布记录中 90 天内无同一事实</p>'}
  <h3>知识库命中</h3><div class="ref"><b>Kith × On K-Tech 2026 秋季系列（媒体号 le.syndrome）</b><span class="sub">同窗口 · 同一事件的另一条贴文，建议合并</span></div>
  <h3>案例命中</h3><div class="ref"><b>Van 9/16</b><span class="sub">「4国外新开店铺除非非常特别否则对于国内读者来说价值不足」</span><p class="muted">具体案例，非规则；本条不是门店信息，不触发</p></div>
  <h3>深核结果</h3><div class="ref"><b>待深核</b><span class="sub">入围后由 Codex App Server 核原始来源、比较参照与完整图集</span></div>
  <h3>审计</h3><table class="audit"><tr><td>打分</td><td>gpt-6-astra · 准则 v0 · 权重 v1</td><td>{d["secs"]} 秒</td><td>{(d.get("usage") or {}).get("inputTokens","—")} tok</td></tr><tr><td>合并</td><td>代码</td><td>—</td><td>—</td></tr></table>
  <div class="actions"><button class="primary">登记条目</button><button>标待核</button><button>人工改判</button></div>
</div></div>'''

# ---------- 其他页面 ----------
overview = '''
<div class="cards">
 <div class="card"><span class="k">贴文全量</span><b>477</b><span class="s">库内 477 · 一致</span></div>
 <div class="card"><span class="k">合并后事件</span><b>422</b><span class="s">重复贴文 55 条已并入</span></div>
 <div class="card"><span class="k">已打分</span><b>100%</b><span class="s">422 / 422 · 由代码计数</span></div>
 <div class="card"><span class="k">看到实图</span><b>86%</b><span class="s">410 / 477 · 67 条纯视频取封面</span></div>
</div>
<div class="panel"><h3>进度 · 期次 2026-09-18 · 窗口 09-16 07:00 → 09-18 07:00 · 距首批时限 <b>12 分钟</b></h3>
<div class="steps"><span class="done">A 取全量 2.1 秒</span><span class="done">B 合并 8 秒</span><span class="done">C 查重 1 分 40 秒</span><span class="done">D 打分 18 分钟</span><span class="doing">E 深核 6 / 24</span><span>F 首批提交</span></div></div>
<div class="grid2">
 <div class="panel"><h3>告警</h3><ul class="alerts"><li class="warn">小红书来源：opencli 超时 2 次，已重试成功</li><li class="warn">读不到图：3 个事件（视频封面下载失败），已标待核</li><li>模型调用失败 1 次（429），重试成功</li></ul></div>
 <div class="panel"><h3>值得主编看一眼</h3><ul class="alerts"><li>硬性排除里综合分 ≥ 7 的：1 条（与 vol.255 同一事实，模型判无新增）</li><li>Van 链接：1 条已进入优先深核</li><li>待补录账号：4 个（来自本期候选与 Van 链接）</li></ul></div>
</div>'''

harvest = '''
<div class="panel"><h3>采集轮 · 本期</h3><div class="table-wrap"><table>
<thead><tr><th>来源</th><th>工具</th><th>时间窗</th><th>接口返回</th><th>去重后</th><th>窗口内</th><th>已打分</th><th>看到图</th><th>结果</th></tr></thead>
<tbody>
<tr><td>Instagram · 贴文库</td><td>csw_posts_window</td><td>09-16 07:00 → 09-18 07:00</td><td>477</td><td>477</td><td>477</td><td><b>477</b></td><td>410</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>小红书 · 关键词 12 个</td><td>opencli</td><td>同上</td><td>37</td><td>37</td><td>37</td><td><b>37</b></td><td>37</td><td><span class="tag warn">partial · 超时 2 次后成功</span></td></tr>
<tr><td>GO OUT</td><td>opencli</td><td>同上</td><td>14</td><td>14</td><td>9</td><td><b>9</b></td><td>9</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>CAMP HACK</td><td>opencli</td><td>同上</td><td>11</td><td>11</td><td>6</td><td><b>6</b></td><td>6</td><td><span class="tag ok">ok</span></td></tr>
<tr><td>Van 分享链接</td><td>直通</td><td>—</td><td>1</td><td>1</td><td>1</td><td><b>1</b></td><td>1</td><td><span class="tag van">Van 链接 · 单独统计</span></td></tr>
</tbody></table></div></div>
<div class="grid2">
<div class="panel"><h3>来源台账</h3><table class="plain"><tr><td>Instagram 贴文库</td><td>必扫</td><td class="ok">05:00 已抓取 · 228 条入库</td></tr><tr><td>小红书</td><td>必扫</td><td class="ok">09-20 05:41</td></tr><tr><td>品牌官网 / 媒体</td><td>普通</td><td class="ok">09-20 05:44</td></tr><tr><td>GO OUT · CAMP HACK</td><td>普通（Van 补充，不加权）</td><td class="ok">09-20 05:44</td></tr></table></div>
<div class="panel"><h3>待补录账号 · 4</h3><table class="plain"><tr><td>@moss_tents</td><td>Van 9/18 指定链接 · MOSS THE EDEN</td><td><button>导出</button></td></tr><tr><td>@carharttwip</td><td>Van 9/18 指定 · 刊发库 vol.260</td><td></td></tr><tr><td>@wearebraindead</td><td>Van 9/18 指定 · Brain Dead × FIVE TEN</td><td></td></tr><tr><td>@heimplanet（全球号）</td><td>在册的是 heimplanet_japan · Van 9/18 指定</td><td></td></tr></table><p class="muted">补录在 csw 后台完成；清单每周汇总一次。</p></div>
</div>'''

pending = '''
<div class="panel"><h3>待核结转 · 9</h3><div class="table-wrap"><table>
<thead><tr><th>事件</th><th>缺口</th><th>首次出现</th><th>剩余期数</th><th>上次重评</th><th>操作</th></tr></thead>
<tbody>
<tr><td><b>Helinox｜复写七八十年代露营风格</b><span class="sub">@helinox_kr</span></td><td>缺发售信息与具体产品清单；文案只有背景</td><td>r48</td><td>5</td><td>—</td><td><button>补证后重评</button><button>关闭</button></td></tr>
<tr><td><b>FUTURE FOX｜炉顶附件 10 月下旬新型</b><span class="sub">@futurefox</span></td><td>缺官方页与价格</td><td>r47</td><td>3</td><td>r48：仍缺价格</td><td><button>补证后重评</button><button>关闭</button></td></tr>
<tr><td><b>New Balance｜Flying 77 与石</b><span class="sub">@newbalance</span></td><td>图不足（只有 1 张海报）</td><td>r47</td><td>3</td><td>r48：无新图</td><td><button>补证后重评</button><button>关闭</button></td></tr>
<tr><td><b>示意｜读不到图的事件</b><span class="sub">视频封面下载失败</span></td><td>未读到实图，不算看过</td><td>r48</td><td>7</td><td>—</td><td><button>重试下载</button></td></tr>
</tbody></table></div><p class="muted">当期截止前未补齐不阻塞已批准选题；到期自动关闭并写明原因，不自动入选。</p></div>'''

kb = '''
<div class="toolbar"><input class="search" value="Brompton" placeholder="检索品牌、产品、别名或关键词"><span class="tag">品牌别名：小布 · ブロンプトン</span></div>
<div class="grid2">
<div class="panel"><h3>检索结果 · 2</h3>
<div class="ref"><b>and wander × Brompton：骑小布，有了一套官方穿法 · vol.248</b><span class="sub">2026-09-02 · <span class="tag ok">正式已发布</span> · 拆条 6 · 本条：and wander × Brompton 联名骑行服与折叠车</span></div>
<div class="ref"><b>Carhartt WIP × OMNIUM：把工装骑上街 · vol.260</b><span class="sub">2026-09-18 · <span class="tag ok">正式已发布</span> · 货运自行车联名</span></div>
<h3>品牌页 · Brompton</h3><p class="muted">近 30 天：1 篇 · 近 90 天：1 篇 · 角度：联名骑行装、通勤与户外切换</p></div>
<div class="panel"><h3>同步状态</h3><table class="plain"><tr><td>最近同步</td><td>09-20 05:00 · ok</td></tr><tr><td>覆盖到</td><td>2026-09-19</td></tr><tr><td>正式已发布</td><td>261 篇（回填至 vol.1）</td></tr><tr><td>参考范文</td><td>35 篇 · 其中 10 篇为 Van 指定正面范例</td></tr><tr><td>未发布稿件</td><td>2 篇（草稿箱）· 不参与查重</td></tr><tr><td>缺口</td><td>群发类 3 篇公开页抓取失败 · 已列清单</td></tr></table>
<h3>十篇正面范例</h3><table class="plain"><tr><td>09-18</td><td>Carhartt WIP × OMNIUM：把工装骑上街 vol.260</td><td><span class="tag ok">拆解完成</span></td></tr><tr><td>09-16</td><td>BRUNT：用装备，唱情歌 专题 Vol.12</td><td><span class="tag ok">拆解完成</span></td></tr><tr><td>09-11</td><td>Apple Ultra 4：够不够让户外人换表？vol.255</td><td><span class="tag warn">待核推断 2 处</span></td></tr><tr><td>09-03</td><td>1983年，数字游民OG把办公室装上单车（贴图文）</td><td><span class="tag ok">拆解完成</span></td></tr><tr><td colspan="3" class="muted">…共 10 篇；前六篇「数据表现不错」为 Van 提供的评价，未独立核验</td></tr></table></div>
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
<tr><td>被打回的同一事实同一角度不再重复提报</td><td><span class="tag ok">本人确认</span></td><td>「其他全部打回并不要在之后的选题里重复出现」· 9/16</td><td>被否条目</td><td>—</td></tr>
<tr><td>图片不足的先待核，核实够用再上</td><td><span class="tag">具体案例</span></td><td>「5可保留但需要验证图片是否足够」· 9/16</td><td>当时第 5 条 Springbar</td><td><button>对</button><button>不对</button><button>改</button></td></tr>
<tr><td>联名若只有 logo 叠加、无使用信息，优先级低</td><td><span class="tag warn">模型推断</span></td><td>从 9/16、9/18 两次否决归纳</td><td>联名类 · 待校准</td><td><button>对</button><button>不对</button><button>改</button></td></tr>
</tbody></table></div></div>
<div class="grid2">
<div class="panel"><h3>案例库 · 最近</h3><div class="ref"><b>9/18 · 退回两条联名，改用自选 8 条</b><span class="sub">White Mountaineering × MOONSTAR、SOUTH2 WEST8 × BEN MILLER → 否决（未给理由）</span></div><div class="ref"><b>9/16 · 七条退六条</b><span class="sub">1 非新品；2、3、6、7 产品价值不足；4 国外新店；5 保留待核图片</span></div><div class="ref"><b>9/15 · 三条全退</b><span class="sub">「三个产品的报道价值都不高；选题数目太少」</span></div></div>
<div class="panel"><h3>回收覆盖清单</h3><table class="plain"><tr><td>主编会话库</td><td>私聊 62 · 群 247</td><td class="ok">已回收</td></tr><tr><td>文案会话库</td><td>私聊 91 · 群 29</td><td class="ok">已回收（写作表达线）</td></tr><tr><td>其余七个会话库</td><td>私聊 0–6 · 群 2–49</td><td class="ok">已回收</td></tr><tr><td>飞书群完整记录</td><td>—</td><td class="warn">待权限</td></tr><tr><td>身份映射</td><td>九个应用下的 open_id</td><td class="warn">待做</td></tr></table></div>
</div>'''

rubric = '''
<div class="grid2">
<div class="panel"><h3>六个维度与锚点</h3><table class="plain rub">
<tr><th>具体变化或报道切口</th><td>10：Kith × On「性能科技转向日常」有具体系列与设计说明 · 6：gnuhr 面料送检，切口是材料 · 2：Mt.SUMI 补货，无变化</td></tr>
<tr><th>使用关联</th><td>9：Brompton × and wander 骑行与山野的切换 · 3：只有配色</td></tr>
<tr><th>信息增量</th><td>8：读者能理解一种新做法 · 2：重复已知信息</td></tr>
<tr><th>比较参照</th><td>9：明确对比上一代或同类 · 1：无参照（不编造）</td></tr>
<tr><th>可解释性</th><td>9：能形成有事实支持的编辑判断 · 4：只能说好看</td></tr>
<tr><th>CSW 视角</th><td>9：有用有趣有料兼具 · 2：与户外潮流无关</td></tr>
</table><p class="muted">每级锚点引用范例与被否案例；新增或修改锚点写审计。品牌知名度不是维度。</p></div>
<div class="panel"><h3>权重版本</h3><table class="plain"><tr><th>版本</th><th>变化 / 使用 / 增量 / 参照 / 解释 / CSW</th><th>生效</th><th>谁 · 为什么</th></tr><tr><td>v1</td><td>.25 / .15 / .20 / .10 / .15 / .15</td><td>r49 起</td><td>开发 · 初始，按 Van 框架的措辞轻重</td></tr><tr><td>v2 草案</td><td>.20 / .20 / .20 / .10 / .15 / .15</td><td>—</td><td>主编 · 想提高使用关联</td></tr></table>
<h3>回测 · Van 有态度的条目在当前权重下的排名</h3><table class="plain"><tr><td>gnuhr Heatwave（亲选）</td><td>综合 6.5 · 第 7 名 / 12</td></tr><tr><td>S2W8 × BEN MILLER（否决）</td><td>综合 7.6 · 第 4 名 / 12</td></tr><tr><td>White Mountaineering × MOONSTAR（否决）</td><td>综合 6.4 · 第 9 名 / 12</td></tr></table><p class="muted">样本只有三条，结论只用于提示维度或锚点可能还没对上她的取舍；重复打分一致性：待 M3 检验。</p></div>
</div>'''

metrics = '''
<div class="cards">
 <div class="card"><span class="k">自主发现情况</span><b>0 / 7</b><span class="s">r48 基线：采用条目里系统先于 Van 发现的</span></div>
 <div class="card"><span class="k">第一轮推荐质量</span><b>2 → 0</b><span class="s">r48：首轮推荐 2 条，采用 0，随后她给 8 条链接</span></div>
 <div class="card"><span class="k">Van 的实际投入</span><b>约 45 分钟</b><span class="s">r48：审题 + 补链接 + 说明（估算）</span></div>
 <div class="card"><span class="k">审阅覆盖</span><b>12% → 100%</b><span class="s">r48 → 并行期目标</span></div>
</div>
<div class="panel"><h3>按期次</h3><div class="chart"><div class="axis"></div><svg viewBox="0 0 720 200" role="img" aria-label="自主发现比例按期次折线，r48 为 0，之后为目标示意" style="max-width:100%;height:auto"><polyline points="40,170 200,170 360,120 520,90 680,60" fill="none" stroke="#4F46E5" stroke-width="2" stroke-dasharray="0 0 100 0"/><g font-size="12" fill="currentColor" opacity=".7"><text x="40" y="192">r48</text><text x="200" y="192">r49</text><text x="360" y="192">r50</text><text x="520" y="192">r51</text><text x="680" y="192">r52</text><text x="8" y="24">100%</text><text x="8" y="174">0%</text></g><line x1="40" y1="20" x2="40" y2="176" stroke="currentColor" opacity=".3"/><line x1="40" y1="176" x2="700" y2="176" stroke="currentColor" opacity=".3"/></svg><p class="muted">r48 为真实基线；r49 起为目标走势示意。数值目标按实测再定。</p></div></div>'''

ops = '''
<div class="cards">
 <div class="card"><span class="k">采集服务</span><b class="ok">运行中</b><span class="s">v0.1.0 · 上次期次 05:31 开工</span></div>
 <div class="card"><span class="k">Codex App Server</span><b class="ok">就绪</b><span class="s">固定版本 · 独立 CODEX_HOME</span></div>
 <div class="card"><span class="k">csw MCP · 本地 MCP</span><b class="ok">已连接</b><span class="s">只读白名单 9 个工具</span></div>
 <div class="card"><span class="k">本期模型费用</span><b>约 1.2 美元</b><span class="s">打分 422 次 · 深核 24 次（示意）</span></div>
</div>
<div class="grid2">
<div class="panel"><h3>模型调用 · 本期</h3><table class="plain"><tr><th>阶段</th><th>次数</th><th>中位耗时</th><th>token</th><th>失败 / 重试</th></tr><tr><td>查重判断</td><td>96</td><td>1.5 秒</td><td>0.6k</td><td>0 / 0</td></tr><tr><td>打分（文字 + 图）</td><td>422</td><td>18 秒</td><td>6.8k</td><td>1 / 1</td></tr><tr><td>深核</td><td>24</td><td>2 分 10 秒</td><td>31k</td><td>0 / 0</td></tr></table><p class="muted">实测参考：本机 8 条样本每回合 11–22 秒；生产用独立 CODEX_HOME 后底座上下文会明显低于本机的 2 万 token。</p></div>
<div class="panel"><h3>设置</h3><table class="plain"><tr><td>采集窗口</td><td>前一天到当天（UTC）· 周一覆盖到上周五</td></tr><tr><td>并行线程</td><td>3</td></tr><tr><td>每事件缩略图</td><td>3 张 · w_768</td></tr><tr><td>待核结转期限</td><td>7 期</td></tr><tr><td>权重版本</td><td>v1（改动写审计）</td></tr><tr><td>回放</td><td><button>选历史窗口重跑</button></td></tr></table></div>
</div>'''

NAV = [("overview", "今日总览"), ("board", "打分台账"), ("van", "Van 模式"), ("detail", "事件详情"), ("harvest", "采集与覆盖"), ("pending", "待核结转"), ("kb", "刊发知识库"), ("memory", "选题记忆"), ("rubric", "判断框架与权重"), ("metrics", "指标"), ("ops", "运行与设置")]
NOTES = {
 "overview": "主编 05:30 后第一眼看的页。四个数字都由代码计数；进度条按六步显示各自耗时；告警只列需要人处理的。",
 "board": "核心页。默认按综合分排序，硬性排除折叠在底部可捞回；改权重版本即时重排，不重跑模型；展开行看依据、三句话、缺口、命中与合并的贴文。",
 "van": "只给 Van 看的精简视图：图、标题、三句话、综合分与三个按钮。她的标记即时进入记忆库并在主编视图显示；正式决定仍经引擎 03 闸。",
 "detail": "三栏：左原料（图集、贴文、披露时间），中判断（六维度依据、三句话、缺口），右对照与审计（查重、知识库、案例、深核、模型调用）。",
 "harvest": "回答「找了什么、找到几条、看过几条」。已打分列应恒为 100%；Van 补充的来源标为普通来源；待补录账号带依据。",
 "pending": "证据或图片不足的不永久淘汰：写明缺口、结转期数、上次重评结果；到期关闭要写原因。",
 "kb": "检索只命中正式已发布的做查重；参考范文与未发布稿件分开标记；同步状态与缺口清单常驻。",
 "memory": "每条准则标明身份（具体案例 / 模型推断 / 本人确认）与原话；Van 校准模式逐条对不对；回收覆盖清单如实列缺口。",
 "rubric": "维度定义直接取自 Van 的框架；每级有锚点；权重带版本与理由；用她的历史决定回测。",
 "metrics": "先看 Van 的三项目标指标，再看技术指标；她提供链接后的处理结果单独统计。",
 "ops": "开发用：服务与依赖状态、模型调用与费用、失败重试、设置与回放。",
}
SCREENS = {"overview": overview, "board": board, "van": f'<div class="vgrid">{van_cards}</div>', "detail": detail, "harvest": harvest, "pending": pending, "kb": kb, "memory": memory, "rubric": rubric, "metrics": metrics, "ops": ops}
TITLES = dict(NAV)
sections = "".join(f'<section class="screen" id="s-{k}" {"" if k=="overview" else "hidden"}><div class="shead"><div><h2>{TITLES[k]}</h2><p class="note">{E(NOTES[k])}</p></div><span class="crumb">期次 r48 · 2026-09-18 · 窗口 09-16 07:00 → 09-18 07:00</span></div>{SCREENS[k]}</section>' for k in SCREENS)
navs = "".join(f'<a href="#" data-s="{k}" class="{"on" if k=="overview" else ""}">{n}</a>' for k, n in NAV)

page = f'''<title>收集员工作台界面设计稿</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Noto+Sans+SC:wght@400;500;700&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
:root {{ --ground:#F5F6F8; --surface:#FFFFFF; --ink:#1B1F2A; --muted:#6B7280; --rule:#E5E7EB; --accent:#4F46E5; --accent-soft:#EEF0FD; --ok:#1F8A5B; --ok-soft:#E4F5EC; --warn:#B45309; --warn-soft:#FDF1DC; --van:#8A3FA8; --van-soft:#F3E8F9; --dim:#9CA3AF;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif; --mono:"IBM Plex Mono",Menlo,monospace; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5CCB93; --ok-soft:#173327; --warn:#F2B96B; --warn-soft:#3A2A12; --van:#C48BE0; --van-soft:#33203D; --dim:#6B7280; }} }}
:root[data-theme="dark"] {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --ok:#5CCB93; --ok-soft:#173327; --warn:#F2B96B; --warn-soft:#3A2A12; --van:#C48BE0; --van-soft:#33203D; --dim:#6B7280; }}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:13.5px; line-height:1.6; padding-inline:16px; padding-block:16px 60px; }}
.frame {{ max-width:1440px; margin-inline:auto; background:var(--surface); border:1px solid var(--rule); border-radius:10px; overflow:hidden; display:grid; grid-template-columns:212px minmax(0,1fr); min-height:900px; }}
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
.topbar {{ display:flex; justify-content:space-between; align-items:center; padding:10px 22px; border-bottom:1px solid var(--rule); font-size:13px; }}
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
.panel h3 {{ margin:0 0 10px; font-size:14px; }} .panel h3:not(:first-child) {{ margin-top:16px; }}
.grid2 {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(360px,1fr)); gap:12px; }}
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
.tag.low {{ color:var(--dim); }} .tag.short {{ background:var(--accent-soft); color:var(--accent); border-color:transparent; }} .tag.pending {{ background:var(--warn-soft); color:var(--warn); border-color:transparent; }} .tag.dim, .tag.muted {{ color:var(--dim); }}
.table-wrap {{ overflow-x:auto; border:1px solid var(--rule); border-radius:8px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; }} th, td {{ text-align:left; vertical-align:top; padding:8px 10px; border-bottom:1px solid var(--rule); }}
th {{ font-size:12px; color:var(--muted); background:var(--ground); white-space:nowrap; font-weight:600; }} tr:last-child td {{ border-bottom:0; }}
table.plain th, table.plain td {{ padding:7px 8px; font-size:13px; }} table.plain td.ok {{ color:var(--ok); }} table.plain td.warn {{ color:var(--warn); }}
table.board td {{ font-size:12.5px; }} td.num {{ font-family:var(--mono); color:var(--muted); text-align:right; }} td.score b {{ font-size:16px; font-family:var(--mono); }}
td.th {{ white-space:nowrap; }} td.th img, .noimg {{ border-radius:4px; object-fit:cover; margin-right:3px; vertical-align:middle; background:var(--ground); }}
.noimg {{ display:inline-flex; align-items:center; justify-content:center; font-size:11px; color:var(--dim); border:1px dashed var(--rule); }}
td.title b {{ display:block; font-size:13px; }} .sub {{ display:block; font-size:11.5px; color:var(--muted); }}
.bars {{ display:inline-flex; gap:2px; align-items:flex-end; height:26px; }} .bar {{ width:9px; height:26px; background:var(--ground); border-radius:2px; display:inline-flex; align-items:flex-end; }} .bar i {{ display:block; width:100%; background:var(--accent); border-radius:2px; }}
td.hits {{ max-width:180px; }} td.act {{ white-space:nowrap; }} td.act button {{ margin-right:4px; }}
tr.expand td {{ background:var(--ground); }} .expand-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(280px,1fr)); gap:14px; padding:6px 4px; }}
.expand-grid h4 {{ margin:0 0 6px; font-size:12.5px; color:var(--accent); }} .expand-grid ul, .expand-grid ol {{ margin:0; padding-left:1.2em; font-size:12.5px; }} .expand-grid p {{ margin:4px 0; font-size:12.5px; }}
tr.group td {{ background:var(--ground); color:var(--muted); font-size:12.5px; }} tr.dim td {{ opacity:.65; }}
.foot {{ color:var(--muted); font-size:12.5px; margin:0; }} .muted {{ color:var(--muted); }}
.vgrid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(420px,1fr)); gap:12px; }}
.vcard {{ border:1px solid var(--rule); border-radius:8px; padding:12px; display:flex; gap:12px; background:var(--surface); }} .vcard img {{ border-radius:6px; object-fit:cover; }} .vbody {{ min-width:0; flex:1; }} .vbody b {{ font-size:14px; }}
.three {{ margin:8px 0; padding-left:1.2em; font-size:12.5px; display:flex; flex-direction:column; gap:3px; }}
.vact {{ display:flex; gap:6px; flex-wrap:wrap; align-items:center; }} .vact input {{ flex:1; min-width:160px; }}
.detail {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(320px,1fr)); gap:14px; }} .col {{ border:1px solid var(--rule); border-radius:8px; padding:14px; background:var(--surface); min-width:0; }} .col h3 {{ margin:14px 0 8px; font-size:13.5px; }} .col h3:first-child {{ margin-top:0; }}
.gallery {{ display:flex; gap:6px; flex-wrap:wrap; }} .gallery img {{ border-radius:6px; object-fit:cover; }}
.post p {{ font-size:12.5px; white-space:pre-line; margin:6px 0; }} .post a {{ font-size:11.5px; color:var(--accent); word-break:break-all; }}
table.dims th {{ background:none; width:6em; font-weight:600; color:var(--ink); }} table.dims td.ev {{ font-size:12px; color:var(--muted); }}
.bar-h {{ display:inline-block; width:80px; height:8px; background:var(--ground); border-radius:4px; vertical-align:middle; }} .bar-h i {{ display:block; height:100%; background:var(--accent); border-radius:4px; }}
.ref {{ border:1px solid var(--rule); border-radius:6px; padding:8px 10px; margin-bottom:8px; }} .ref b {{ font-size:12.5px; }} .ref p {{ margin:4px 0 0; font-size:12px; }}
table.audit td {{ font-size:12px; }} .actions {{ display:flex; gap:6px; margin-top:12px; }}
table.rub th {{ width:9em; white-space:normal; }}
.chart svg {{ display:block; }}
@media (max-width:900px) {{ .frame {{ grid-template-columns:1fr; }} aside {{ border-right:0; border-bottom:1px solid var(--rule); }} nav.side {{ display:flex; flex-wrap:wrap; }} .role {{ display:none; }} .crumb {{ white-space:normal; }} }}
</style>
<div class="frame">
<aside>
  <div class="brand"><i></i><div><b>情报收集员工作台</b><span>营事编集室 · 设计稿</span></div></div>
  <div class="navh">导航</div>
  <nav class="side">{navs}</nav>
  <div class="role">当前身份：主编（operator）<br>切换到 Van 视图只保留「Van 模式 · 选题记忆 · 刊发知识库 · 指标」</div>
</aside>
<main>
  <div class="topbar"><span>环境 · 生产 <span class="pill">采集服务 运行中</span><span class="pill">引擎 已连接</span></span><span class="muted">设计稿：贴文、打分与依据为 9/16–9/18 窗口的真实数据；标「示意」的为示例</span></div>
  {sections}
</main>
</div>
<script>
document.querySelectorAll('nav.side a').forEach(a=>a.addEventListener('click',ev=>{{ev.preventDefault();document.querySelectorAll('nav.side a').forEach(x=>x.classList.toggle('on',x===a));document.querySelectorAll('.screen').forEach(s=>s.hidden=(s.id!=='s-'+a.dataset.s));window.scrollTo({{top:0}});}}));
</script>'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "screens", page.count('class="screen"'))
