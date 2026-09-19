import re, sys, html, markdown
src, out = sys.argv[1], sys.argv[2]
md = open(src, encoding="utf-8").read()
# 去掉文首 H1 与引言块（页面自带页眉）
md = re.sub(r"\A# [^\n]*\n\n(?:> [^\n]*\n)+\n", "", md, count=1)
body = markdown.markdown(md, extensions=["tables", "fenced_code", "toc", "sane_lists"],
                         extension_configs={"toc": {"slugify": lambda v, s: re.sub(r"[^\w一-鿿]+", "-", v).strip("-").lower()}})
body = re.sub(r'<pre><code class="language-mermaid">(.*?)</code></pre>', r'<div class="diagram"><pre class="mermaid">\1</pre></div>', body, flags=re.S)
body = re.sub(r"<table>", '<div class="table-wrap"><table>', body).replace("</table>", "</table></div>")
toc = "".join(f'<li><a href="#{m.group(1)}">{m.group(2)}</a></li>' for m in re.finditer(r'<h2 id="([^"]+)">(.*?)</h2>', body))
bars = [("Instagram · csw MCP", 49, 401), ("网页 · opencli（partial 轮）", 0, 70), ("小红书 · opencli", 1, 37)]
rows = "".join(
    f'<div class="cov-row"><div class="cov-label">{html.escape(n)}</div>'
    f'<div class="cov-bar" role="img" aria-label="看过 {a} 条，共 {b} 条"><span style="width:{max(a/b*100,0.8):.1f}%"></span></div>'
    f'<div class="cov-num"><b>{a}</b> / {b}</div></div>' for n, a, b in bars)
page = f'''<title>收集员 Codex 采集引擎</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&display=swap">
<style>
:root {{
  --ground:#F2F5F4; --surface:#FFFFFF; --ink:#16201E; --muted:#5A6A66; --rule:#D3DBD8;
  --accent:#0E6A5B; --accent-soft:#DDEEEA; --bad:#B23B2A; --code-bg:#E9EEEC;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif;
  --mono:"IBM Plex Mono","SF Mono",Menlo,Consolas,monospace;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
    --ground:#111615; --surface:#19201F; --ink:#E3EAE8; --muted:#94A29E; --rule:#2B3634;
    --accent:#56C6B1; --accent-soft:#173430; --bad:#F08A76; --code-bg:#1F2827;
  }}
}}
:root[data-theme="dark"] {{
  --ground:#111615; --surface:#19201F; --ink:#E3EAE8; --muted:#94A29E; --rule:#2B3634;
  --accent:#56C6B1; --accent-soft:#173430; --bad:#F08A76; --code-bg:#1F2827;
}}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:16px; line-height:1.8; padding-inline:20px; padding-block:40px 96px; }}
.page {{ max-width:820px; margin-inline:auto; display:flex; flex-direction:column; gap:28px; }}
header .eyebrow {{ font-family:var(--mono); font-size:12.5px; letter-spacing:.06em; color:var(--muted); display:flex; flex-wrap:wrap; gap:6px 14px; align-items:center; }}
.chip {{ border:1px solid var(--bad); color:var(--bad); padding:1px 8px; border-radius:3px; letter-spacing:.04em; }}
h1 {{ font-size:clamp(26px,5vw,36px); line-height:1.3; font-weight:700; margin:14px 0 10px; text-wrap:balance; }}
h1 span {{ display:block; font-size:.66em; font-weight:500; color:var(--accent); margin-top:4px; }}
.lede {{ font-size:18px; color:var(--muted); margin:0; max-width:38em; }}
.lede b {{ color:var(--ink); font-weight:500; }}
.cov {{ background:var(--surface); border:1px solid var(--rule); border-radius:6px; padding:20px 22px; display:flex; flex-direction:column; gap:14px; }}
.cov h2 {{ all:unset; font-weight:700; font-size:15px; display:block; }}
.cov-row {{ display:grid; grid-template-columns:minmax(0,15em) minmax(0,1fr) 5.5em; gap:14px; align-items:center; }}
.cov-label {{ font-size:14px; }}
.cov-bar {{ height:14px; border-radius:2px; background:repeating-linear-gradient(135deg,var(--rule) 0 4px,transparent 4px 8px); border:1px solid var(--rule); overflow:hidden; }}
.cov-bar span {{ display:block; height:100%; background:var(--accent); }}
.cov-num {{ font-family:var(--mono); font-size:13.5px; font-variant-numeric:tabular-nums; text-align:right; color:var(--muted); }}
.cov-num b {{ color:var(--ink); font-weight:500; }}
.cov figcaption, .cov .note {{ font-size:13px; color:var(--muted); line-height:1.6; }}
.legend {{ display:flex; gap:18px; flex-wrap:wrap; font-size:13px; color:var(--muted); }}
.legend i {{ display:inline-block; width:22px; height:10px; vertical-align:-1px; margin-right:6px; border:1px solid var(--rule); border-radius:2px; }}
.legend .a i {{ background:var(--accent); border-color:var(--accent); }}
.legend .b i {{ background:repeating-linear-gradient(135deg,var(--rule) 0 4px,transparent 4px 8px); }}
nav ol {{ margin:0; padding:0; list-style:none; display:flex; flex-wrap:wrap; gap:6px 8px; counter-reset:none; }}
nav a {{ display:block; font-size:13.5px; padding:3px 10px; border:1px solid var(--rule); border-radius:3px; color:var(--ink); text-decoration:none; background:var(--surface); }}
nav a:hover, nav a:focus-visible {{ border-color:var(--accent); color:var(--accent); outline:none; }}
article {{ min-width:0; }}
article h2 {{ font-size:22px; line-height:1.4; margin:56px 0 14px; padding-top:18px; border-top:2px solid var(--ink); text-wrap:balance; }}
article h3 {{ font-size:17px; margin:34px 0 10px; color:var(--accent); text-wrap:balance; }}
article p, article li {{ max-width:42em; }}
article p {{ margin:12px 0; }}
article ul, article ol {{ padding-left:1.4em; margin:12px 0; display:flex; flex-direction:column; gap:6px; }}
article li > ul, article li > ol {{ margin:6px 0 0; }}
article strong {{ font-weight:700; }}
article a {{ color:var(--accent); }}
code {{ font-family:var(--mono); font-size:.86em; background:var(--code-bg); padding:1px 5px; border-radius:3px; word-break:break-word; }}
pre {{ background:var(--code-bg); border:1px solid var(--rule); border-radius:6px; padding:14px 16px; overflow-x:auto; line-height:1.6; margin:16px 0; }}
pre code {{ background:none; padding:0; font-size:13px; word-break:normal; }}
.diagram {{ overflow-x:auto; background:var(--surface); border:1px solid var(--rule); border-radius:6px; padding:16px; margin:18px 0; }}
.diagram pre {{ background:none; border:0; padding:0; margin:0; min-width:680px; font-family:var(--mono); font-size:12px; }}
.table-wrap {{ overflow-x:auto; margin:18px 0; border:1px solid var(--rule); border-radius:6px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; font-size:14px; line-height:1.65; font-variant-numeric:tabular-nums; }}
th, td {{ text-align:left; vertical-align:top; padding:9px 12px; border-bottom:1px solid var(--rule); min-width:5.5em; }}
th {{ font-weight:700; font-size:13px; color:var(--muted); background:var(--ground); white-space:nowrap; }}
tr:last-child td {{ border-bottom:0; }}
hr {{ border:0; border-top:1px solid var(--rule); margin:40px 0; }}
footer {{ font-size:13px; color:var(--muted); border-top:1px solid var(--rule); padding-top:16px; }}
@media (max-width:560px) {{
  body {{ font-size:15.5px; padding-inline:16px; }}
  .cov-row {{ grid-template-columns:1fr 5.5em; }}
  .cov-label {{ grid-column:1 / -1; }}
}}
</style>
<div class="page">
<header>
  <div class="eyebrow"><span>营事编集室 · 资讯日更</span><span>开发版 · 第二版 · 2026-09-20</span><span class="chip">方向已认可 · 未动工</span></div>
  <h1>情报收集员改造<span>自定义客户端 + Codex App Server</span></h1>
  <p class="lede"><b>代码管采集，模型按 Van 的判断框架打分并写依据，引擎仍是唯一真相。</b>只替换情报收集员一个角色；Codex App Server 只承担入围条目的深核。</p>
</header>
<figure class="cov" style="margin:0">
  <h2>r48（9 月 18 日期）：接口给了多少，收集员实际看了多少</h2>
  {rows}
  <div class="legend"><span class="a"><i></i>看过并形成判断</span><span class="b"><i></i>加载了但没看</span></div>
  <figcaption>数据来自生产引擎 <code>intake_sweeps</code>，是收集员自己上报的数字。Instagram 一路 401 条里有 352 条没人看过。</figcaption>
</figure>
<nav aria-label="目录"><ol>{toc}</ol></nav>
<article>
{body}
</article>
<footer>原文在仓库 <code>docs/情报收集员_Codex采集引擎方案_20260918.md</code>，以仓库版本为准。</footer>
</div>
'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "h2", toc.count("<li>"))
