import json, os, sys, time, urllib.request, urllib.error, concurrent.futures as cf, re, random
D = os.path.dirname(os.path.abspath(__file__))
for ln in open(os.path.expanduser("~/.typesafe/.env")):
    m = re.match(r'\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*["\']?([^"\'\s]+)', ln)
    if m: KEY = m.group(1)
URL = "https://api.typesafe.ai/v1/systemone"
READER = "readers of a Chinese-language magazine about outdoor, camping, and urban-outdoor lifestyle products"
Q = {
 "novelty": {"type":"score","instructions":"How new is the thing this post is about? Judge only from `caption`.",
  "criteria":["No product or news at all: scenery, daily life, greetings, staff notes.",
              "An existing product shown again with no new information.",
              "A restock, re-release, or a new color or size of an existing product.",
              "A newly released or upcoming product, model, or collaboration item.",
              "A debut of something first-of-its-kind: an entirely new product line, a first-ever collaboration, or a concept not seen before."]},
 "distinctiveness": {"type":"score","instructions":"How distinctive and valuable is the product or subject itself, apart from whether it is new? Judge from `caption`.",
  "criteria":["No identifiable product or subject.",
              "A commonplace item with nothing notable: ordinary seasonal clothing, basic goods, routine merchandise.",
              "A solid product with one notable feature, material, or function.",
              "A distinctive product: unusual design, notable technology or material, serious craftsmanship, or a real story behind it.",
              "An exceptional object enthusiasts would talk about: iconic, innovative, rare, collectible, or a striking cross-field creation such as art, vehicles, or architecture meeting outdoor gear."]},
 "reader_relevance": {"type":"score","instructions":f"How relevant is this post to {READER}, who live in China and mostly cannot visit overseas local stores or events?",
  "criteria":["Purely local information with no takeaway for them: a local shop notice, opening hours, a local event or workshop.",
              "Mostly local: an overseas store opening, pop-up, or exhibition with little product content.",
              "A product of some interest that is regional or hard for them to obtain.",
              "A product or story that is interesting regardless of where the reader lives.",
              "Globally notable, or something these readers can directly follow, buy, or will certainly hear about."]},
 "info_completeness": {"type":"score","instructions":"How much concrete, reportable information about the product does `caption` contain?",
  "criteria":["None.","Only a product or brand name.","A name plus one concrete fact.",
              "A name plus several facts such as release date, price, materials, or specifications.",
              "Rich detail: specifications, price, release date, where to buy, and background or design intent."]},
 "brand_weight": {"type":"score","instructions":"How much name recognition and talk value do the brands, designers, or creators named in this post carry in the outdoor and urban-outdoor lifestyle scene?",
  "criteria":["No brand or creator identifiable.","A small local maker or shop known to few.",
              "An established niche brand known to enthusiasts.",
              "A well-known brand in this scene.",
              "A major name, or a notable collaboration that crosses fields such as fashion, art, automobiles, or design."]},
}
def ask(post):
    state={"account":post["account"],"caption":(post.get("description") or "")[:1800]}
    body=json.dumps({"state":state,"model":"jev-1.13.0","questions":Q}).encode()
    for attempt in range(6):
        try:
            req=urllib.request.Request(URL,data=body,headers={"Authorization":"Bearer "+KEY,"Content-Type":"application/json"})
            with urllib.request.urlopen(req,timeout=60) as r: out=json.loads(r.read())
            return {"shortcode":post["shortcode"],**out}
        except urllib.error.HTTPError as e:
            if e.code in (429,500,502,503,504): time.sleep(min(8,0.5*2**attempt)+random.random()); continue
            return {"shortcode":post["shortcode"],"error":f"http {e.code}: {e.read()[:200]}"}
        except Exception as e: time.sleep(1+attempt)
    return {"shortcode":post["shortcode"],"error":"retries exhausted"}
posts=[json.loads(l) for l in open(f"{D}/posts.jsonl") if l.strip()]
t0=time.time()
with cf.ThreadPoolExecutor(max_workers=12) as ex, open(f"{D}/dims.jsonl","w") as f:
    for res in ex.map(ask,posts): f.write(json.dumps(res,ensure_ascii=False)+"\n")
rows=[json.loads(l) for l in open(f"{D}/dims.jsonl")]; ok=[r for r in rows if "answers" in r]
tok=sum(r["usage"]["input_tokens"] for r in ok)
print(f"posts={len(rows)} ok={len(ok)} wall={time.time()-t0:.0f}s tokens={tok} cost=${tok/1e6*0.042:.4f}")
for r in rows:
    if "error" in r: print("ERR",r["error"][:200]); break
