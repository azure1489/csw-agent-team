import json, os, re, collections, statistics as st
D=os.path.dirname(os.path.abspath(__file__))
posts={json.loads(l)["shortcode"]:json.loads(l) for l in open(f"{D}/posts.jsonl") if l.strip()}
dims={json.loads(l)["shortcode"]:json.loads(l)["answers"] for l in open(f"{D}/dims.jsonl")}
labels=[l for l in json.load(open(f"{D}/labels.json")) if l["shortcode"] in dims]
DIM=["novelty","distinctiveness","reader_relevance","info_completeness","brand_weight"]
CN={"novelty":"新品性","distinctiveness":"独特性","reader_relevance":"读者相关","info_completeness":"信息量","brand_weight":"品牌分量"}
def vec(k): return [dims[k][d]["score"] for d in DIM]
print("== 各维度分布（0–4）")
for d in DIM:
    v=sorted(dims[k][d]["score"] for k in dims); conf=st.mean(dims[k][d]["confidence"] for k in dims)
    print(f"  {CN[d]:6} 中位 {v[len(v)//2]:.2f}  P90 {v[int(len(v)*.9)]:.2f}  平均置信度 {conf:.2f}")
print("\n== 有 Van 态度的三条（同一窗口内）")
for l in labels:
    if l["status"] in ("written","rejected"):
        k=l["shortcode"]; print(f'  [{"Van 亲选" if l["status"]=="written" else "Van 否决"}] {l["brand"][:20]:20} '+"  ".join(f"{CN[d]} {dims[k][d]['score']:.1f}" for d in DIM))
# 两套权重：A 偏「新闻性」，B 偏「东西本身」
WA={"novelty":.35,"distinctiveness":.15,"reader_relevance":.15,"info_completeness":.2,"brand_weight":.15}
WB={"novelty":.10,"distinctiveness":.45,"reader_relevance":.20,"info_completeness":.10,"brand_weight":.15}
def comp(k,W): return sum(W[d]*dims[k][d]["score"]/4 for d in DIM)
for name,W in (("A 偏新闻性",WA),("B 偏东西本身",WB)):
    order=sorted(dims,key=lambda k:-comp(k,W)); rank={k:i+1 for i,k in enumerate(order)}
    print(f"\n== 权重 {name}：三条的排名（共 {len(order)}）")
    for l in labels:
        if l["status"] in ("written","rejected"): print(f'   {"亲选" if l["status"]=="written" else "否决"} {l["brand"][:20]:20} 第 {rank[l["shortcode"]]} 名')
order=sorted(dims,key=lambda k:-dims[k]["distinctiveness"]["score"])
print("\n== 「独特性」最高的 14 条（去掉同账号重复）")
seen=set(); n=0
for k in order:
    p=posts[k]; key=(p["account"], re.sub(r"\s+","",p["description"] or "")[:30])
    if key in seen: continue
    seen.add(key); n+=1
    print(f'  {dims[k]["distinctiveness"]["score"]:.1f} 新{dims[k]["novelty"]["score"]:.1f} 读{dims[k]["reader_relevance"]["score"]:.1f} {p["account"][:22]:22} | {(p["description"] or "")[:70].replace(chr(10)," ")}')
    if n>=14: break
