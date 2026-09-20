import json, subprocess, os, time, threading, queue, re, base64, urllib.request, concurrent.futures as cf
D=os.path.dirname(os.path.abspath(__file__)); T="/private/tmp/claude-502/-Users-azure-project-csw-agent-team/08ba728f-a585-483c-b169-7112f8dfe4f4/scratchpad/tsx"
posts={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{T}/posts.jsonl") if l.strip()}
media={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{T}/media.jsonl") if l.strip()}
def find(acc, pat):
    for k,p in posts.items():
        if p["account"]==acc and re.search(pat,p["description"] or "") and (media.get(k,{}).get("images")): return k
PICK=[("DdZwiM9EnWL","gnuhr"),("DdYgHUNE6ux","wm_moonstar"),("DdYmJ-ahq34","s2w8_benmiller"),("DdVqdOJmDr5","omnium"),
 (find("robin_outdoor_base","Brompton"),"brompton"),(find("zanearts_outdoor","ゼクールー"),"zanearts"),(find("nepenthes.official","SOREL"),"needles_sorel"),
 (find("nepenthes.official","SUBU"),"s2w8_nanga_subu"),(find("helinox_kr","HERITAGEFLOSS"),"helinox_heritage"),(find("sundaymountain.jp","GOAL ZERO"),"goalzero"),
 (find("bpr_beams","BD Barcelona"),"bd_dali"),(find("houyhnhnm_official","REGAL"),"regal_converse"),(find("le.syndrome","Kith"),"kith_on"),(find("mt.sumi","再入荷"),"mtsumi_restock")]
PICK=[(k,n) for k,n in PICK if k]
DIMS=["change","use","gain","compare","explain","csw"]
SCHEMA={"type":"object","additionalProperties":False,
 "required":["brand","title","dims","evidence","look","image_seen","three_sentences","unanswered","gaps","priority_hits","lower_hits","kind"],
 "properties":{"brand":{"type":"string"},"title":{"type":"string"},
  "dims":{"type":"object","additionalProperties":False,"required":DIMS,"properties":{d:{"type":"integer","minimum":0,"maximum":10} for d in DIMS}},
  "evidence":{"type":"object","additionalProperties":False,"required":DIMS,"properties":{d:{"type":"string"} for d in DIMS}},
  "look":{"type":"string"},"image_seen":{"type":"boolean"},
  "three_sentences":{"type":"object","additionalProperties":False,"required":["what_changed","why_it_matters","how_different"],"properties":{k:{"type":"string"} for k in ["what_changed","why_it_matters","how_different"]}},
  "unanswered":{"type":"string","enum":["none","missing_material","angle_not_formed","low_value"]},
  "gaps":{"type":"array","items":{"type":"string"}},
  "priority_hits":{"type":"array","items":{"type":"string"}},"lower_hits":{"type":"array","items":{"type":"string"}},
  "kind":{"type":"string","enum":["新品发布","联名","复刻或补货","门店或活动","品牌故事","穿搭或用户内容","促销或通知","其他"]}}}
RUBRIC="""你按 CSW（营事编集室，面向国内户外潮流爱好者的公众号）的选题判断框架给一条 Instagram 贴文打分。只依据给你的文案和图片，不要上网，不要调用工具。
核心判断：这件事有没有足够具体的变化，值得 CSW 帮读者解释一次。非新品也可以因为有依据的设计、文化、历史或当下关联而值得报道。
六个维度各给 0–10 的整数，并在 evidence 里用一句话写依据（引用文案或图里看到的事实）：
change 具体变化或报道切口：结构或使用方式改了什么、品牌首次做了什么、原产品为什么改、某种户外行为有什么新做法；非新品看有没有具体的设计、文化、历史切口。只能概括成"某品牌发布新品"通常不够（≤4）。
use 使用关联：人怎么穿、搭、住、跑、移动，怎样在城市与户外之间切换；产品为什么这样设计。纯配色、普通 logo 联名、没有新信息的复刻通常低（≤3）。
gain 信息增量：读者能得到什么——新的使用思路、值得了解的设计逻辑、文化背景、对户外生活变化的解释，或帮助理解某类产品。
compare 比较参照：与上一代、品牌以往做法或同类产品有什么不同，为什么现在出现。文案与图里有明确参照给高分；没有参照给低分，不要编造。
explain 可解释性：能否形成有事实支持的编辑判断。只有"好看""有趣""值得关注"通常不成熟（≤4）。
csw CSW 视角：对户外潮流爱好者有用、有趣、有料的程度；户外专业知识与潮流文化判断力。
品牌知名度不是维度，不要因为大牌加分。
另外：look 用不超过 40 字写实图里看到的外观与设计要点；image_seen 表示你是否真的看到了图片；three_sentences 试答三句话（发生了什么变化 / 为什么值得户外用户知道 / 它和以前有什么不一样），每句不超过 40 字；答不清时 unanswered 填 missing_material（缺资料）、angle_not_formed（角度未成立）或 low_value（价值不足），否则 none；gaps 列出缺的资料（如发售日期、价格、官方来源、图片不足）；priority_hits 从这七类里选命中的：老产品结构性改款 / 解决具体使用问题 / 品牌新的空间社群或服务方式 / 装备进入新的生活场景 / 小众品类的新设计 / 存在真实争议或取舍 / 能解释消费或生活方式变化；lower_hits 从这七类里选命中的：只有新配色或普通logo联名 / 换材质但说不清差异 / 宣称创新但无可核实变化 / 纯明星带货 / 缺少可靠来源 / 只能说好看 / 强行拔高成趋势。brand 写品牌名，title 用不超过 24 字写"品牌｜一句话标题"。"""
p=subprocess.Popen(["codex","app-server"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,text=True,bufsize=1)
lock=threading.Lock(); pending={}; turns={}
def send(o):
    with lock: p.stdin.write(json.dumps(o,ensure_ascii=False)+"\n"); p.stdin.flush()
def reader():
    for line in p.stdout:
        try: m=json.loads(line)
        except Exception: continue
        if "id" in m and "method" in m: send({"id":m["id"],"result":{"decision":"decline"}}); continue
        if "id" in m and m["id"] in pending: pending[m["id"]].put(m); continue
        prm=m.get("params",{}); tid=prm.get("threadId"); meth=m.get("method")
        if tid in turns:
            if meth=="item/completed" and prm["item"].get("type")=="agentMessage": turns[tid]["final"]=prm["item"].get("text")
            if meth=="thread/tokenUsage/updated": turns[tid]["usage"]=prm["tokenUsage"]["total"]
            if meth=="turn/completed": turns[tid]["status"]=prm["turn"].get("status"); turns[tid]["done"].set()
threading.Thread(target=reader,daemon=True).start()
_id=[0]
def call(method,params,timeout=120):
    _id[0]+=1; i=_id[0]; pending[i]=queue.Queue(); send({"id":i,"method":method,"params":params}); return pending[i].get(timeout=timeout)
call("initialize",{"clientInfo":{"name":"csw-collector-design","title":"design","version":"0.0.3"},"capabilities":{"experimentalApi":True}}); send({"method":"initialized"})
work=os.path.join(D,"vwork"); os.makedirs(work,exist_ok=True)
def thumb(u,w):
    fp=os.path.join(work,re.sub(r"[^A-Za-z0-9]","_",u)[-40:]+f"_{w}.jpg")
    if not os.path.exists(fp): urllib.request.urlretrieve(u+f"?x-oss-process=image/resize,w_{w}",fp)
    return fp
def score(k,name):
    t0=time.time(); post=posts[k]; imgs=(media[k]["images"] or [])[:3]
    th=call("thread/start",{"cwd":work,"approvalPolicy":"never","sandbox":"read-only","ephemeral":True,"model":"gpt-5.6-sol","developerInstructions":RUBRIC})
    tid=th["result"]["thread"]["id"]; turns[tid]={"done":threading.Event()}
    inp=[{"type":"text","text":f"账号：{post['account']}\n发布时间：{post['date_posted']}\n图片总数：{len(media[k]['images'] or [])}（附前 {len(imgs)} 张）\n点赞 {post['likes']}，账号粉丝 {post['followers']}\n文案：\n{(post['description'] or '')[:1500]}"}]
    inp+=[{"type":"localImage","path":thumb(u,768)} for u in imgs]
    call("turn/start",{"threadId":tid,"effort":"low","outputSchema":SCHEMA,"input":inp}); turns[tid]["done"].wait(240)
    T=turns[tid]; out={"name":name,"shortcode":k,"account":post["account"],"date_posted":post["date_posted"],"likes":post["likes"],"followers":post["followers"],"url":post["url"],
         "caption":(post["description"] or "")[:600],"n_images":len(media[k]["images"] or []),"secs":round(time.time()-t0,1),"usage":T.get("usage")}
    try: out["scores"]=json.loads(T.get("final") or "")
    except Exception: out["error"]=(T.get("final") or "")[:200]
    out["thumbs"]=[]
    for u in imgs[:2]:
        fp=thumb(u,240); out["thumbs"].append("data:image/jpeg;base64,"+base64.b64encode(open(fp,"rb").read()).decode())
    return out
t0=time.time()
with cf.ThreadPoolExecutor(max_workers=3) as ex: res=list(ex.map(lambda a:score(*a),PICK))
json.dump(res,open(f"{D}/events_scored.json","w"),ensure_ascii=False,indent=1)
print(f"{len(res)} 事件 {time.time()-t0:.0f}s")
for r in res:
    s=r.get("scores")
    if not s: print("  失败",r["name"],r.get("error")); continue
    d=s["dims"]; print(f'  {r["name"]:16} {s["title"][:22]:22} '+" ".join(f"{k}{d[k]}" for k in DIMS)+f'  图{s["image_seen"]} {s["unanswered"]} | {r["secs"]}s')
p.terminate()
