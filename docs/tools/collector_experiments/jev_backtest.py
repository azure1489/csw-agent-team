# -*- coding: utf-8 -*-
"""阶段 0.7：Jev 三种用法的回测——初评阈值、同一事件、核对依据。

总方案把 Jev 的职责收窄成三件事：对全部候选做六维初评、判两条候选是不是同一件事、
核对生成模型给的依据是否真能支持它的结论。这个脚本分别用真实数据量一遍。

金标从哪来：
  初评   正样本 = csw 里「已生成过文章」的贴文（团队真的写过它）；
         负样本 = 9/17–9/18 窗口里未被写过的贴文。两者都是 Instagram 原文，同构可比。
  同一事件 图片哈希在跨账号转载里对不上（实测 0 组），所以按品牌别名共现分桶：
         同账号同品牌 / 跨账号同品牌 / 跨品牌随机。第三桶是干净的真负例，
         前两桶是「多半是同一件事」，报分布而不是报准确率。
  核对   干净的合成金标：把 0.4 实测里生成模型写的依据，配它自己的正文（正例）
         或配另一条贴文的正文（反例）。反例按构造就不可能被支持。

跑法：python3 jev_backtest.py --data <kbprobe 目录> [--limit N]
密钥从 ~/.typesafe/.env 读，不打印。
"""
import argparse, collections, glob, json, os, random, re, sys, time
import urllib.request, urllib.error
import concurrent.futures as cf

URL = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-latest"


def key():
    for ln in open(os.path.expanduser("~/.typesafe/.env"), encoding="utf-8"):
        m = re.match(r'\s*(?:export\s+)?TYPESAFE_API_KEY\s*=\s*["\']?([^"\'\s]+)', ln)
        if m:
            return m.group(1)
    sys.exit("~/.typesafe/.env 里没有 TYPESAFE_API_KEY")


KEY = key()


def ask(state, questions, tag=None, timeout=60):
    body = json.dumps({"state": state, "model": MODEL, "questions": questions}).encode()
    for attempt in range(6):
        try:
            req = urllib.request.Request(
                URL, data=body,
                headers={"Authorization": "Bearer " + KEY, "Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return {"tag": tag, **json.loads(r.read())}
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 504, 529):
                time.sleep(min(8, 0.5 * 2 ** attempt) + random.random())
                continue
            return {"tag": tag, "error": f"http {e.code}: {e.read()[:200].decode('utf-8','replace')}"}
        except Exception as e:
            time.sleep(1 + attempt)
            last = f"{type(e).__name__}: {e}"
    return {"tag": tag, "error": "重试用尽"}


def run(items, build, workers=8):
    t0 = time.time()
    with cf.ThreadPoolExecutor(max_workers=workers) as ex:
        res = list(ex.map(lambda it: ask(*build(it)), items))
    bad = [r for r in res if "error" in r]
    tok = sum(r.get("usage", {}).get("input_tokens", 0) for r in res if "usage" in r)
    print(f"    {len(res)} 次调用，{len(bad)} 失败，{time.time()-t0:.1f}s，输入 {tok} token")
    for b in bad[:3]:
        print(f"    失败样例：{b['error'][:140]}")
    return res


# ── 数据 ────────────────────────────────────────────────────────────────
def load(data):
    def items(pat):
        out = []
        for f in sorted(glob.glob(os.path.join(data, pat))):
            d = json.load(open(f, encoding="utf-8"))
            d = d.get("data", d)
            out += d.get("items") or []
        return out

    window, generated = items("w_*.json"), items("gen_*.json")
    brands = []
    for a in items("acc_*.json"):
        for k in ("accountName", "account"):
            if a.get(k):
                brands.append(a[k].strip())
    brands = sorted({b for b in brands if len(b) >= 4})
    gids = {p["postId"] for p in generated}
    window = [p for p in window if p["postId"] not in gids]
    return window, generated, brands


def text_of(p, n=1600):
    return ((p.get("description") or "") + "\n" + (p.get("translatedText") or "")).strip()[:n]


def heat_of(p):
    try:
        likes, fol = int(p.get("likes") or 0), int(p.get("followers") or 0)
    except (TypeError, ValueError):
        return "未知"
    return f"{likes} 赞" + (f"，账号 {fol} 粉丝" if fol else "")


def state_of(p):
    return {"account": p.get("account"), "caption": text_of(p),
            "content_type": p.get("contentType"), "photo_count": len(p.get("mediaList") or []),
            "heat": heat_of(p)}


# ── 问题定义 ─────────────────────────────────────────────────────────────
FRAME = ("You are screening one Instagram post for CSW, a Chinese-language magazine about outdoor, "
         "camping and urban-outdoor living. CSW's core question: does this contain a change concrete "
         "enough that CSW should explain it to readers once? A post that is not a new product can still "
         "qualify through a grounded design, culture, history or current-affairs angle. Brand fame is "
         "NOT a criterion. Judge only from the fields in state. You see the caption only — no images — "
         "so do not penalise a post for things that would be visible in photos.")

LV = lambda no, unclear, yes: [{"level": "不成立", "what": no},
                               {"level": "不明", "what": unclear},
                               {"level": "成立", "what": yes}]

TRIAGE_Q = {
    "change": {"type": "score",
               "instructions": "Does the caption state a concrete change — a new product, version, collaboration, material, structure, event or space — with specifics?",
               "criteria": LV("No concrete change: a reshown product, a greeting, a mood shot, a generic promotion.",
                              "Something may have changed but the caption does not say what specifically.",
                              "A specific, nameable change is stated.")},
    "use": {"type": "score",
            "instructions": "Does the caption connect to how a person would actually wear, carry, pitch, cook with or travel with the thing?",
            "criteria": LV("No usage relation; only appearance or an announcement.",
                           "Usage is implied but not described.",
                           "Usage, wearing, carrying or a concrete scenario is described.")},
    "gain": {"type": "score",
             "instructions": "Does the caption add information a reader could learn from — design intent, specifications, release date, price, channel, background?",
             "criteria": LV("Only a name, or nothing.", "One vague fact.",
                            "Several concrete facts an editor could cite.")},
    "compare": {"type": "score",
                "instructions": "Does the caption give, or obviously allow, a comparison reference: a previous generation, the brand's earlier practice, or a similar product?",
                "criteria": LV("No reference and none implied.", "A reference is hinted but not stated.",
                               "A comparison reference is stated or obviously available.")},
    "explain": {"type": "score",
                "instructions": "Is there an editorial point with factual support — something whose design choice can be tied to a use, a need or a piece of history? 'Looks good' or 'interesting' is not enough.",
                "criteria": LV("Nothing to explain.", "Possibly, but the caption gives too little.",
                               "A clear point that rewards explanation.")},
    "csw": {"type": "score",
            "instructions": "Does the subject sit inside CSW's territory — outdoor, camping, urban-outdoor living, with a design or culture angle, for readers in China?",
            "criteria": LV("Off-topic, or a purely local shop notice.",
                           "Related but marginal, or hard to relate to readers in China.",
                           "Squarely in CSW's territory.")},
    "worth": {"type": "noul",
              "instructions": "Overall: is there a change concrete enough here that CSW should explain it to readers once?",
              "criteria": {"true": "Yes — an editor would take this into a selection meeting.",
                           "false": "No — there is nothing here CSW would explain."}},
    "lower": {"type": "noul",
              "instructions": "Does this post fall into CSW's 'lower the priority' list: only a new colourway or a plain logo collaboration; a material swap with no stated difference; a claim of innovation with nothing verifiable; pure celebrity placement; only 'it looks good'?",
              "criteria": {"true": "Yes, it is one of those.", "false": "No, it is not."}},
}

PAIR_Q = {
    "same_event": {"type": "noul",
                   "instructions": "Do these two posts report the SAME underlying event — the same product launch, the same collaboration, the same store opening, the same exhibition? Two different posts by different accounts about one launch count as the same event. Two different products from one brand do not.",
                   "criteria": {"true": "Same underlying event.", "false": "Different events."}},
    "comparison": {"type": "choice",
                   "instructions": "post_b is an item CSW has already covered or already has on hand. Judging post_a against it, which is true?",
                   "criteria": {
                       "same_fact_no_gain": "post_a reports the same fact from the same angle and adds nothing.",
                       "same_brand_with_gain": "Same brand or product line, but post_a adds a fact or an angle the other does not have.",
                       "unrelated": "They are not about the same thing."}},
}

CHECK_Q = {
    "supported": {"type": "noul",
                  "instructions": "`statement` is a justification an editor wrote for the verdict on dimension `dimension`. `material` is the ONLY source material the editor was given. Does `material` actually support `statement`? Answer false if the statement quotes or asserts anything that is not in `material` — including a quoted sentence that does not appear there.",
                  "criteria": {"true": "Everything the statement asserts can be found in the material.",
                               "false": "The statement asserts something the material does not contain."}},
}


# ── 取值 ────────────────────────────────────────────────────────────────
def noul(r, k):
    a = (r.get("answers") or {}).get(k) or {}
    return a.get("noul")


def score3(r, k):
    """三档 score 取「成立」那一档的概率；概率键是字符串。"""
    a = (r.get("answers") or {}).get(k) or {}
    p = a.get("probabilities") or {}
    return p.get("2", p.get(2))


def pct(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return "—"
    q = lambda f: xs[min(len(xs) - 1, int(f * len(xs)))]
    return f"中位 {q(.5):.2f}  四分位 {q(.25):.2f}/{q(.75):.2f}"


def best_threshold(pos, neg):
    """在同一条 worth 概率轴上找一个阈值，报它的召回、精确与淘汰比例。"""
    vals = sorted({round(v, 2) for v in pos + neg})
    best = None
    for t in vals:
        tp = sum(1 for v in pos if v >= t)
        fp = sum(1 for v in neg if v >= t)
        recall = tp / max(1, len(pos))
        prec = tp / max(1, tp + fp)
        # 初评是分流不是淘汰：召回优先，宁可多留
        f = 0 if recall + prec == 0 else (1 + 4) * prec * recall / (4 * prec + recall)
        if best is None or f > best[0]:
            best = (f, t, recall, prec, 1 - (tp + fp) / (len(pos) + len(neg)))
    return best


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--limit", type=int, default=80, help="初评每组取多少条")
    ap.add_argument("--pairs", type=int, default=30, help="同一事件每桶取多少对")
    ap.add_argument("--checks", type=int, default=50, help="核对取多少条依据")
    ap.add_argument("--judgements", default=None, help="0.4 的判断输出 json")
    a = ap.parse_args()

    window, generated, brands = load(a.data)
    rng = random.Random(20260922)
    print(f"底料：窗口未写过 {len(window)} 条，已生成过文章 {len(generated)} 条，品牌别名 {len(brands)} 个\n")

    # ── 一 初评 ──────────────────────────────────────────────────────
    print("一 · 六维初评与阈值")
    pos = rng.sample([p for p in generated if text_of(p)], min(a.limit, len(generated)))
    neg = rng.sample([p for p in window if text_of(p)], min(a.limit, len(window)))
    res_pos = run(pos, lambda p: (state_of(p), with_frame(TRIAGE_Q), "pos"))
    res_neg = run(neg, lambda p: (state_of(p), with_frame(TRIAGE_Q), "neg"))
    print(f"\n    {'维度':<10}{'已写过（成立概率）':<32}{'未写过（成立概率）'}")
    for k in ["change", "use", "gain", "compare", "explain", "csw"]:
        print(f"    {k:<10}{pct([score3(r, k) for r in res_pos]):<32}{pct([score3(r, k) for r in res_neg])}")
    wp = [noul(r, "worth") for r in res_pos if noul(r, "worth") is not None]
    wn = [noul(r, "worth") for r in res_neg if noul(r, "worth") is not None]
    print(f"    {'worth':<10}{pct(wp):<32}{pct(wn)}")
    lp = [noul(r, "lower") for r in res_pos if noul(r, "lower") is not None]
    ln = [noul(r, "lower") for r in res_neg if noul(r, "lower") is not None]
    print(f"    {'lower':<10}{pct(lp):<32}{pct(ln)}")
    if wp and wn:
        f, t, rec, prec, cut = best_threshold(wp, wn)
        print(f"\n    worth ≥ {t:.2f}：已写过的留住 {rec:.0%}，留下来的里有 {prec:.0%} 是已写过的，"
              f"整体筛掉 {cut:.0%}")
        for t2 in (0.3, 0.5, 0.7):
            r2 = sum(1 for v in wp if v >= t2) / len(wp)
            c2 = 1 - (sum(1 for v in wp if v >= t2) + sum(1 for v in wn if v >= t2)) / (len(wp) + len(wn))
            print(f"    worth ≥ {t2:.2f}：已写过的留住 {r2:.0%}，整体筛掉 {c2:.0%}")

    # ── 二 同一事件与对照结论 ────────────────────────────────────────
    print("\n二 · 同一事件与对照结论")
    buckets = make_pairs(window, brands, rng, a.pairs)
    for name, pairs in buckets.items():
        if not pairs:
            print(f"  {name}：没造出样本")
            continue
        res = run(pairs, lambda pr: ({"post_a": state_of(pr[0]), "post_b": state_of(pr[1])},
                                     with_frame(PAIR_Q), name))
        same = [noul(r, "same_event") for r in res if noul(r, "same_event") is not None]
        choice = collections.Counter(
            ((r.get("answers") or {}).get("comparison") or {}).get("choice") for r in res)
        print(f"  {name}（{len(pairs)} 对）  same_event {pct(same)}   对照结论 {dict(choice)}")

    # ── 三 核对依据 ──────────────────────────────────────────────────
    if a.judgements and os.path.exists(a.judgements):
        print("\n三 · 核对依据（抓编造）")
        pairs = make_checks(a.judgements, window + generated, rng, a.checks)
        res = run(pairs, lambda c: ({"dimension": c["dim"], "material": c["material"],
                                     "statement": c["statement"]}, with_frame(CHECK_Q), c["label"]))
        good = [noul(r, "supported") for r, c in zip(res, pairs) if c["label"] == "真依据"]
        fake = [noul(r, "supported") for r, c in zip(res, pairs) if c["label"] == "张冠李戴"]
        good = [v for v in good if v is not None]
        fake = [v for v in fake if v is not None]
        print(f"    真依据配本条正文（{len(good)} 条）：supported {pct(good)}")
        print(f"    真依据配别条正文（{len(fake)} 条）：supported {pct(fake)}")
        for t in (0.3, 0.5, 0.7):
            keep = sum(1 for v in good if v >= t) / max(1, len(good))
            caught = sum(1 for v in fake if v < t) / max(1, len(fake))
            print(f"    阈值 {t:.1f}：真依据放行 {keep:.0%}，张冠李戴抓出 {caught:.0%}")


def with_frame(qs):
    return {k: {**q, "instructions": FRAME + " " + q["instructions"]} for k, q in qs.items()}


def brands_in(text, brands):
    low = text.lower()
    return {b for b in brands if b.lower() in low}


def make_pairs(window, brands, rng, n):
    """三桶：同账号同品牌、跨账号同品牌、跨品牌随机。第三桶是干净的真负例。"""
    tagged = [(p, brands_in(text_of(p) + " " + (p.get("account") or ""), brands)) for p in window]
    tagged = [(p, bs) for p, bs in tagged if bs and text_of(p)]
    by_brand = collections.defaultdict(list)
    for p, bs in tagged:
        for b in bs:
            by_brand[b].append(p)

    same_acct, cross_acct = [], []
    for b, ps in by_brand.items():
        for i in range(len(ps)):
            for j in range(i + 1, len(ps)):
                (same_acct if ps[i].get("account") == ps[j].get("account") else cross_acct).append((ps[i], ps[j]))
    rng.shuffle(same_acct); rng.shuffle(cross_acct)

    unrelated = []
    pool = [p for p, _ in tagged]
    while len(unrelated) < n and len(pool) > 1:
        x, y = rng.sample(pool, 2)
        if not (brands_in(text_of(x), brands) & brands_in(text_of(y), brands)):
            unrelated.append((x, y))
    return {"同账号同品牌": same_acct[:n], "跨账号同品牌": cross_acct[:n], "跨品牌随机": unrelated[:n]}


def make_checks(path, posts, rng, n):
    """正例 = 依据配它自己的正文；反例 = 同一条依据配另一条贴文的正文。
    只取引用了正文原话（带「」）的依据——纯看图的依据靠正文本来就核不了，
    真实运行时那部分材料是识别输出。"""
    by_id = {p["postId"]: p for p in posts}
    rows = json.load(open(path, encoding="utf-8"))
    cand = []
    for r in rows:
        for o in (r.get("obj") or {}).get("judgements") or []:
            post = by_id.get(str(o.get("candidate_key")))
            if not post:
                continue
            for dim, d in (o.get("dims") or {}).items():
                b = (d or {}).get("basis") or ""
                if "「" in b and len(b) > 20:
                    cand.append({"dim": dim, "statement": b, "post": post})
    rng.shuffle(cand)
    cand = cand[:n]
    out = []
    for c in cand:
        out.append({"dim": c["dim"], "statement": c["statement"],
                    "material": text_of(c["post"]), "label": "真依据"})
        other = rng.choice([p for p in posts if p["postId"] != c["post"]["postId"]])
        out.append({"dim": c["dim"], "statement": c["statement"],
                    "material": text_of(other), "label": "张冠李戴"})
    return out


if __name__ == "__main__":
    main()
