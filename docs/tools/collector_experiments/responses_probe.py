# -*- coding: utf-8 -*-
"""阶段 0.4：直连 Responses 接口做逐条判断的实测。

为什么不走 codex app-server：turn/start 是异步的、同一线程不能并发带不同 outputSchema 的
回合、每回合底座还要 11k–18.7k token。批量判断要的是「几十条并行、每条一个严格 schema」，
直连接口才对路；app-server 留给深核。

这个脚本要回答四件事：
  1. 正文 + 多张 base64 图 + 严格 json_schema 能不能一次拿到合 schema 的结果
  2. 单条耗时与 token 用量（→ 每天 300 条的时间与成本底数）
  3. 并发 8 会不会 429，退避怎么设
  4. 图片给多少张、给多大，才在质量与成本之间划得来

跑法（在 agent 主机上）：
    python3 -u responses_probe.py --data /tmp/kbprobe --n 8 --images 6 --concurrency 8

密钥只从 /root/.hermes/profiles/csw-agent/.env 读，不打印、不落盘。
"""
import argparse, base64, glob, json, os, random, ssl, sys, threading, time, urllib.request
from concurrent.futures import ThreadPoolExecutor

ENV_FILE = "/root/.hermes/profiles/csw-agent/.env"


def load_env(path=ENV_FILE):
    """只取这四个键；其余一律不读，避免把无关密钥带进进程。"""
    want = {"SUB2API_API_KEY", "SUB2API_BASE_URL", "CSW_API_KEY", "CSW_API_URL"}
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            k = k.strip()
            if k in want:
                out[k] = v.strip().strip('"').strip("'")
    missing = want - out.keys()
    assert not missing, f"{path} 缺少 {missing}"
    return out


# ── 判断输出的严格 schema（总方案 §7.4）──────────────────────────────────
# strict 模式要求每一层都写全 required 并关掉 additionalProperties，
# 所以 §7.4 里的 `{}` 占位必须展开。
def dim():
    return {
        "type": "object", "additionalProperties": False,
        "required": ["verdict", "basis"],
        "properties": {
            "verdict": {"type": "string", "enum": ["yes", "no", "unclear"]},
            "basis": {"type": "string", "description": "依据，引正文原话或指明第几张图"},
        },
    }


def strs(desc):
    return {"type": "array", "items": {"type": "string"}, "description": desc}


JUDGEMENT_SCHEMA = {
    "type": "object", "additionalProperties": False,
    "required": ["candidate_key", "tier", "dims", "three_sentences", "unanswered",
                 "comparison", "heat_note", "look", "image_seen", "gaps",
                 "priority_hits", "lower_hits", "jev_disagreement"],
    "properties": {
        "candidate_key": {"type": "string"},
        "tier": {"type": "string", "enum": ["recommend", "alternate", "not_recommend", "pending_check"]},
        "dims": {
            "type": "object", "additionalProperties": False,
            "required": ["change", "use", "gain", "compare", "explain", "csw"],
            "properties": {k: dim() for k in ["change", "use", "gain", "compare", "explain", "csw"]},
        },
        "three_sentences": {
            "type": "object", "additionalProperties": False,
            "required": ["what_changed", "why_it_matters", "how_different"],
            "properties": {k: {"type": "string"} for k in
                           ["what_changed", "why_it_matters", "how_different"]},
        },
        "unanswered": {"type": "string",
                       "enum": ["none", "missing_material", "angle_not_formed", "low_value"]},
        "comparison": {
            "type": "object", "additionalProperties": False,
            "required": ["verdict", "against", "note"],
            "properties": {
                "verdict": {"type": "string",
                            "enum": ["same_fact_no_gain", "same_brand_with_gain", "unrelated"]},
                "against": {"type": "string"},
                "note": {"type": "string"},
            },
        },
        "heat_note": {"type": "string"},
        "look": {"type": "string", "description": "实图里看到的外观与设计要点"},
        "image_seen": {"type": "boolean", "description": "是否真的读到了实图；否则必须落 pending_check"},
        "gaps": strs("缺什么材料"),
        "priority_hits": strs("命中的七类优先关注"),
        "lower_hits": strs("命中的七类降低优先级"),
        "jev_disagreement": {"type": "string", "description": "与初评不一致时写明，否则空串"},
    },
}

BATCH_SCHEMA = {
    "type": "object", "additionalProperties": False,
    "required": ["judgements"],
    "properties": {"judgements": {"type": "array", "items": JUDGEMENT_SCHEMA}},
}

RUBRIC = """你是户外生活媒体「营事编集室」的情报收集员，按主编 Van 的框架逐条判断一条 Instagram 贴文是否值得写。

核心问题：这件事有没有足够具体的变化，值得 CSW 帮读者解释一次。

六个维度，各判 成立 / 不成立 / 不明，每个都要给依据（引正文原话，或指明第几张图）：
- change 变化必须具体：结构或使用方式改了什么、品牌首次做了什么、原产品为什么改。「某品牌发布新品」通常不够。
- use 与真实使用有关：人怎么穿、搭、住、跑、移动；产品为什么这样设计。纯配色、普通 logo 联名、无新信息的复刻通常优先级低。
- gain 读者能理解的价值：新的使用思路、值得了解的设计逻辑、文化背景、户外生活变化的解释。
- compare 比较参照：与上一代、品牌以往做法或同类产品有什么不同、为什么现在出现。参照要准确，不虚构旧款问题。
- explain 有事实支持的编辑判断：能解释某设计影响什么使用动作。只有「好看」「有趣」不够。
- csw 调性适配：是否符合 CSW 的报道角度与读者。

七类优先关注：老产品结构性改款；装备解决具体使用问题；品牌新的空间、社群或服务方式；装备进入新生活场景且有超出造型挪用的信息；小众品类值得解释的新设计；有真实争议或取舍的产品；能从产品解释有证据的消费或生活方式变化。
七类降低优先级：只有新配色或普通 logo 联名；换材质说不出差异；宣称创新无可核实变化；纯明星带货；缺可靠来源先补证；只能说好看；必须强行拔高成趋势。
两者都是倾向，不是黑名单。品牌知名度不是标准，不作维度。

三句话必须答得出：发生了什么变化？为什么值得户外用户知道？它和以前有什么不一样？答不清就在 unanswered 里写明是缺资料、角度未成立还是价值不足。

硬要求：
- 不打分、不加权。结论档只有 recommend / alternate / not_recommend / pending_check。
- 没真正读到实图就把 image_seen 设为 false 并落 pending_check，不因此淘汰。
- 依据必须来自给你的材料，不许编。编不出来就把该维度判 unclear。
"""


def http_json(url, payload, headers, timeout=180):
    body = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    ctx = ssl.create_default_context()
    with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
        return r.status, json.loads(r.read())


def fetch_image_b64(url, width=768, timeout=60):
    """走 OSS 的 image/resize 直接要小图，不在本地解码——少一份 Pillow 依赖，也省带宽。"""
    sep = "&" if "?" in url else "?"
    sized = f"{url}{sep}x-oss-process=image/resize,w_{width}/format,jpg/quality,q_80"
    with urllib.request.urlopen(sized, timeout=timeout) as r:
        raw = r.read()
    return base64.b64encode(raw).decode(), len(raw)


def load_posts(data_dir, n, images):
    posts = []
    for f in sorted(glob.glob(os.path.join(data_dir, "w_*.json"))):
        d = json.load(open(f, encoding="utf-8"))
        d = d.get("data", d)
        for p in d.get("items") or []:
            ml = [m for m in (p.get("mediaList") or []) if m.get("mediaType") == "Photo"]
            if p.get("contentType") in ("Image", "Carousel") and ml and len(ml) == len(p.get("mediaList") or []):
                p["_photos"] = ml[:images]
                posts.append(p)
    random.Random(20260922).shuffle(posts)
    return posts[:n]


def build_batch_input(items):
    """一次请求判多条：准则与 schema 只付一次钱。
    每条之间用编号分段，图片紧跟在对应那条后面，避免模型串条。"""
    content = [{"type": "input_text", "text": RUBRIC + f"\n\n下面给你 {len(items)} 条候选，"
                f"逐条独立判断，按给定顺序在 judgements 数组里各输出一条，不要合并、不要漏。\n"}]
    for idx, (post, photos) in enumerate(items, 1):
        meta = {
            "candidate_key": post.get("postId"),
            "account": post.get("account"),
            "date_posted": post.get("datePosted"),
            "likes": post.get("likes"),
            "comments": post.get("numComments"),
            "followers": post.get("followers"),
            "photo_count": len(post.get("mediaList") or []),
            "photos_given": len(photos),
        }
        content.append({"type": "input_text", "text":
            f"\n===== 候选 {idx} / {len(items)} =====\n"
            f"【元信息】{json.dumps(meta, ensure_ascii=False)}\n"
            f"【正文】{post.get('description') or '（无正文）'}\n"
            f"【译文】{post.get('translatedText') or '（无译文）'}\n"
            f"【对照材料】（本次实测不给；comparison.verdict 按 unrelated，"
            f"并在 gaps 里写明「未给对照材料」）\n"
            f"下面 {len(photos)} 张图属于候选 {idx}："})
        for b64 in photos:
            content.append({"type": "input_image", "image_url": f"data:image/jpeg;base64,{b64}"})
    return [{"role": "user", "content": content}]


def build_input(post, photos_b64):
    meta = {
        "candidate_key": post.get("postId"),
        "account": post.get("account"),
        "date_posted": post.get("datePosted"),
        "likes": post.get("likes"),
        "comments": post.get("numComments"),
        "followers": post.get("followers"),
        "url": post.get("url"),
        "photo_count": len(post.get("mediaList") or []),
        "photos_given": len(photos_b64),
    }
    text = (
        f"{RUBRIC}\n\n"
        f"【候选元信息】\n{json.dumps(meta, ensure_ascii=False, indent=2)}\n\n"
        f"【正文】\n{post.get('description') or '（无正文）'}\n\n"
        f"【译文】\n{post.get('translatedText') or '（无译文）'}\n\n"
        f"【对照材料】\n（本次实测不给对照材料；comparison.verdict 按 unrelated 处理，"
        f"并在 gaps 里写明「未给对照材料」。真实运行时五类缺一不判。）\n\n"
        f"下面是这条贴文的 {len(photos_b64)} 张实图，按顺序编号。请据此填写 look 与各维度依据。"
    )
    content = [{"type": "input_text", "text": text}]
    for b64 in photos_b64:
        content.append({"type": "input_image", "image_url": f"data:image/jpeg;base64,{b64}"})
    return [{"role": "user", "content": content}]


def judge_one(env, batch, model, images, timeout):
    """batch 是一组候选；长度 1 时就是逐条模式。"""
    t_img = time.time()
    prepared, nbytes = [], 0
    for post in batch:
        photos = []
        for m in post["_photos"]:
            try:
                b64, n = fetch_image_b64(m["mediaUrl"])
                photos.append(b64); nbytes += n
            except Exception as e:
                print(f"  图片下载失败 {post.get('postId')}: {type(e).__name__}", flush=True)
        prepared.append((post, photos))
    img_ms = int((time.time() - t_img) * 1000)
    photos_n = sum(len(p) for _, p in prepared)
    key = ",".join(str(p.get("postId"))[-6:] for p in batch)

    if len(batch) == 1:
        post, photos = prepared[0]
        payload_input = build_input(post, photos)
        schema, name = JUDGEMENT_SCHEMA, "intake_judgement"
    else:
        payload_input = build_batch_input(prepared)
        schema, name = BATCH_SCHEMA, "intake_judgements"

    payload = {
        "model": model,
        "input": payload_input,
        "text": {"format": {"type": "json_schema", "name": name,
                            "strict": True, "schema": schema}},
        "max_output_tokens": 3000 * len(batch),
        "store": False,
    }
    headers = {"Authorization": f"Bearer {env['SUB2API_API_KEY']}",
               "Content-Type": "application/json"}
    url = env["SUB2API_BASE_URL"].rstrip("/") + "/responses"

    t0 = time.time()
    attempt, last = 0, None
    while attempt < 4:
        attempt += 1
        try:
            status, resp = http_json(url, payload, headers, timeout)
            ms = int((time.time() - t0) * 1000)
            return dict(ok=True, post=key, size=len(batch), ms=ms, img_ms=img_ms,
                        img_bytes=nbytes, photos=photos_n, attempt=attempt, resp=resp)
        except urllib.error.HTTPError as e:
            body = e.read()[:400].decode("utf-8", "replace")
            last = f"HTTP {e.code} {body}"
            if e.code in (429, 500, 502, 503, 504):
                back = min(2 ** attempt, 16) + random.random()
                print(f"  {key} {e.code}，退避 {back:.1f}s（第 {attempt} 次）", flush=True)
                time.sleep(back)
                continue
            break
        except Exception as e:
            last = f"{type(e).__name__}: {e}"
            time.sleep(2 ** attempt)
    return dict(ok=False, post=key, size=len(batch), ms=int((time.time() - t0) * 1000),
                img_ms=img_ms, img_bytes=nbytes, photos=photos_n,
                attempt=attempt, error=last)


def extract(resp):
    """Responses 接口的输出在 output[].content[].text；顺带把 usage 取出来。"""
    txt = None
    for item in resp.get("output") or []:
        for c in item.get("content") or []:
            if c.get("type") in ("output_text", "text") and c.get("text"):
                txt = c["text"]
    if txt is None:
        txt = resp.get("output_text")
    usage = resp.get("usage") or {}
    return txt, usage


def validate(obj):
    """只查 strict schema 该保证的那几件事，出错就说明 strict 没生效。"""
    problems = []
    for k in JUDGEMENT_SCHEMA["required"]:
        if k not in obj:
            problems.append(f"缺 {k}")
    if obj.get("tier") not in JUDGEMENT_SCHEMA["properties"]["tier"]["enum"]:
        problems.append(f"tier 非法 {obj.get('tier')!r}")
    for k in ["change", "use", "gain", "compare", "explain", "csw"]:
        d = (obj.get("dims") or {}).get(k) or {}
        if d.get("verdict") not in ("yes", "no", "unclear"):
            problems.append(f"dims.{k}.verdict 非法 {d.get('verdict')!r}")
        if not (d.get("basis") or "").strip():
            problems.append(f"dims.{k}.basis 为空")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="/tmp/kbprobe")
    ap.add_argument("--n", type=int, default=8)
    ap.add_argument("--images", type=int, default=6)
    ap.add_argument("--concurrency", type=int, default=8)
    ap.add_argument("--batch", type=int, default=1, help="一次请求判几条候选")
    ap.add_argument("--model", default="gpt-6-astra")
    ap.add_argument("--timeout", type=int, default=240)
    ap.add_argument("--out", default="/tmp/responses_probe_out.json")
    a = ap.parse_args()

    env = load_env()
    print(f"网关 {env['SUB2API_BASE_URL']}  模型 {a.model}  "
          f"密钥长度 {len(env['SUB2API_API_KEY'])}（不回显）")
    posts = load_posts(a.data, a.n, a.images)
    batches = [posts[i:i + a.batch] for i in range(0, len(posts), a.batch)]
    print(f"取到 {len(posts)} 条候选，每条最多 {a.images} 张图，"
          f"每请求 {a.batch} 条 → {len(batches)} 个请求，并发 {a.concurrency}\n")

    t0 = time.time()
    with ThreadPoolExecutor(max_workers=a.concurrency) as ex:
        results = list(ex.map(lambda b: judge_one(env, b, a.model, a.images, a.timeout), batches))
    wall = time.time() - t0

    ok = [r for r in results if r["ok"]]
    print(f"\n完成 {len(ok)}/{len(results)}，墙钟 {wall:.1f}s\n")
    print(f"{'候选':<14}{'状态':<6}{'耗时':>8}{'取图':>8}{'图':>4}{'重试':>5}"
          f"{'入':>9}{'出':>8}  {'档':<14} schema")
    rows, tot_in, tot_out = [], 0, 0
    for r in results:
        if not r["ok"]:
            print(f"{r['post']:<14}{'失败':<6}{r['ms']:>7}ms{r['img_ms']:>7}ms"
                  f"{r['photos']:>4}{r['attempt']:>5}  {r.get('error','')[:60]}")
            continue
        txt, usage = extract(r["resp"])
        try:
            obj = json.loads(txt)
            objs = obj["judgements"] if r.get("size", 1) > 1 else [obj]
            probs = [p for o in objs for p in validate(o)]
            if len(objs) != r.get("size", 1):
                probs.append(f"条数不符：要 {r.get('size')} 得 {len(objs)}")
            tier = "/".join(str(o.get("tier", "?"))[:4] for o in objs)
            verdict = "合格" if not probs else "；".join(probs)[:40]
        except Exception as e:
            obj, tier, verdict = None, "-", f"不是 JSON：{type(e).__name__}"
        ti, to = usage.get("input_tokens", 0), usage.get("output_tokens", 0)
        tot_in += ti; tot_out += to
        print(f"{r['post']:<14}{'成功':<6}{r['ms']:>7}ms{r['img_ms']:>7}ms"
              f"{r['photos']:>4}{r['attempt']:>5}{ti:>9}{to:>8}  {str(tier):<14}{verdict}")
        rows.append(dict(post=r["post"], size=r.get("size", 1), ms=r["ms"],
                         img_ms=r["img_ms"], photos=r["photos"],
                         img_bytes=r["img_bytes"], attempt=r["attempt"],
                         input_tokens=ti, output_tokens=to, obj=obj))

    if ok:
        per_cand = sum(x.get("size", 1) for x in rows)
        lat = sorted(x["ms"] for x in rows)
        print(f"\n耗时 中位 {lat[len(lat)//2]}ms  最快 {lat[0]}ms  最慢 {lat[-1]}ms")
        print(f"token 入合计 {tot_in}（均 {tot_in//len(rows)}）  出合计 {tot_out}（均 {tot_out//len(rows)}）")
        pics = sum(x["photos"] for x in rows)
        mb = sum(x["img_bytes"] for x in rows) / 1e6
        print(f"图片 {pics} 张 {mb:.1f} MB，取图中位 {sorted(x['img_ms'] for x in rows)[len(rows)//2]}ms")
        print(f"每条候选：入 {tot_in//max(1,per_cand)} token，出 {tot_out//max(1,per_cand)} token")
        print(f"按每天 300 条外推（同样的批量与并发）：入 {tot_in//max(1,per_cand)*300/1000:.0f}k，"
              f"出 {tot_out//max(1,per_cand)*300/1000:.0f}k token；"
              f"墙钟约 {wall/max(1,per_cand)*300/60:.1f} 分钟")
        retried = [x for x in rows if x["attempt"] > 1]
        print(f"429/5xx 退避触发 {len(retried)} 次")
    json.dump(rows, open(a.out, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
    print(f"\n完整结果写到 {a.out}")


if __name__ == "__main__":
    main()
