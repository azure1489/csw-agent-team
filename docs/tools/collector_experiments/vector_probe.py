# -*- coding: utf-8 -*-
"""阶段 0.5：Qwen3-VL 向量与重排服务吞吐实测（agent 主机本机 127.0.0.1:8022，免密钥）。"""
import base64, glob, json, statistics, threading, time, urllib.request, urllib.error

BASE = "http://127.0.0.1:8022"

def post(path, payload, timeout=300):
    """显存不够时服务以 500 返回 CUDA OOM——调用方必须能识别并降批，生产代码同理。"""
    req = urllib.request.Request(BASE + path, data=json.dumps(payload).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"HTTP {e.code} {e.read()[:140].decode('utf-8','replace')}") from None

def timed(label, fn, n):
    try:
        t0 = time.time(); r = fn(); ms = (time.time() - t0) * 1000
        print(f"  {label}：{ms:>7.0f}ms  每条 {ms/n:>6.1f}ms")
        return r, ms
    except RuntimeError as e:
        print(f"  {label}：失败 {str(e)[:100]}")
        return None, None

def load(n=60):
    posts = []
    for f in sorted(glob.glob("/tmp/kbprobe/w_*.json")):
        d = json.load(open(f)); d = d.get("data", d)
        for p in d.get("items") or []:
            ml = [m for m in (p.get("mediaList") or []) if m.get("mediaType") == "Photo"]
            if p.get("contentType") in ("Image", "Carousel") and ml and len(ml) == len(p.get("mediaList") or []):
                posts.append(p)
    return posts[:n]

def img_b64(url, w=768):
    sep = "&" if "?" in url else "?"
    with urllib.request.urlopen(f"{url}{sep}x-oss-process=image/resize,w_{w}/format,jpg/quality,q_80", timeout=60) as r:
        return base64.b64encode(r.read()).decode()

def part_text(t): return {"type": "text", "text": t}
def part_img(b): return {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{b}"}}

posts = load()
texts = [(p.get("description") or "")[:1500] for p in posts if (p.get("description") or "").strip()]
print("health:", urllib.request.urlopen(BASE + "/health", timeout=30).status, f" 底料 {len(texts)} 段")
print(f"正文长度 中位 {statistics.median(len(t) for t in texts):.0f} 字，最长 {max(len(t) for t in texts)}")

print("\n纯文本向量化（input 是扁平字符串数组）")
for bs in (1, 4, 8, 16, 24):
    r, _ = timed(f"批 {bs:>2}", lambda bs=bs: post("/v1/embeddings", {"input": (texts * 4)[:bs]}), bs)
    if r and bs == 1:
        print(f"      维度 {len(r['data'][0]['embedding'])}")

print("\n图文融合向量（一条 = 正文 + 1 张图）")
b64s = [img_b64(p["mediaList"][0]["mediaUrl"]) for p in posts[:8]]
print(f"  w_768 单张 base64 中位 {statistics.median(len(b)/1024 for b in b64s):.0f} KB")
for bs in (1, 2, 4, 8):
    timed(f"批 {bs}", lambda bs=bs: post("/v1/embeddings", {"input": [
        [part_text((posts[i].get("description") or "x")[:800]), part_img(b64s[i % len(b64s)])]
        for i in range(bs)]}), bs)

print("\n纯图片向量")
for bs in (1, 2, 4, 8):
    timed(f"批 {bs}", lambda bs=bs: post("/v1/embeddings", {"input": [
        [part_img(b64s[i % len(b64s)])] for i in range(bs)]}), bs)

print("\nrerank（文本，每段截 600 字）")
for n in (8, 24, 50):
    r, _ = timed(f"{n:>2} 段", lambda n=n: post("/v1/rerank", {
        "query": "轻量化帐篷的结构改款",
        "documents": [d[:600] for d in (texts * 3)[:n]],
        "instruction": "Given a search query, retrieve relevant candidates that answer the query."}), n)
    if r and n == 8:
        print(f"      返回键 {list(r)}")

print("\n抢卡：向量化与 rerank 各 4 次同时打（单卡串行，看互相拖多少）")
lat = {"emb": [], "rr": []}
def emb():
    for _ in range(4):
        t0 = time.time()
        try: post("/v1/embeddings", {"input": (texts * 2)[:8]})
        except RuntimeError: pass
        lat["emb"].append((time.time() - t0) * 1000)
def rr():
    for _ in range(4):
        t0 = time.time()
        try: post("/v1/rerank", {"query": "帐篷", "documents": [d[:600] for d in (texts * 2)[:24]]})
        except RuntimeError: pass
        lat["rr"].append((time.time() - t0) * 1000)
t0 = time.time(); ts = [threading.Thread(target=emb), threading.Thread(target=rr)]
[t.start() for t in ts]; [t.join() for t in ts]; wall = (time.time() - t0) * 1000
print(f"  墙钟 {wall:.0f}ms   向量化 8 条中位 {statistics.median(lat['emb']):.0f}ms"
      f"   rerank 24 段中位 {statistics.median(lat['rr']):.0f}ms")
