# -*- coding: utf-8 -*-
"""把 Van 认可的十篇公众号文章做成知识库内容。

用法：python build_examples.py <cache_dir> <out_dir> parse|describe|extract|render
  parse    抓取页面（缓存到 cache_dir/html）、解析成条目、下载图片（缩到 800 宽）→ out_dir/NN_*/article.json、article.md、images/
  describe 用视觉模型给每张图写「对应正文哪一点、画面、可否作配图」→ image_descriptions.json
  extract  逐篇、逐条拆出选题对象、具体变化或切口、读者价值、比较参照、编辑判断、待核推断 → extraction.json
  render   生成 index.json、README.md 与每篇的 范例拆解.md
"""
import re, html, json, time, datetime, urllib.request, os, sys, subprocess, shutil

CACHE, OUT, CMD = sys.argv[1], sys.argv[2], sys.argv[3]
IDS = ["kW77tsZvzphjZSdBw69xLw", "o2Ao7Tecypn6ay1bCzXBqw", "4qIAQ1Bchyz9gI8Mn5t6Iw", "79BIggQkZyh2ejdcIsuvRQ", "uw9gMsqbdHag--dcxu7ggQ",
       "062AbIDlAL-j12fMx53fZQ", "kk9MVmoT0aSP-43-38oyLw", "9ux3Z0JyYVDc7pkbrd7elQ", "rEzJY7rZmDjnG1q5ipcgpA", "k-3Z9WPd7ueh-OiKeL6Rvg"]
NOTE = {7: "追加满意案例", 8: "追加满意案例", 9: "追加满意案例", 10: "追加满意案例"}
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
os.makedirs(f"{CACHE}/html", exist_ok=True); os.makedirs(OUT, exist_ok=True)


def dec(x):
    x = x.replace("\\x26", "&").replace("\\x0a", "\n").replace("\\x22", '"').replace("\\x27", "'").replace("\\x3c", "<").replace("\\x3e", ">").replace("\\/", "/")
    return html.unescape(html.unescape(x))


def fetch(n, i):
    fp = f"{CACHE}/html/{n:02d}.html"
    if not os.path.exists(fp):
        req = urllib.request.Request(f"https://mp.weixin.qq.com/s/{i}", headers={"User-Agent": UA})
        open(fp, "wb").write(urllib.request.urlopen(req, timeout=30).read()); time.sleep(2.5)
    return open(fp, encoding="utf-8", errors="ignore").read()


def text_of(inner):
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", inner))).strip()


def blocks_type0(s):
    """普通图文：按 js_content 里 p / section / img 的出现顺序给出块。"""
    j = s.find('id="js_content"'); seg = s[j:]
    cut = seg.find("<script")
    seg = seg[:cut] if cut > 0 else seg
    out = []
    for m in re.finditer(r'<(p|section|h[1-6]|blockquote|img)\b([^>]*)>(.*?)(?=<(?:p|section|h[1-6]|blockquote|img)\b|$)', seg, re.S):
        tag, attrs, inner = m.group(1), m.group(2), m.group(3)
        if tag == "img":
            src = re.search(r'data-src="([^"]+)"', attrs)
            if src and "mmbiz" in src.group(1): out.append({"t": "img", "src": html.unescape(src.group(1))})
            continue
        txt = text_of(inner)
        if not txt: continue
        bold = ("<strong" in inner) or ("font-weight: bold" in inner) or ("font-weight:bold" in inner)
        out.append({"t": "p", "text": txt, "bold": bold})
    return out


def split_items(blocks):
    """合集：条目以「纯数字段落 → 品牌段 → 标题段」开头，到下一个数字或 END 结束。非合集：整篇一条。"""
    idx = [k for k, b in enumerate(blocks) if b["t"] == "p" and re.fullmatch(r"\d{1,2}", b["text"])]
    end = next((k for k, b in enumerate(blocks) if b["t"] == "p" and b["text"].strip().upper() == "END"), len(blocks))
    end = min([end] + [k for k, b in enumerate(blocks) if b["t"] == "p" and b["text"].startswith("往期回顾")])
    intro = [b["text"] for b in blocks[:idx[0]] if b["t"] == "p"] if idx else []
    items = []
    if not idx:
        body = blocks[:end]
        return intro, [{"seq": 1, "brand": None, "title": None, "paras": [b for b in body if b["t"] == "p"], "images": [b["src"] for b in body if b["t"] == "img"]}]
    for k, start in enumerate(idx):
        stop = idx[k + 1] if k + 1 < len(idx) else end
        seg = blocks[start + 1:stop]
        heads = [b for b in seg[:3] if b["t"] == "p"]
        brand = heads[0]["text"] if len(heads) > 0 else None
        # 合集的条目是「品牌行 + 标题行」；专题的章节只有一行标题，第二行已是正文
        title = heads[1]["text"] if len(heads) > 1 and len(heads[1]["text"]) <= 40 else None
        nhead = 2 if title else (1 if brand else 0)
        cnt = 0; rest = []
        for b in seg:
            if b["t"] == "p" and cnt < nhead: cnt += 1; continue
            rest.append(b)
        items.append({"seq": int(blocks[start]["text"]), "brand": brand, "title": title,
                      "paras": [b for b in rest if b["t"] == "p"], "images": [b["src"] for b in rest if b["t"] == "img"]})
    return intro, items


def parse_type8(s):
    urls = []
    for u in re.findall(r"cdn_url\s*[:=]\s*(?:JsDecode\()?'(https?:[^']+)'", s):
        u = dec(u)
        if u not in urls: urls.append(u)
    c = re.search(r"content_noencode\s*[:=]\s*(?:JsDecode\()?'((?:[^'\\]|\\.)*)'", s)
    body = dec(c.group(1)) if c else ""
    paras = [{"t": "p", "text": p.strip(), "bold": False} for p in re.split(r"\n+", body) if p.strip()]
    return paras, urls


def dl_image(url, dest):
    if os.path.exists(dest): return True
    tmp = dest + ".dl"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": UA, "Referer": "https://mp.weixin.qq.com/"})
        open(tmp, "wb").write(urllib.request.urlopen(req, timeout=40).read())
        r = subprocess.run(["sips", "-Z", "800", "-s", "format", "jpeg", "-s", "formatOptions", "72", tmp, "--out", dest], capture_output=True)
        if r.returncode != 0 or not os.path.exists(dest): shutil.move(tmp, dest)
        else: os.remove(tmp)
        return True
    except Exception as e:
        print("  下载失败", url[:60], e); return False


def slug(title):
    t = re.sub(r"[｜|:：×x].*$", "", title).strip()
    t = re.sub(r"[^\w一-鿿]+", "", t)[:16]
    return t or "article"


def parse():
    out = []
    for n, i in enumerate(IDS, 1):
        s = fetch(n, i)
        t = re.search(r'<meta property="og:title" content="([^"]*)"', s)
        title = html.unescape(t.group(1)) if t else f"文章{n}"
        st = re.search(r"item_show_type\s*[:=]\s*['\"]?(\d+)", s)
        ct = re.search(r"create_time\s*[:=]\s*(?:JsDecode\()?['\"](\d{10})", s) or re.search(r'var ct = "(\d{10})"', s)
        date = datetime.datetime.fromtimestamp(int(ct.group(1))).strftime("%Y-%m-%d") if ct else None
        kind8 = st and st.group(1) == "8"
        vol = re.search(r"[Vv]ol\.?\s*(\d+)", title)
        series = "营事专题室" if "专题室" in title else ("营事编集室" if "编集室" in title else "贴图文" if kind8 else "单篇")
        d = f"{OUT}/{n:02d}_{slug(title)}"; os.makedirs(f"{d}/images", exist_ok=True)
        if kind8:
            paras, urls = parse_type8(s); intro = []
            items = [{"seq": 1, "brand": None, "title": None, "paras": paras, "images": urls}]
            bt = title.split("：")
            if len(bt) >= 2 and len(bt[0]) <= 12: items[0]["brand"], items[0]["title"] = bt[0], "：".join(bt[1:])
        else:
            intro, items = split_items(blocks_type0(s))
        art = {"n": n, "id": i, "url": f"https://mp.weixin.qq.com/s/{i}", "title": title, "date": date, "series": series,
               "vol": int(vol.group(1)) if vol else None, "form": "贴图文" if kind8 else "普通图文", "note": NOTE.get(n),
               "status": "正式已发布", "evidence": "公众号公开页可访问，页面 create_time 为发布日期", "intro": intro, "items": []}
        total_img = 0
        for it in items:
            imgs = []
            for k, u in enumerate(it["images"], 1):
                fn = f"{n:02d}_{it['seq']:02d}_{k:02d}.jpg"
                ok = dl_image(u, f"{d}/images/{fn}")
                imgs.append({"k": k, "file": f"images/{fn}", "src": u, "downloaded": ok})
            total_img += len(imgs)
            art["items"].append({"seq": it["seq"], "brand": it["brand"], "title": it["title"],
                                 "text": "\n\n".join(p["text"] for p in it["paras"]), "chars": sum(len(p["text"]) for p in it["paras"]),
                                 "images": imgs})
        json.dump(art, open(f"{d}/article.json", "w"), ensure_ascii=False, indent=1)
        md = [f"# {title}", "", f"- 发布日期：{date}｜形态：{art['form']}｜系列：{series}{'（vol.' + str(art['vol']) + '）' if art['vol'] else ''}｜来源：{art['url']}", ""]
        if intro: md += ["## 导读", ""] + [p for p in intro] + [""]
        for it in art["items"]:
            head = "｜".join(x for x in [it["brand"], it["title"]] if x) or f"条目 {it['seq']}"
            md += [f"## {it['seq']}. {head}", ""]
            md += [it["text"], ""]
            md += [f"![{it['seq']}-{im['k']}]({im['file']})" for im in it["images"]] + [""]
        open(f"{d}/article.md", "w", encoding="utf-8").write("\n".join(md))
        out.append({"n": n, "dir": os.path.basename(d), "title": title, "date": date, "form": art["form"], "series": series, "vol": art["vol"], "items": len(art["items"]), "images": total_img, "note": NOTE.get(n)})
        print(f"{n:2} {date} {art['form']} 条目{len(art['items']):2} 图{total_img:3} | {title}")
    json.dump(out, open(f"{OUT}/_parse_summary.json", "w"), ensure_ascii=False, indent=1)


if CMD == "parse":
    parse()


# ---------------- 模型调用（Codex App Server，本机） ----------------
def start_codex():
    import threading, queue
    st = {"p": subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1),
          "lock": threading.Lock(), "pending": {}, "turns": {}, "i": 0}
    def send(o):
        with st["lock"]:
            st["p"].stdin.write(json.dumps(o, ensure_ascii=False) + "\n"); st["p"].stdin.flush()
    def reader():
        for line in st["p"].stdout:
            try: m = json.loads(line)
            except Exception: continue
            if "id" in m and "method" in m: send({"id": m["id"], "result": {"decision": "decline"}}); continue
            if "id" in m and m["id"] in st["pending"]: st["pending"][m["id"]].put(m); continue
            prm = m.get("params", {}); tid = prm.get("threadId"); T = st["turns"].get(tid)
            if not T: continue
            meth = m.get("method")
            if meth == "item/completed" and prm["item"].get("type") == "agentMessage": T["final"] = prm["item"].get("text")
            if meth == "thread/tokenUsage/updated": T["usage"] = prm["tokenUsage"]["total"]
            if meth == "turn/completed": T["status"] = prm["turn"].get("status"); T["err"] = prm["turn"].get("error"); T["done"].set()
    threading.Thread(target=reader, daemon=True).start()
    def call(method, params, timeout=120):
        with st["lock"]: st["i"] += 1; i = st["i"]; st["pending"][i] = queue.Queue()
        send({"id": i, "method": method, "params": params}); return st["pending"][i].get(timeout=timeout)
    call("initialize", {"clientInfo": {"name": "csw-kb-builder", "title": "kb", "version": "0.0.1"}, "capabilities": {"experimentalApi": True}}); send({"method": "initialized"})
    def run(instructions, inputs, schema, effort="low", wait=300):
        th = call("thread/start", {"cwd": CACHE, "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": True, "model": "gpt-5.6-sol", "developerInstructions": instructions})
        if "error" in th: return None, str(th["error"])[:200]
        tid = th["result"]["thread"]["id"]; st["turns"][tid] = {"done": threading.Event()}
        r = call("turn/start", {"threadId": tid, "effort": effort, "outputSchema": schema, "input": inputs})
        if "error" in r: return None, str(r["error"])[:200]
        st["turns"][tid]["done"].wait(wait); T = st["turns"][tid]
        try: return json.loads(T.get("final") or ""), None
        except Exception: return None, (T.get("err") or (T.get("final") or "")[:200] or "no output")
    st["run"] = run; st["stop"] = lambda: st["p"].terminate()
    return st


def load_articles():
    arts = []
    for d in sorted(os.listdir(OUT)):
        fp = f"{OUT}/{d}/article.json"
        if os.path.isfile(fp):
            a = json.load(open(fp, encoding="utf-8")); a["_dir"] = f"{OUT}/{d}"; arts.append(a)
    return arts


DESC_RULES = """你在给一篇中文户外生活方式公众号文章里的配图写说明，供以后的选题判断参考。读者是国内的户外、露营、城市户外读者。只依据给你的正文和图片，不要上网，不要调用任何工具。
对每张图分别回答：
what：画面里实际有什么（产品、场景、人物、文字、海报），不超过 40 个汉字。
relates_to：这张图对应正文的哪一点，引用正文里的关键词，不超过 30 个汉字；对应不上写「与正文无直接对应」。
missing：正文提到但这张图上看不到的关键信息，不超过 30 个汉字；没有写「无」。
kind：产品图 / 细节图 / 使用场景 / 人物穿搭 / 海报或文字图 / 截图 / 环境或空间 / 其他。
usable：这张图适不适合作为资讯配图（清晰、主体明确、无大段文字水印）。
按输入顺序输出，k 与输入编号一致。"""
DESC_SCHEMA = {"type": "object", "additionalProperties": False, "required": ["images"], "properties": {"images": {"type": "array", "items": {
    "type": "object", "additionalProperties": False, "required": ["k", "what", "relates_to", "missing", "kind", "usable"],
    "properties": {"k": {"type": "integer"}, "what": {"type": "string"}, "relates_to": {"type": "string"}, "missing": {"type": "string"},
                   "kind": {"type": "string", "enum": ["产品图", "细节图", "使用场景", "人物穿搭", "海报或文字图", "截图", "环境或空间", "其他"]}, "usable": {"type": "boolean"}}}}}}


def describe():
    import concurrent.futures as cf
    cx = start_codex(); jobs = []
    for a in load_articles():
        for it in a["items"]:
            imgs = [im for im in it["images"] if im.get("downloaded")]
            for c in range(0, len(imgs), 6): jobs.append((a, it, imgs[c:c + 6]))
    print("图片说明任务", len(jobs), "批")
    def work(job):
        a, it, imgs = job; t0 = time.time()
        head = "｜".join(x for x in [it.get("brand"), it.get("title")] if x) or a["title"]
        inp = [{"type": "text", "text": f"文章：{a['title']}（{a['date']}）\n条目：{head}\n正文：\n{it['text'][:2500]}\n\n下面依次是编号 {imgs[0]['k']} 到 {imgs[-1]['k']} 的图。"}]
        for im in imgs: inp.append({"type": "localImage", "path": os.path.join(a["_dir"], im["file"])})
        res, err = cx["run"](DESC_RULES, inp, DESC_SCHEMA)
        return a["n"], it["seq"], [im["k"] for im in imgs], res, err, round(time.time() - t0, 1)
    with cf.ThreadPoolExecutor(max_workers=3) as ex: results = list(ex.map(work, jobs))
    cx["stop"]()
    by_art = {}
    for n, seq, ks, res, err, secs in results:
        d = by_art.setdefault(n, {"n": n, "items": {}, "failed": []})
        if not res: d["failed"].append({"seq": seq, "ks": ks, "err": err}); print(f"  失败 文章{n} 条目{seq} 图{ks} {err}"); continue
        got = {x["k"]: x for x in res.get("images", [])}
        for k in ks:
            d["items"].setdefault(str(seq), {})[str(k)] = got.get(k) or {"k": k, "what": "", "relates_to": "", "missing": "", "kind": "其他", "usable": False, "note": "模型未返回"}
        print(f"  文章{n} 条目{seq} 图{ks} {secs}s")
    for a in load_articles():
        d = by_art.get(a["n"], {"n": a["n"], "items": {}, "failed": []})
        d["model"] = "gpt-5.6-sol via codex app-server"; d["built_at"] = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
        json.dump(d, open(f"{a['_dir']}/image_descriptions.json", "w"), ensure_ascii=False, indent=1)


EXTRACT_RULES = """你在拆解一篇 CSW（营事编集室）公众号文章，目的是让以后做选题的人理解这个公众号选什么、从什么角度报。读者是国内的户外、露营、城市户外读者。只依据给你的正文，不要上网，不要调用任何工具。
对整篇：topic_summary 用一句话说这篇讲了什么；expression_notes 用两到四句说这篇怎样讲清专业知识、呈现细节、自然表达（举正文里的具体句段）。
对每个条目（合集里的每一则资讯，或单篇文章本身）：
subject 选题对象：品牌、产品或事件，一句。
change_or_angle 具体变化或报道切口：这件事具体变了什么、文章从哪个角度切入，一到两句。
reader_value 读者价值：对国内读者有什么用或有什么意思，一句。
comparison 比较参照：文章拿什么做参照（旧款、同类、历史、别的品牌），没有写「无」。
editorial_judgement 编辑判断：文章里有事实支持的判断句，逐字引用一到两句。
inferred 待核推断：你推断出来但正文没有明说的选题理由，每条一句，可以为空数组；不要把推断写成作者说过的话。
type：新品 / 联名 / 补货或再版 / 活动或展会 / 门店或空间 / 文化或历史 / 品牌故事 / 专题 / 其他。
dims：六个维度各判「成立 / 不成立 / 不明」并配一句依据——change 具体变化、use 使用关联（和读者怎么用、穿、去有关）、gain 信息增量（有日期、价格、规格、渠道等新信息）、compare 比较参照、explain 值得解释（有需要 CSW 解释的专业点或背景）、csw 调性适配（符合这个号一贯的户外生活方式趣味）。
seq 与输入的条目编号一致。"""
DIM = {"type": "object", "additionalProperties": False, "required": ["verdict", "basis"], "properties": {"verdict": {"type": "string", "enum": ["成立", "不成立", "不明"]}, "basis": {"type": "string"}}}
EXTRACT_SCHEMA = {"type": "object", "additionalProperties": False, "required": ["topic_summary", "expression_notes", "items"], "properties": {
    "topic_summary": {"type": "string"}, "expression_notes": {"type": "string"},
    "items": {"type": "array", "items": {"type": "object", "additionalProperties": False,
        "required": ["seq", "subject", "change_or_angle", "reader_value", "comparison", "editorial_judgement", "inferred", "type", "dims"],
        "properties": {"seq": {"type": "integer"}, "subject": {"type": "string"}, "change_or_angle": {"type": "string"}, "reader_value": {"type": "string"},
                       "comparison": {"type": "string"}, "editorial_judgement": {"type": "string"}, "inferred": {"type": "array", "items": {"type": "string"}},
                       "type": {"type": "string", "enum": ["新品", "联名", "补货或再版", "活动或展会", "门店或空间", "文化或历史", "品牌故事", "专题", "其他"]},
                       "dims": {"type": "object", "additionalProperties": False, "required": ["change", "use", "gain", "compare", "explain", "csw"],
                                "properties": {k: DIM for k in ["change", "use", "gain", "compare", "explain", "csw"]}}}}}}}


def extract():
    import concurrent.futures as cf
    cx = start_codex(); arts = load_articles()
    def work(a):
        t0 = time.time()
        parts = [f"文章标题：{a['title']}\n发布日期：{a['date']}\n系列：{a['series']}{' vol.' + str(a['vol']) if a['vol'] else ''}"]
        if a["intro"]: parts.append("导读：" + " ".join(a["intro"])[:600])
        for it in a["items"]:
            head = "｜".join(x for x in [it.get("brand"), it.get("title")] if x) or "（整篇）"
            parts.append(f"\n【条目 {it['seq']}】{head}\n{it['text'][:3500]}")
        res, err = cx["run"](EXTRACT_RULES, [{"type": "text", "text": "\n".join(parts)}], EXTRACT_SCHEMA, effort="medium", wait=420)
        return a, res, err, round(time.time() - t0, 1)
    with cf.ThreadPoolExecutor(max_workers=3) as ex: results = list(ex.map(work, arts))
    cx["stop"]()
    for a, res, err, secs in results:
        if not res: print(f"  失败 文章{a['n']} {err}"); continue
        res["model"] = "gpt-5.6-sol via codex app-server"; res["built_at"] = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
        json.dump(res, open(f"{a['_dir']}/extraction.json", "w"), ensure_ascii=False, indent=1)
        print(f"  文章{a['n']} 条目{len(res['items'])} {secs}s")


def render():
    arts = load_articles(); index = []; kb_lines = []
    for a in arts:
        ex = json.load(open(f"{a['_dir']}/extraction.json", encoding="utf-8")) if os.path.exists(f"{a['_dir']}/extraction.json") else {"items": []}
        ds = json.load(open(f"{a['_dir']}/image_descriptions.json", encoding="utf-8")) if os.path.exists(f"{a['_dir']}/image_descriptions.json") else {"items": {}}
        exi = {x["seq"]: x for x in ex.get("items", [])}
        md = [f"# 范例拆解 · {a['title']}", "", f"- 发布日期 {a['date']}｜{a['form']}｜{a['series']}{'（vol.' + str(a['vol']) + '）' if a['vol'] else ''}｜{a['url']}",
              f"- 发布状态：{a['status']}（{a['evidence']}）" + (f"｜Van 备注：{a['note']}" if a.get("note") else ""), ""]
        if ex.get("topic_summary"): md += ["## 整篇", "", f"**讲了什么**：{ex['topic_summary']}", "", f"**表达手法**：{ex['expression_notes']}", ""]
        items_out = []
        for it in a["items"]:
            e = exi.get(it["seq"], {}); head = "｜".join(x for x in [it.get("brand"), it.get("title")] if x) or f"条目 {it['seq']}"
            md += [f"## {it['seq']}. {head}", ""]
            if e:
                md += [f"- 类型：{e['type']}", f"- 选题对象：{e['subject']}", f"- 具体变化或切口：{e['change_or_angle']}", f"- 读者价值：{e['reader_value']}",
                       f"- 比较参照：{e['comparison']}", f"- 编辑判断（原文）：{e['editorial_judgement']}"]
                md += ["- 待核推断：" + ("；".join(e["inferred"]) if e["inferred"] else "无"), ""]
                md += ["| 维度 | 判定 | 依据 |", "|---|---|---|"]
                for k, name in [("change", "具体变化"), ("use", "使用关联"), ("gain", "信息增量"), ("compare", "比较参照"), ("explain", "值得解释"), ("csw", "调性适配")]:
                    md.append(f"| {name} | {e['dims'][k]['verdict']} | {e['dims'][k]['basis']} |")
                md.append("")
            dd = ds.get("items", {}).get(str(it["seq"]), {})
            if it["images"]:
                md += ["| 图 | 画面 | 对应正文 | 图上没有的 | 类型 | 可作配图 |", "|---|---|---|---|---|---|"]
                for im in it["images"]:
                    x = dd.get(str(im["k"]), {})
                    md.append(f"| [{im['k']}]({im['file']}) | {x.get('what', '')} | {x.get('relates_to', '')} | {x.get('missing', '')} | {x.get('kind', '')} | {'是' if x.get('usable') else '否' if x else ''} |")
                md.append("")
            items_out.append({"seq": it["seq"], "brand": it.get("brand"), "title": it.get("title"), "chars": it["chars"], "images": len(it["images"]),
                              "type": e.get("type"), "subject": e.get("subject"), "dims": {k: v["verdict"] for k, v in e.get("dims", {}).items()} if e else None})
        open(f"{a['_dir']}/范例拆解.md", "w", encoding="utf-8").write("\n".join(md))
        index.append({"n": a["n"], "dir": os.path.basename(a["_dir"]), "id": a["id"], "url": a["url"], "title": a["title"], "date": a["date"], "form": a["form"],
                      "series": a["series"], "vol": a["vol"], "note": a.get("note"), "status": a["status"], "topic_summary": ex.get("topic_summary"), "items": items_out})
        kb_lines.append(json.dumps({"platform": "wechat", "account": "营事编集室", "post_id": a["id"], "url": a["url"], "published_at": a["date"], "title": a["title"],
                                    "body_text": "\n\n".join(("｜".join(x for x in [it.get("brand"), it.get("title")] if x) + "\n" + it["text"]) for it in a["items"]),
                                    "source": "import", "state": "published", "raw_json": {"is_reference": True, "reference_note": a.get("note") or "Van 认可的范例", "form": a["form"], "series": a["series"], "vol": a["vol"]},
                                    "items": [{"seq": it["seq"], "brand": it.get("brand"), "product": exi.get(it["seq"], {}).get("subject"), "angle": exi.get(it["seq"], {}).get("change_or_angle"), "title": it.get("title"), "split_by": "auto"} for it in a["items"]]}, ensure_ascii=False))
    json.dump(index, open(f"{OUT}/index.json", "w"), ensure_ascii=False, indent=1)
    open(f"{OUT}/kb_import.jsonl", "w", encoding="utf-8").write("\n".join(kb_lines) + "\n")
    rows = "\n".join(f"| {a['n']} | {a['date']} | {a['form']}{'·' + a['series'] if a['series'] not in ('单篇', '贴图文') else ''}{' vol.' + str(a['vol']) if a['vol'] else ''} | [{a['title'].replace('|', '｜')}]({a['dir']}/范例拆解.md) | {len(a['items'])} | {sum(i['images'] for i in a['items'])} | {a['note'] or ''} |" for a in index)
    readme = f"""# 刊发知识库 · 范例（Van 认可的十篇公众号文章）

构建日期：{datetime.datetime.now().strftime('%Y-%m-%d')}｜生成器：`docs/tools/kb/build_examples.py`｜发布状态：十篇都是公众号公开页可访问的**正式已发布**文章，日期取页面 create_time。

用途：让收集员、研究员、主编理解 CSW 选什么、从什么角度报；其次给文案与主编校准表达。范例里没有明确体现的判断标为「待核推断」，不写成 Van 说过的理由。

| # | 日期 | 形态 | 标题 | 条目 | 图 | 备注 |
|---|---|---|---|---|---|---|
{rows}

每篇一个目录：`article.md`（正文与图位）、`article.json`（结构化正文与图片清单）、`images/`（缩到 800 宽的图）、`image_descriptions.json`（每张图：画面、对应正文哪一点、图上没有的、类型、可否作配图）、`extraction.json` 与 `范例拆解.md`（选题对象、具体变化或切口、读者价值、比较参照、编辑判断原文、待核推断、六维度判定）。

`index.json` 汇总十篇与每条的类型、选题对象、六维度判定；`kb_import.jsonl` 是按引擎 `ledger_published_posts` / `ledger_post_items` 字段整理的导入行（`raw_json.is_reference = true`），待引擎加上范例标记与导入命令后入库。

图片说明与拆解由本机 Codex App Server（gpt-5.6-sol）生成，模型版本与生成时间写在各 json 里；人工核对后可直接改 md 与 json。
"""
    open(f"{OUT}/README.md", "w", encoding="utf-8").write(readme)
    print("index", len(index), "篇；条目", sum(len(a["items"]) for a in index), "；图", sum(i["images"] for a in index for i in a["items"]))


if CMD == "describe": describe()
elif CMD == "extract": extract()
elif CMD == "render": render()
