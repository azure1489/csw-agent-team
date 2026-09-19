import json, os, re, collections, statistics as st
D=os.path.dirname(os.path.abspath(__file__))
posts={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{D}/posts.jsonl") if l.strip()}
ans={json.loads(l)["shortcode"]:json.loads(l)["answers"] for l in open(f"{D}/answers.jsonl")}
labels=json.load(open(f"{D}/labels.json"))
def comp(a):
    n=a["newsworthiness"]["score"]/4
    return (0.35*a["new_product"]["noul"] + 0.15*a["concrete_details"]["noul"] + 0.30*n + 0.10*a["collaboration"]["noul"]
            - 0.30*a["styling_or_ugc"]["noul"] - 0.20*a["promotion_admin"]["noul"] - 0.10*a["store_or_event"]["noul"])
score={k:comp(a) for k,a in ans.items()}
order=sorted(score, key=score.get, reverse=True); rank={k:i+1 for i,k in enumerate(order)}
N=len(order)
def lang(t):
    if re.search(r"[぀-ヿ]",t): return "ja"
    if re.search(r"[가-힯]",t): return "ko"
    if re.search(r"[一-鿿]",t): return "zh"
    return "en/other"
print("== 语言分布与 kind 置信度均值")
bl=collections.defaultdict(list)
for k,a in ans.items(): bl[lang(posts[k].get("description") or "")].append(a["kind"]["confidence"])
for l,v in sorted(bl.items(), key=lambda x:-len(x[1])): print(f"  {l:9} n={len(v):3} kind置信度均值={st.mean(v):.2f} 低于0.6的占比={sum(1 for x in v if x<0.6)/len(v):.0%}")
print("\n== kind 分布:", dict(collections.Counter(a["kind"]["choice"] for a in ans.values()).most_common()))
print("== 漏斗: 综合分 ≥0.5:", sum(1 for v in score.values() if v>=0.5), " ≥0.6:", sum(1 for v in score.values() if v>=0.6), " ≥0.7:", sum(1 for v in score.values() if v>=0.7), f" / {N}")
lab={l["shortcode"]:l for l in labels if l["shortcode"] in ans}
print(f"\n== 有标签的 {len(lab)} 条")
for l in lab.values():
    if l["status"] in ("written","rejected","pending_check","shortlisted","approved_write"):
        k=l["shortcode"]; a=ans[k]
        print(f'  [{l["status"]:13}] 排名 {rank[k]:3}/{N} 综合分 {score[k]:.2f} new={a["new_product"]["noul"]:.2f} news={a["newsworthiness"]["score"]:.1f} kind={a["kind"]["choice"]} | r{l["run_id"]} {l["brand"]} {l["title"][:26]}')
by=collections.defaultdict(list)
for l in lab.values():
    if l["status"]=="dropped": by[(l["reason_code"] or "?", l["actor"] or "?")].append(score[l["shortcode"]])
print("\n== agent 淘汰的条目，按理由看 Jev 综合分")
for (rc,actor),v in sorted(by.items(), key=lambda x:-len(x[1])):
    v=sorted(v); print(f"  {rc:18} {actor:10} n={len(v):3} 中位数={v[len(v)//2]:.2f} 高于0.6的条数={sum(1 for x in v if x>=0.6)}")
unl=[k for k in order if k not in lab]
print(f"\n== 从未被任何人看过的 {len(unl)} 条里，Jev 排最前的 22 条")
for k in unl[:22]:
    p=posts[k]; a=ans[k]
    print(f'  #{rank[k]:3} {score[k]:.2f} news={a["newsworthiness"]["score"]:.1f} {a["kind"]["choice"][:15]:15} {p["account"][:24]:24} {str(p["date_posted"])[5:16]} | {(p["description"] or "")[:64].replace(chr(10)," ")}')
print("\n== agent 标 no_value 淘汰、但 Jev 给高分的（看谁错）")
hi=[(score[l["shortcode"]],l) for l in lab.values() if l["status"]=="dropped" and l["reason_code"]=="no_value" and score[l["shortcode"]]>=0.6]
for s,l in sorted(hi, key=lambda x:-x[0])[:10]:
    p=posts[l["shortcode"]]; print(f'  {s:.2f} {p["account"][:22]:22} r{l["run_id"]} {l["actor"]:10} | {l["title"][:30]} | {(p["description"] or "")[:50].replace(chr(10)," ")}')
json.dump({k:{"rank":rank[k],"score":round(score[k],3)} for k in order}, open(f"{D}/ranking.json","w"))
