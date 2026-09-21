# -*- coding: utf-8 -*-
"""架构与部署分析页：markdown + 部署拓扑图。用法：python build_arch.py 分析.md out.html"""
import re, sys, os, markdown
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import diagrams as dg
src, out = sys.argv[1], sys.argv[2]
md = open(src, encoding="utf-8").read()
md = re.sub(r"\A# [^\n]*\n\n[^\n]*\n\n(\*\*一句话\*\*[^\n]*\n)\n---\n", "", md, count=1)
body = markdown.markdown(md, extensions=["tables", "sane_lists", "toc"],
        extension_configs={"toc": {"slugify": lambda v, s: re.sub(r"[^\w一-鿿]+", "-", v).strip("-").lower()}})
body = body.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
body = body.replace("<!-- FIG:deploy -->", dg.fig_deploy())
toc = "".join(f'<li><a href="#{m.group(1)}">{m.group(2)}</a></li>' for m in re.finditer(r'<h2 id="([^"]+)">(.*?)</h2>', body))
page = f'''<title>收集员工作台架构与部署</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&family=Noto+Serif+SC:wght@600;700&display=swap">
<style>
:root {{ --ground:#F4F5F7; --surface:#FFFFFF; --ink:#1B1F2A; --muted:#5F6673; --rule:#DCDFE6; --accent:#4F46E5; --accent-soft:#EEF0FD; --warn:#B23B2A; --code-bg:#ECEEF3;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif; --serif:"Noto Serif SC","Songti SC",serif; --mono:"IBM Plex Mono",Menlo,monospace; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --warn:#F08A76; --code-bg:#22263A; }} }}
:root[data-theme="dark"] {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --warn:#F08A76; --code-bg:#22263A; }}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:16px; line-height:1.85; padding-inline:20px; padding-block:44px 96px; }}
.page {{ max-width:1000px; margin-inline:auto; display:flex; flex-direction:column; gap:28px; }}
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
article h3 {{ font-size:17px; font-weight:700; margin:34px 0 8px; color:var(--accent); }}
article p {{ margin:12px 0; max-width:46em; }}
article ul, article ol {{ padding-left:1.4em; margin:12px 0; display:flex; flex-direction:column; gap:6px; max-width:46em; }}
code {{ font-family:var(--mono); font-size:.86em; background:var(--code-bg); padding:1px 5px; border-radius:3px; }}
.table-wrap {{ overflow-x:auto; margin:18px 0; border:1px solid var(--rule); border-radius:6px; background:var(--surface); }}
table {{ border-collapse:collapse; width:100%; font-size:14.5px; line-height:1.7; }}
th, td {{ text-align:left; vertical-align:top; padding:10px 13px; border-bottom:1px solid var(--rule); min-width:6em; }}
th {{ font-size:13px; color:var(--muted); background:var(--ground); font-weight:700; white-space:nowrap; }}
tr:last-child td {{ border-bottom:0; }}
.diagram {{ margin:22px 0; background:var(--surface); border:1px solid var(--rule); border-radius:8px; padding:18px 18px 12px; overflow-x:auto; }}
.diagram svg {{ display:block; min-width:640px; }}
.diagram figcaption {{ font-size:13.5px; color:var(--muted); line-height:1.6; margin-top:10px; max-width:60em; }}
footer {{ font-size:13.5px; color:var(--muted); border-top:1px solid var(--rule); padding-top:16px; }}
@media (max-width:560px) {{ body {{ font-size:15.5px; padding-inline:16px; }} }}
</style>
<div class="page">
<header>
  <div class="eyebrow"><span>营事编集室 · 情报收集员改造</span><span>分析稿 · 2026-09-21</span><span class="chip">动工前底数</span></div>
  <h1>情报收集员工作台<span>架构与部署分析</span></h1>
  <p class="lede">新东西只有一个 Rust 进程和它托管的工作台，放在 agent 主机上，和已经在那台机上的向量服务同机；引擎主机与 csw-agent 主机不动，各只加一个接口。真正的风险不在架构，在 agent 主机已经很挤，以及三个外部依赖各是单点。数据为 9/21 实查。</p>
</header>
<nav aria-label="目录"><ol>{toc}</ol></nav>
<article>{body}</article>
<footer>配套：《情报收集员工作台 · 完整流程》确认稿、《设计说明》、《情报收集员改造方案》开发版。</footer>
</div>'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "figs", page.count("<figure"))
