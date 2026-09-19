import json, os, sys, time, urllib.request, urllib.error, concurrent.futures as cf, re, random
D = os.path.dirname(os.path.abspath(__file__))
for ln in open(os.path.expanduser("~/.typesafe/.env")):
    m = re.match(r'\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*["\']?([^"\'\s]+)', ln)
    if m: KEY = m.group(1)
URL = "https://api.typesafe.ai/v1/systemone"

# 每个问题只问一件事；措辞按字面写全（Jev 按字面理解），判据写进 criteria。
QUESTIONS = {
  "new_product": {"type": "noul",
    "instructions": "Does `caption` announce a specific product becoming available: a new product, a new colorway or size, a limited edition, or a restock / re-release, with the product identifiable by name?",
    "criteria": {"true": "A specific named product is announced as newly released, upcoming, or restocked.",
                 "false": "No specific product release is announced (styling photos, scenery, user reposts, general brand imagery, event recaps, staff notes)."}},
  "concrete_details": {"type": "noul",
    "instructions": "Does `caption` give at least one concrete commercial fact about a product: a release date, a price, materials or specifications, or where it can be bought?"},
  "collaboration": {"type": "noul",
    "instructions": "Does `caption` announce a collaboration product made jointly by two or more named brands, designers, or artists?"},
  "store_or_event": {"type": "noul",
    "instructions": "Is the main subject of `caption` a physical place or gathering (a store opening, pop-up, exhibition, fair, workshop, or event) rather than a product?"},
  "styling_or_ugc": {"type": "noul",
    "instructions": "Is this post only a styling / outfit photo, a repost of a customer's photo, mood or scenery imagery, or an event recap, with no product release information?"},
  "promotion_admin": {"type": "noul",
    "instructions": "Is this post primarily a discount / sale promotion, a giveaway, a shipping or holiday notice, a recruitment notice, or another administrative announcement?"},
  "kind": {"type": "choice",
    "instructions": "What kind of post is this, judged from `caption` and `image_descriptions`?",
    "criteria": {"product_release": "Announces a specific new product, colorway, or limited edition",
                 "collaboration": "Announces a joint product between named brands or creators",
                 "restock": "Announces a restock or re-release of an existing product",
                 "store_or_event": "About a store, pop-up, exhibition, or event",
                 "brand_story": "Craft, history, behind-the-scenes, or founder story without a release",
                 "styling_or_ugc": "Outfit, styling, customer photo, scenery, or mood imagery",
                 "promotion_admin": "Sale, giveaway, notice, recruitment",
                 "other": "None of the above"}},
  "newsworthiness": {"type": "score",
    "instructions": "How newsworthy is this post for a Chinese-language magazine that reports new outdoor, camping, and urban-outdoor lifestyle products to enthusiasts?",
    "criteria": ["Nothing to report: no product, no event, no news of any kind.",
                 "Minor: an existing product shown again, or routine content with no new information.",
                 "Some news: a new product or colorway from a small or little-known maker, with few details.",
                 "Clear news: a named new product or collaboration with concrete details such as date, price, or specs.",
                 "Strong news: a notable brand's new release, a collaboration between known names, or an unusual, distinctive product that enthusiasts would want to read about."]},
}

def ask(post):
    state = {"account": post["account"], "caption": (post.get("description") or "")[:1800],
             "image_descriptions": (post.get("alt_text") or "")[:600]}
    body = json.dumps({"state": state, "model": "jev-1.13.0", "questions": QUESTIONS}).encode()
    for attempt in range(6):
        t0 = time.time()
        try:
            req = urllib.request.Request(URL, data=body, headers={"Authorization": "Bearer " + KEY, "Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=60) as r:
                out = json.loads(r.read())
            return {"shortcode": post["shortcode"], "ms": int((time.time() - t0) * 1000), "attempts": attempt + 1, **out}
        except urllib.error.HTTPError as e:
            code = e.code; msg = e.read()[:200].decode("utf-8", "ignore")
            if code in (429, 500, 502, 503, 504): time.sleep(min(8, 0.5 * 2 ** attempt) + random.random()); continue
            return {"shortcode": post["shortcode"], "error": f"http {code}: {msg}"}
        except Exception as e:
            time.sleep(1 + attempt); err = str(e)
    return {"shortcode": post["shortcode"], "error": "retries exhausted"}

posts = [json.loads(l) for l in open(f"{D}/posts.jsonl") if l.strip()]
n = int(sys.argv[1]) if len(sys.argv) > 1 else len(posts)
posts = posts[:n]
t0 = time.time(); done = 0
with cf.ThreadPoolExecutor(max_workers=12) as ex, open(f"{D}/answers.jsonl", "w") as f:
    for res in ex.map(ask, posts):
        f.write(json.dumps(res, ensure_ascii=False) + "\n"); done += 1
wall = time.time() - t0
rows = [json.loads(l) for l in open(f"{D}/answers.jsonl")]
ok = [r for r in rows if "answers" in r]
tok = sum(r["usage"]["input_tokens"] for r in ok)
ms = sorted(r["ms"] for r in ok)
print(f"posts={len(rows)} ok={len(ok)} errors={len(rows)-len(ok)} wall={wall:.1f}s")
print(f"input_tokens={tok} avg/post={tok//max(1,len(ok))} cost=${tok/1e6*0.042:.4f}")
print(f"latency ms p50={ms[len(ms)//2]} p90={ms[int(len(ms)*0.9)]} max={ms[-1]} retried={sum(1 for r in ok if r['attempts']>1)}")
for r in rows:
    if "error" in r: print("ERR", r["shortcode"], r["error"][:160]); break
