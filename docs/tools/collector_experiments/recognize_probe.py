# -*- coding: utf-8 -*-
"""阶段 0.5 附带：图片识别这一步的吞吐实测。

总方案 §5.5 要求每张图连同该条正文送视觉模型，出结构化描述；§4.3 的时间表估「识别新图
约 5–7 分钟（10 路并行、每张约 4 秒）」。但 0.4 已经测到这个网关的吞吐上限大约是
8 并发、每请求几十到几百秒——识别走的是同一个网关，所以那个估计得重新量。

一次请求 = 一条贴文的全部图片（≤6 张）+ 正文，输出每张图一条结构化描述。

跑法（agent 主机）：
    python3 -u recognize_probe.py --data /tmp/kbprobe --n 16 --concurrency 8 --model gpt-6-astra
"""
import argparse, glob, json, os, random, sys, time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from responses_probe import load_env, http_json, fetch_image_b64, extract  # noqa: E402

IMAGE_KINDS = ["产品图", "细节图", "使用场景", "海报或文字图", "人物穿搭", "截图", "无关"]


def schema(n):
    """每张图一条描述。strict 模式下数组长度靠提示词约束，返回后再核对条数。"""
    one = {
        "type": "object", "additionalProperties": False,
        "required": ["index", "matches_text", "content", "missing_from_text", "kind", "usable_as_figure"],
        "properties": {
            "index": {"type": "integer", "description": "第几张图，从 1 起"},
            "matches_text": {"type": "string", "description": "对应正文哪一点"},
            "content": {"type": "string", "description": "画面里的产品、场景、文字"},
            "missing_from_text": {"type": "string", "description": "正文提到但画面没有的"},
            "kind": {"type": "string", "enum": IMAGE_KINDS},
            "usable_as_figure": {"type": "boolean", "description": "可否作配图"},
        },
    }
    return {"type": "object", "additionalProperties": False, "required": ["images"],
            "properties": {"images": {"type": "array", "items": one}}}


PROMPT = """你在为户外生活媒体做素材识别。下面是一条 Instagram 贴文的正文和它的全部图片。

逐张判断，按给定顺序在 images 数组里各输出一条，index 从 1 开始，不要合并、不要漏、不要多。
每张图要写清：
- matches_text：这张图对应正文的哪一点。正文里找不到对应就写「正文未提及」。
- content：画面里实际有什么——产品、场景、可读到的文字。只写看得见的，不要推测。
- missing_from_text：正文说了但这张图里看不到的东西。没有就写「无」。
- kind：图片类型。
- usable_as_figure：这张图能不能直接当文章配图。

看不清就说看不清，不要编。
"""


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


def recognize(env, post, model, timeout):
    t_img = time.time()
    photos, nbytes = [], 0
    for m in post["_photos"]:
        try:
            b64, nb = fetch_image_b64(m["mediaUrl"])
            photos.append(b64); nbytes += nb
        except Exception:
            pass
    img_ms = int((time.time() - t_img) * 1000)
    if not photos:
        return dict(ok=False, post=post.get("postId"), error="没下到图", ms=0,
                    img_ms=img_ms, photos=0, attempt=0)

    content = [{"type": "input_text",
                "text": f"{PROMPT}\n【正文】\n{post.get('description') or '（无正文）'}\n\n"
                        f"下面是这条贴文的 {len(photos)} 张图，按顺序编号。"}]
    for b64 in photos:
        content.append({"type": "input_image", "image_url": f"data:image/jpeg;base64,{b64}"})

    payload = {"model": model, "input": [{"role": "user", "content": content}],
               "text": {"format": {"type": "json_schema", "name": "media_descriptions",
                                   "strict": True, "schema": schema(len(photos))}},
               "max_output_tokens": 400 * len(photos) + 500, "store": False}
    headers = {"Authorization": f"Bearer {env['SUB2API_API_KEY']}", "Content-Type": "application/json"}
    url = env["SUB2API_BASE_URL"].rstrip("/") + "/responses"

    t0, attempt, last = time.time(), 0, None
    while attempt < 4:
        attempt += 1
        try:
            _, resp = http_json(url, payload, headers, timeout)
            return dict(ok=True, post=post.get("postId"), ms=int((time.time() - t0) * 1000),
                        img_ms=img_ms, photos=len(photos), img_bytes=nbytes,
                        attempt=attempt, resp=resp)
        except Exception as e:
            code = getattr(e, "code", None)
            last = f"{type(e).__name__} {code or ''}"
            if code in (429, 500, 502, 503, 504) or code is None:
                time.sleep(min(2 ** attempt, 16) + random.random())
                continue
            break
    return dict(ok=False, post=post.get("postId"), ms=int((time.time() - t0) * 1000),
                img_ms=img_ms, photos=len(photos), attempt=attempt, error=last)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="/tmp/kbprobe")
    ap.add_argument("--n", type=int, default=16)
    ap.add_argument("--images", type=int, default=6)
    ap.add_argument("--concurrency", type=int, default=8)
    ap.add_argument("--model", default="gpt-6-astra")
    ap.add_argument("--timeout", type=int, default=420)
    a = ap.parse_args()

    env = load_env()
    posts = load_posts(a.data, a.n, a.images)
    npix = sum(len(p["_photos"]) for p in posts)
    print(f"模型 {a.model}  {len(posts)} 条贴文 / {npix} 张图  并发 {a.concurrency}\n")

    t0 = time.time()
    with ThreadPoolExecutor(max_workers=a.concurrency) as ex:
        res = list(ex.map(lambda p: recognize(env, p, a.model, a.timeout), posts))
    wall = time.time() - t0

    ok = [r for r in res if r["ok"]]
    print(f"{'候选':<22}{'耗时':>9}{'图':>4}{'重试':>5}{'入':>8}{'出':>7}  条数核对")
    tot_in = tot_out = ok_imgs = 0
    for r in res:
        if not r["ok"]:
            print(f"{r['post']:<22}{r['ms']:>8}ms{r['photos']:>4}{r['attempt']:>5}  失败 {r.get('error')}")
            continue
        txt, usage = extract(r["resp"])
        try:
            imgs = json.loads(txt)["images"]
            note = "合格" if len(imgs) == r["photos"] else f"条数不符：要 {r['photos']} 得 {len(imgs)}"
            ok_imgs += len(imgs)
        except Exception as e:
            note = f"解析失败 {type(e).__name__}"
        ti, to = usage.get("input_tokens", 0), usage.get("output_tokens", 0)
        tot_in += ti; tot_out += to
        print(f"{r['post']:<22}{r['ms']:>8}ms{r['photos']:>4}{r['attempt']:>5}{ti:>8}{to:>7}  {note}")

    if ok:
        lat = sorted(x["ms"] for x in ok)
        print(f"\n完成 {len(ok)}/{len(res)}，墙钟 {wall:.1f}s")
        print(f"耗时 中位 {lat[len(lat)//2]}ms  最快 {lat[0]}ms  最慢 {lat[-1]}ms")
        print(f"token 入 {tot_in}（均 {tot_in//len(ok)}）  出 {tot_out}（均 {tot_out//len(ok)}）")
        print(f"每张图 {wall/max(1,npix):.2f}s；描述产出 {ok_imgs}/{npix} 张")
        print(f"按一期 358 条 / 1871 张图外推（同并发）："
              f"{wall/len(res)*358/60:.1f} 分钟，"
              f"入 {tot_in//len(ok)*358/1000:.0f}k token，出 {tot_out//len(ok)*358/1000:.0f}k token")
        print(f"退避触发 {sum(1 for x in ok if x['attempt'] > 1)} 次")


if __name__ == "__main__":
    main()
