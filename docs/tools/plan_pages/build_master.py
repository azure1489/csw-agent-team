# -*- coding: utf-8 -*-
"""总方案页：markdown + 五张设计图（系统位置、完整流程、采集器模型、候选状态、部署拓扑）。
用法：python build_master.py 总方案.md out.html（需要 markdown 库）"""
import re, sys, os, markdown
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import diagrams as dg
src, out = sys.argv[1], sys.argv[2]
md = open(src, encoding="utf-8").read()
m = re.match(r"\A# [^\n]*\n\n(?P<meta>[^\n]*)\n\n\*\*一句话\*\*：(?P<lede>[^\n]*)\n\n---\n", md)
assert m, "页首格式不符：# 标题 / 元信息行 / **一句话** / ---"
meta, lede = m.group("meta"), m.group("lede")
md = md[m.end():]
body = markdown.markdown(md, extensions=["tables", "sane_lists", "toc", "fenced_code"],
        extension_configs={"toc": {"slugify": lambda v, s: re.sub(r"[^\w一-鿿]+", "-", v).strip("-").lower()}})
body = body.replace("<table>", '<div class="table-wrap"><table>').replace("</table>", "</table></div>")
FIGS = {"system": dg.fig_system, "full": dg.fig_full, "collectors": dg.fig_collectors, "state": dg.fig_state, "deploy": dg.fig_deploy}
for key, fn in FIGS.items():
    tag = f"<!-- FIG:{key} -->"
    assert tag in body, f"缺占位 {tag}"
    body = body.replace(tag, fn())
toc = "".join(f'<li><a href="#{m.group(1)}">{m.group(2)}</a></li>' for m in re.finditer(r'<h2 id="([^"]+)">(.*?)</h2>', body))
page = f'''<title>收集员工作台总方案</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&family=Noto+Serif+SC:wght@600;700&display=swap">
<style>
:root {{ --ground:#F4F5F7; --surface:#FFFFFF; --ink:#1B1F2A; --muted:#5F6673; --rule:#DCDFE6; --accent:#4F46E5; --accent-soft:#EEF0FD; --warn:#B23B2A; --code-bg:#ECEEF3;
  --sans:"Noto Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",system-ui,sans-serif; --serif:"Noto Serif SC","Songti SC",serif; --mono:"IBM Plex Mono",Menlo,monospace; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --warn:#F08A76; --code-bg:#22263A; }} }}
:root[data-theme="dark"] {{ --ground:#12141A; --surface:#1A1D26; --ink:#E6E8EE; --muted:#9AA1AE; --rule:#2C3140; --accent:#8B85F6; --accent-soft:#232647; --warn:#F08A76; --code-bg:#22263A; }}
body {{ background:var(--ground); color:var(--ink); font-family:var(--sans); font-size:16px; line-height:1.85; padding-inline:20px; padding-block:44px 96px; }}
.page {{ max-width:1080px; margin-inline:auto; display:flex; flex-direction:column; gap:28px; }}
.eyebrow {{ font-size:13px; color:var(--muted); letter-spacing:.08em; display:flex; flex-wrap:wrap; gap:4px 14px; }}
.chip {{ border:1px solid var(--accent); color:var(--accent); padding:0 8px; border-radius:3px; }}
h1 {{ font-family:var(--serif); font-weight:700; font-size:clamp(28px,5vw,38px); line-height:1.3; margin:12px 0 14px; text-wrap:balance; }}
h1 span {{ display:block; font-family:var(--sans); font-size:.5em; font-weight:500; color:var(--accent); margin-top:6px; }}
.lede {{ font-size:17px; line-height:1.9; margin:0; max-width:46em; }}
.meta {{ font-size:13.5px; color:var(--muted); margin:10px 0 0; max-width:60em; }}
nav ol {{ list-style:none; margin:0; padding:0; display:flex; flex-wrap:wrap; gap:6px 8px; }}
nav a {{ display:block; font-size:13.5px; padding:3px 11px; border:1px solid var(--rule); border-radius:3px; background:var(--surface); color:var(--ink); text-decoration:none; }}
nav a:hover {{ border-color:var(--accent); color:var(--accent); }}
article {{ min-width:0; }}
article h2 {{ font-family:var(--serif); font-weight:700; font-size:24px; margin:56px 0 14px; padding-top:18px; border-top:2px solid var(--ink); text-wrap:balance; }}
article h3 {{ font-size:17px; font-weight:700; margin:34px 0 8px; color:var(--accent); }}
article p {{ margin:12px 0; max-width:46em; }}
article ul, article ol {{ padding-left:1.4em; margin:12px 0; display:flex; flex-direction:column; gap:6px; max-width:46em; }}
code {{ font-family:var(--mono); font-size:.86em; background:var(--code-bg); padding:1px 5px; border-radius:3px; }}
pre {{ background:var(--code-bg); border:1px solid var(--rule); border-radius:6px; padding:14px 16px; overflow-x:auto; font-size:13px; line-height:1.6; margin:16px 0; }}
pre code {{ background:none; padding:0; font-size:inherit; }}
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
  <div class="eyebrow"><span>营事编集室 · 情报收集员改造</span><span>总方案 · 2026-09-21</span><span class="chip">合并稿</span></div>
  <h1>情报收集员工作台<span>总方案</span></h1>
  <p class="lede">{lede}</p>
  <p class="meta">{meta}</p>
</header>
<nav aria-label="目录"><ol>{toc}</ol></nav>
<article>{body}</article>
<footer>本文合并此前全部方案文档的现行口径；细节以此为准，界面细节见《界面设计稿》，协议字段与配置见开发版。</footer>
</div>'''
open(out, "w", encoding="utf-8").write(page)
print("bytes", len(page.encode()), "figs", page.count("<figure"), "h2", page.count("<h2 "))
