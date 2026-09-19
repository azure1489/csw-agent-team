import json, subprocess, sys, os, time, select, threading, queue, re
D=os.path.dirname(os.path.abspath(__file__))
posts={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{D}/posts.jsonl") if l.strip()}
media={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{D}/media.jsonl") if l.strip()}
ans={json.loads(l)["shortcode"]:json.loads(l)["answers"] for l in open(f"{D}/answers.jsonl")}
def find(acc, pat):
    for k,p in posts.items():
        if p["account"]==acc and re.search(pat,p["description"] or "") and (media.get(k,{}).get("images")): return k
ugc=max((k for k in ans if media.get(k,{}).get("images")), key=lambda k:ans[k]["styling_or_ugc"]["noul"])
promo=max((k for k in ans if media.get(k,{}).get("images")), key=lambda k:ans[k]["promotion_admin"]["noul"])
SAMPLE=[("Van 亲选","DdZwiM9EnWL"),("Van 否决","DdYgHUNE6ux"),("Van 否决","DdYmJ-ahq34"),("与 Van 亲选同一事件","DdVqdOJmDr5"),
        ("未审·联名",find("robin_outdoor_base","Brompton")),("未审·新品",find("zanearts_outdoor","ゼクールー")),("穿搭类",ugc),("促销类",promo)]
SAMPLE=[(t,k) for t,k in SAMPLE if k and k in posts]
DIMS=["novelty","distinctiveness","reader_relevance","info_completeness","brand_weight","image_material","visual_appeal"]
SCHEMA={"type":"object","additionalProperties":False,"required":DIMS+["image_notes","one_line"],
        "properties":{**{d:{"type":"integer","minimum":0,"maximum":4} for d in DIMS},"image_notes":{"type":"string"},"one_line":{"type":"string"}}}
RUBRIC="""你在给一条 Instagram 贴文按七个维度打分，每个维度 0–4 的整数。读者是中文户外、露营、城市户外生活方式杂志的读者，人在国内。只依据给你的文案和图片，不要上网，不要调用任何工具。
novelty 新品性：0 没有产品或新闻；1 旧品再次露出；2 补货、再贩、既有产品的新色新尺寸；3 新发布或即将发布的产品、型号、联名；4 前所未有的首发。
distinctiveness 独特性：0 无可识别对象；1 寻常货品；2 有一个值得一提的特点；3 设计、技术、工艺或故事上有辨识度；4 发烧友会谈论的特别物件，包含艺术、车辆、建筑等跨界之作。
reader_relevance 读者相关：0 纯本地信息；1 以海外门店或活动为主；2 有意思但地域性强、难买到；3 不论身在何处都值得看；4 国际上受关注，或国内读者可直接关注、购买。
info_completeness 信息量：0 无；1 只有名字；2 名字加一条事实；3 名字加日期、价格、材料、规格中的几项；4 规格、价格、日期、购买渠道、设计背景俱全。
brand_weight 品牌分量：0 无法识别；1 少有人知的小店；2 发烧友熟悉的小众品牌；3 圈内知名品牌；4 大牌，或跨时尚、艺术、汽车、设计的重磅联名。
image_material 图片素材（只看图）：0 没有可用图，只有海报或文字图；1 只有一张可用；2 有几张但雷同或质量一般；3 多张清晰的产品图；4 成套素材，含细节、使用场景、多角度，可直接用于排版。
visual_appeal 视觉吸引力（只看图）：0 无吸引力；1 平淡；2 尚可；3 好看，有风格；4 惊艳，一眼想点开。
image_notes：用不超过 40 个汉字写图片里实际有什么。one_line：用不超过 40 个汉字写这条最值得报或最不值得报的一点。"""
p=subprocess.Popen(["codex","app-server"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,bufsize=1)
lock=threading.Lock(); pending={}; turns={}
def send(o):
    with lock: p.stdin.write(json.dumps(o,ensure_ascii=False)+"\n"); p.stdin.flush()
def reader():
    for line in p.stdout:
        try: m=json.loads(line)
        except Exception: continue
        if "id" in m and "method" in m:      # 服务端请求：一律拒绝
            send({"id":m["id"],"result":{"decision":"decline"}}); continue
        if "id" in m and m["id"] in pending: pending[m["id"]].put(m); continue
        meth=m.get("method"); prm=m.get("params",{})
        tid=prm.get("threadId")
        if tid in turns:
            if meth=="item/completed" and prm["item"].get("type")=="agentMessage": turns[tid]["final"]=prm["item"].get("text")
            if meth=="thread/tokenUsage/updated": turns[tid]["usage"]=prm["tokenUsage"]["total"]
            if meth=="turn/completed": turns[tid]["status"]=prm["turn"].get("status"); turns[tid]["err"]=prm["turn"].get("error"); turns[tid]["done"].set()
threading.Thread(target=reader,daemon=True).start()
_id=[0]
def call(method,params,timeout=120):
    _id[0]+=1; i=_id[0]; pending[i]=queue.Queue(); send({"id":i,"method":method,"params":params})
    return pending[i].get(timeout=timeout)
call("initialize",{"clientInfo":{"name":"csw-collector-smoke","title":"smoke","version":"0.0.2"},"capabilities":{"experimentalApi":True}}); send({"method":"initialized"})
work=os.path.join(D,"vwork"); os.makedirs(work,exist_ok=True)
def score(tag,k):
    t0=time.time(); post=posts[k]; imgs=(media[k]["images"] or [])[:3]
    th=call("thread/start",{"cwd":work,"approvalPolicy":"never","sandbox":"read-only","ephemeral":True,"model":"gpt-5.6-sol","developerInstructions":RUBRIC})
    if "error" in th: return {"tag":tag,"k":k,"error":str(th["error"])[:200]}
    tid=th["result"]["thread"]["id"]; turns[tid]={"done":threading.Event()}
    inp=[{"type":"text","text":f"账号：{post['account']}\n图片总数：{len(media[k]['images'] or [])}（下面附前 {len(imgs)} 张）\n文案：\n{(post['description'] or '')[:1500]}"}]
    import urllib.request
    for n,u in enumerate(imgs):
        fp=os.path.join(work,f"{k}_{n}.jpg")
        if not os.path.exists(fp): urllib.request.urlretrieve(u+"?x-oss-process=image/resize,w_768",fp)
        inp.append({"type":"localImage","path":fp})
    r=call("turn/start",{"threadId":tid,"effort":"low","outputSchema":SCHEMA,"input":inp})
    if "error" in r: return {"tag":tag,"k":k,"error":str(r["error"])[:200]}
    turns[tid]["done"].wait(240)
    T=turns[tid]; out={"tag":tag,"k":k,"account":post["account"],"secs":round(time.time()-t0,1),"status":T.get("status"),"usage":T.get("usage")}
    try: out["scores"]=json.loads(T.get("final") or "")
    except Exception: out["raw"]=(T.get("final") or "")[:300]; out["err"]=T.get("err")
    return out
import concurrent.futures as cf
t0=time.time()
with cf.ThreadPoolExecutor(max_workers=3) as ex: res=list(ex.map(lambda a:score(*a),SAMPLE))
json.dump(res,open(f"{D}/vision.json","w"),ensure_ascii=False,indent=1)
print(f"样本 {len(res)} 条，总耗时 {time.time()-t0:.0f}s")
for r in res:
    s=r.get("scores")
    if not s: print("  失败",r.get("tag"),r.get("account"),r.get("error") or r.get("err") or r.get("raw")); continue
    u=r["usage"] or {}
    print(f'  [{r["tag"]}] {r["account"][:20]:20} 新{s["novelty"]} 独{s["distinctiveness"]} 读{s["reader_relevance"]} 信{s["info_completeness"]} 牌{s["brand_weight"]} 图{s["image_material"]} 美{s["visual_appeal"]} | {r["secs"]}s in={u.get("inputTokens")} cached={u.get("cachedInputTokens")}\n      图：{s["image_notes"]}\n      评：{s["one_line"]}')
p.terminate()
