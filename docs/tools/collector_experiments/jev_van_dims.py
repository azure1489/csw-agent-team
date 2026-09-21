# -*- coding: utf-8 -*-
"""用 TypeSafe Jev 按 Van 的六个维度给 13 个真实事件做初评（三档：不成立 / 不明 / 成立），与 Codex 判定和 Van 决定对照。
用法：python jev_van_dims.py events_scored.json events_meta.json out.json"""
import json, os, sys, time, urllib.request, urllib.error, re, random, concurrent.futures as cf
KEY = None
for ln in open(os.path.expanduser("~/.typesafe/.env")):
    m = re.match(r'\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*["\']?([^"\'\s]+)', ln)
    if m: KEY = m.group(1)
URL = "https://api.typesafe.ai/v1/systemone"
EV = [e for e in json.load(open(sys.argv[1], encoding="utf-8")) if e.get("scores")]
META = json.load(open(sys.argv[2], encoding="utf-8"))
FRAME = ("You judge one Instagram post for the editors of CSW, a Chinese-language magazine about outdoor, camping and urban-outdoor lifestyle. "
         "Their core question: does this thing contain a concrete enough change that it is worth CSW explaining it to readers once? "
         "Non-new products can still qualify through a well-grounded design, culture, history or current-affairs link. Brand fame is not a criterion. "
         "Judge only from the fields in state: `caption` (original), `image_description` (what the photos show, written by a vision model), `tags` (labels assigned by CSW's own backend), `heat` (likes relative to the account's usual level).")
LV = lambda no, unclear, yes: [{"what": no}, {"what": unclear}, {"what": yes}]
Q = {
 "change": {"type": "score", "instructions": "Does the post describe a concrete change: a new product, version, collaboration, material, design, event or store, with specifics?",
            "criteria": LV("No concrete change: a reshown product, greeting, mood shot, generic promotion.", "Something may have changed but the post does not say what specifically.", "A specific, nameable change is stated.")},
 "use": {"type": "score", "instructions": "Does the post connect to how a reader would use, wear, carry or go somewhere with the thing?",
         "criteria": LV("No usage relation; only appearance or announcement.", "Usage is implied but not described.", "Usage, wearing, carrying or a scenario is described concretely.")},
 "gain": {"type": "score", "instructions": "Does the post add reportable information such as release date, price, specifications, sales channel, or design intent?",
          "criteria": LV("Only a name or nothing.", "One vague fact.", "Several concrete facts a reporter could cite.")},
 "compare": {"type": "score", "instructions": "Does the post give or clearly allow a comparison reference: previous model, standard version, similar products, or history?",
             "criteria": LV("No reference and none is implied.", "A reference is hinted but not stated.", "A comparison reference is stated or obviously available.")},
 "explain": {"type": "score", "instructions": "Is there a professional point or background that would be worth CSW explaining to readers (material, technology, craft, culture, history)?",
             "criteria": LV("Nothing to explain.", "Possibly, but the post gives too little.", "A clear point that rewards explanation.")},
 "csw": {"type": "score", "instructions": "Does the subject fit CSW's taste: outdoor, camping and urban-outdoor lifestyle with a design or culture angle, for readers in China?",
         "criteria": LV("Off-topic or purely local shop notice.", "Related but marginal or hard to relate to readers in China.", "Squarely in CSW's territory.")},
 "kind": {"type": "choice", "instructions": "What kind of post is this?", "criteria": {"new_product": "a newly released or upcoming product or model", "collaboration": "a collaboration between brands, designers or artists", "restock": "a restock, re-release or new colorway of an existing product", "event": "an event, exhibition, pop-up or workshop", "store": "a store opening or a space", "culture": "culture, history or a story rather than a product", "styling": "a styling, lifestyle or mood shot", "promotion": "a promotion, sale or shop notice", "other": "none of the above"}},
 "worth": {"type": "noul", "instructions": "Overall, is there a concrete enough change here that CSW should explain it to readers once?"},
}
def state_of(e):
    m = META.get(e["shortcode"], {}); s = e["scores"]
    heat = f'{m.get("likes", 0)} likes; account median {m.get("acct_median", 0)}' + (f'; {m["likes"] / m["acct_median"]:.1f}x usual' if m.get("acct_median") else "")
    return {"account": e["account"], "caption": (e.get("caption") or "")[:1800], "image_description": s.get("look", ""), "tags": m.get("tags", []), "heat": heat}
def ask(e):
    body = json.dumps({"state": state_of(e), "model": "jev-1.13.0", "questions": {k: {**q, "instructions": FRAME + " " + q["instructions"]} for k, q in Q.items()}}).encode()
    for attempt in range(6):
        try:
            req = urllib.request.Request(URL, data=body, headers={"Authorization": "Bearer " + KEY, "Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=60) as r: return {"shortcode": e["shortcode"], **json.loads(r.read())}
        except urllib.error.HTTPError as ex:
            if ex.code in (429, 500, 502, 503, 504): time.sleep(min(8, 0.5 * 2 ** attempt) + random.random()); continue
            return {"shortcode": e["shortcode"], "error": f"http {ex.code}: {ex.read()[:300]}"}
        except Exception as ex: time.sleep(1 + attempt)
    return {"shortcode": e["shortcode"], "error": "retries exhausted"}
t0 = time.time()
with cf.ThreadPoolExecutor(max_workers=6) as ex: res = list(ex.map(ask, EV))
json.dump(res, open(sys.argv[3], "w"), ensure_ascii=False, indent=1)
ok = [r for r in res if "answers" in r]; tok = sum(r.get("usage", {}).get("input_tokens", 0) for r in ok)
print(f"events={len(res)} ok={len(ok)} wall={time.time()-t0:.1f}s tokens={tok} cost=${tok/1e6*0.042:.4f}")
for r in res:
    if "error" in r: print("ERR", r["error"][:200])
DEC = {"DdZwiM9EnWL": "Van 采用", "DdYgHUNE6ux": "Van 否决", "DdYmJ-ahq34": "Van 否决"}
V = lambda v: "成立" if v >= 7 else ("不明" if v >= 4 else "不成立")
print(f'{"事件":28} {"Jev 六维(成立概率)":40} {"Codex 判定":16} {"worth":6} {"kind":22} 决定')
for e, r in zip(EV, res):
    if "answers" not in r: continue
    a = r["answers"]; d = e["scores"]["dims"]
    jev = " ".join(f'{k[:2]}{a[k]["probabilities"].get("2", 0):.2f}' for k in ["change", "use", "gain", "compare", "explain", "csw"])
    cdx = "".join({"成立": "✓", "不明": "?", "不成立": "✗"}[V(d[k])] for k in ["change", "use", "gain", "compare", "explain", "csw"])
    print(f'{e["scores"]["title"][:26]:28} {jev:40} {cdx:16} {a["worth"]["noul"]:.2f}   {a["kind"]["choice"]:22} {DEC.get(e["shortcode"], "")}')
