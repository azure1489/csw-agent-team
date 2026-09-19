import re, sys, html, markdown
src, out = sys.argv[1], sys.argv[2]
md = open(src, encoding="utf-8").read()
md = re.sub(r"\A# .*?\n\n.*?\n\n(\*\*一句话\*\*.*?\n)\n---\n", "", md, count=1, flags=re.S)
body = markdown.markdown(md, extensions=["tables", "fenced_code", "sane_lists", "toc"],
        extension_configs={"toc": {"slugify": lambda v, s: re.sub(r"[^\w一-鿿]+", "-", v).strip("-").lower()}})
body = re.sub(r'<pre><code class="language-mermaid">(.*?)</code></pre>', r'<div class="diagram"><pre class="mermaid">\1</pre></div>', body, flags=re.S)
body = body.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
cells = "".join('<i class="on"></i>' if i < 49 else "<i></i>" for i in range(401))
fig_cov = f'''<figure class="fig">
<div class="grid401" role="img" aria-label="401 个小格代表 401 条贴文，其中 49 个实心，表示审阅过的 49 条">{cells}</div>
<figcaption><span class="key"><i class="on"></i>审阅过的 49 条</span><span class="key"><i></i>未审阅的 352 条</span><span class="src">每一格是一条贴文。r48（9 月 18 日期）csw MCP 一路，数字为收集员自报。</span></figcaption>
</figure>'''
picks = [("gnuhr", True), ("MOSS", False), ("Carhartt WIP", False), ("Daniel Arsham", False),
         ("Brain Dead", False), ("HEIMPLANET", False), ("SATISFY", False), ("另一条", False)]
tiles = "".join(f'<div class="pick{" on" if ok else ""}"><b>{html.escape(n)}</b><span>{"在库" if ok else "不在库"}</span></div>' for n, ok in picks)
fig_picks = f'''<figure class="fig">
<div class="picks" role="img" aria-label="Van 指定的 8 条里只有 gnuhr 一条在贴文库里">{tiles}</div>
<figcaption><span class="src">Van 9 月 18 日指定的 8 条，按短码逐条去 csw 后端贴文库核对的结果。</span></figcaption>
</figure>'''
body = body.replace("<!-- FIG:coverage -->", fig_cov).replace("<!-- FIG:picks -->", fig_picks)
body = re.sub(r'(<h2 id="[^"]*需要各方配合的事[^"]*">.*?)(?=<h2 )', r'<section class="todo">\1</section>', body, count=1, flags=re.S)
toc = "".join(f'<li><a href="#{m.group(1)}">{m.group(2)}</a></li>' for m in re.finditer(r'<h2 id="([^"]+)">(.*?)</h2>', body))
page = f'''<title>收集员改造方案概要版</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&family=Noto+Serif+SC:wght@600;700&display=swap">
<style>
:root {{
  --ground:#F3F5F3; --surface:#FFFFFF; --ink:#18211F; --muted:#5B6A66; --rule:#D4DBD8;
  --accent:#0E6A5B; --accent-soft:#E0EFEB; --warn:#B23B2A; --empty:#DDE3E0; --code-bg:#E9EEEC;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif;
  --serif:"Noto Serif SC","Songti SC","STSong","SimSun",serif;
  --mono:"IBM Plex Mono","SF Mono",Menlo,Consolas,monospace;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
    --ground:#111615; --surface:#19201F; --ink:#E4EAE8; --muted:#95A39F; --rule:#2C3735;
    --accent:#5AC8B3; --accent-soft:#16332F; --warn:#F08A76; --empty:#2A3432; --code-bg:#1F2827;
  }}
}}
:root[data-theme="dark"] {{
  --ground:#111615; --surface:#19201F; --ink:#E4EAE8; --muted:#95A39F; --rule:#2C3735;
  --accent:#5AC8B3; --accent-soft:#16332F; --warn:#F08A76; --empty:#2A3432; --code-bg:#1F2827;
}}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:16px; line-height:1.85; padding-inline:20px; padding-block:44px 96px; }}
.page {{ max-width:780px; margin-inline:auto; display:flex; flex-direction:column; gap:30px; }}
.eyebrow {{ font-size:13px; color:var(--muted); letter-spacing:.08em; display:flex; flex-wrap:wrap; gap:4px 14px; align-items:center; }}
.chip {{ border:1px solid var(--warn); color:var(--warn); padding:0 8px; border-radius:3px; letter-spacing:.04em; }}
h1 {{ font-family:var(--serif); font-weight:700; font-size:clamp(28px,6vw,40px); line-height:1.3; margin:14px 0 16px; text-wrap:balance; }}
h1 span {{ display:block; font-family:var(--sans); font-size:.5em; font-weight:500; color:var(--accent); margin-top:6px; letter-spacing:.01em; }}
.goal {{ margin:0 0 18px; padding:16px 20px; background:var(--accent-soft); border-radius:6px; font-family:var(--serif); font-size:18px; line-height:1.85; }}
.goal span {{ display:block; font-family:var(--sans); font-size:12.5px; letter-spacing:.08em; color:var(--accent); margin-bottom:6px; }}
.lede {{ font-size:17.5px; line-height:1.9; margin:0; }}
.lede b {{ font-weight:700; }}
nav ol {{ list-style:none; margin:0; padding:0; display:flex; flex-wrap:wrap; gap:6px 8px; }}
nav a {{ display:block; font-size:13.5px; padding:3px 11px; border:1px solid var(--rule); border-radius:3px; background:var(--surface); color:var(--ink); text-decoration:none; }}
nav a:hover, nav a:focus-visible {{ border-color:var(--accent); color:var(--accent); outline:none; }}
article {{ min-width:0; }}
article h2 {{ font-family:var(--serif); font-weight:700; font-size:24px; line-height:1.4; margin:60px 0 16px; padding-top:20px; border-top:2px solid var(--ink); text-wrap:balance; }}
article h3 {{ font-size:18px; font-weight:700; margin:36px 0 8px; color:var(--accent); text-wrap:balance; }}
article p {{ margin:13px 0; }}
article ul, article ol {{ padding-left:1.4em; margin:13px 0; display:flex; flex-direction:column; gap:7px; }}
article li > ul {{ margin:7px 0 0; }}
article strong {{ font-weight:700; }}
code {{ font-family:var(--mono); font-size:.86em; background:var(--code-bg); padding:1px 5px; border-radius:3px; word-break:break-word; }}
.diagram {{ overflow-x:auto; background:var(--surface); border:1px solid var(--rule); border-radius:6px; padding:16px; margin:20px 0; }}
.diagram pre {{ margin:0; min-width:640px; font-family:var(--mono); font-size:12px; line-height:1.6; }}
.table-wrap {{ overflow-x:auto; margin:20px 0; border:1px solid var(--rule); border-radius:6px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; font-size:14.5px; line-height:1.7; font-variant-numeric:tabular-nums; }}
th, td {{ text-align:left; vertical-align:top; padding:10px 13px; border-bottom:1px solid var(--rule); min-width:5em; }}
th {{ font-size:13px; color:var(--muted); background:var(--ground); font-weight:700; white-space:nowrap; }}
tr:last-child td {{ border-bottom:0; }}
td:first-child {{ min-width:6em; }}
.fig {{ margin:18px 0 22px; background:var(--surface); border:1px solid var(--rule); border-radius:6px; padding:20px; display:flex; flex-direction:column; gap:14px; }}
.grid401 {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(13px,1fr)); gap:3px; }}
.grid401 i {{ aspect-ratio:1; background:var(--empty); border-radius:1.5px; }}
.grid401 i.on {{ background:var(--accent); }}
figcaption {{ display:flex; flex-wrap:wrap; gap:6px 18px; font-size:13.5px; color:var(--muted); line-height:1.6; }}
.key i {{ display:inline-block; width:11px; height:11px; border-radius:1.5px; background:var(--empty); margin-right:6px; vertical-align:-1px; }}
.key i.on {{ background:var(--accent); }}
.src {{ flex-basis:100%; }}
.picks {{ display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:8px; }}
.pick {{ border:1px dashed var(--rule); border-radius:4px; padding:12px 10px; display:flex; flex-direction:column; gap:2px; min-height:64px; color:var(--muted); }}
.pick b {{ font-size:14.5px; font-weight:700; line-height:1.35; overflow-wrap:anywhere; }}
.pick span {{ font-size:12.5px; }}
.pick.on {{ border:1px solid var(--accent); background:var(--accent); color:var(--surface); }}
.todo {{ background:var(--accent-soft); border-radius:8px; padding:4px 24px 20px; margin-top:60px; }}
.todo h2 {{ border-top:0; margin-top:0; padding-top:20px; }}
.todo ol {{ gap:10px; }}
.todo ol > li::marker {{ font-family:var(--serif); font-weight:700; color:var(--accent); }}
hr {{ border:0; border-top:1px solid var(--rule); margin:40px 0; }}
footer {{ font-size:13.5px; color:var(--muted); border-top:1px solid var(--rule); padding-top:16px; }}
@media (max-width:560px) {{
  body {{ font-size:15.5px; padding-inline:16px; }}
  .picks {{ grid-template-columns:repeat(2,minmax(0,1fr)); }}
  .todo {{ padding-inline:16px; }}
}}
</style>
<div class="page">
<header>
  <div class="eyebrow"><span>营事编集室 · 资讯日更</span><span>概要版 · 第二版 · 2026-09-20</span><span class="chip">方向已认可 · 先验证，再定是否切换</span></div>
  <h1>情报收集员改造方案<span>自定义客户端 + Codex App Server</span></h1>
  <blockquote class="goal"><span>改造目标 · Van 原话</span>系统能够更早、独立地找到我愿意采用的选题，减少我亲自补链接、反复解释和催促。内容要让户外潮流爱好者觉得有用、有趣、有料。全量读取、处理速度和记录完整度，都应服务于这个目标。</blockquote>
  <p class="lede">把情报收集员从 Hermes 上的单会话 agent，换成「自定义客户端 + Codex App Server」。取数、去重、计数交给代码；判断不再是「入选或淘汰」，而是<b>按 Van 给出的判断框架逐条打分、排序并写明依据，文字和代表性实图一起看</b>。引擎的流程定义、交付协议、两道人审都不变。</p>
</header>
<nav aria-label="目录"><ol>{toc}</ol></nav>
<article>
{body}
</article>
<footer>第二版，已按 9 月 19 日确认版反馈回改，逐项回应见第 15 节。协议、配置、代码结构等实现细节见《情报收集员_Codex采集引擎方案_20260918》。</footer>
</div>
'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "h2", toc.count("<li>"), "todo", 'class="todo"' in page, "figs", page.count('class="fig"'), "mermaid", page.count('class="mermaid"'))
