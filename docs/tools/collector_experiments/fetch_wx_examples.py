import re, html, json, time, datetime, urllib.request, os, sys
D=os.path.dirname(os.path.abspath(__file__))
IDS=["kW77tsZvzphjZSdBw69xLw","o2Ao7Tecypn6ay1bCzXBqw","4qIAQ1Bchyz9gI8Mn5t6Iw","79BIggQkZyh2ejdcIsuvRQ","uw9gMsqbdHag--dcxu7ggQ","062AbIDlAL-j12fMx53fZQ","kk9MVmoT0aSP-43-38oyLw","9ux3Z0JyYVDc7pkbrd7elQ","rEzJY7rZmDjnG1q5ipcgpA","k-3Z9WPd7ueh-OiKeL6Rvg"]
UA="Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
def dec(x):
    x=x.replace("\\x26","&").replace("\\x0a","\n").replace("\\x22",'"').replace("\\x27","'").replace("\\x3c","<").replace("\\x3e",">").replace("\\/","/")
    return html.unescape(html.unescape(x))
out=[]
for n,i in enumerate(IDS,1):
    fp=f"{D}/{n:02d}.html"
    if not os.path.exists(fp):
        req=urllib.request.Request(f"https://mp.weixin.qq.com/s/{i}",headers={"User-Agent":UA})
        open(fp,"wb").write(urllib.request.urlopen(req,timeout=30).read()); time.sleep(2.5)
    s=open(fp,encoding="utf-8",errors="ignore").read()
    blocked=("环境异常" in s) or ("完成验证后即可继续访问" in s)
    t=re.search(r'<meta property="og:title" content="([^"]*)"',s)
    st=re.search(r"item_show_type\s*[:=]\s*['\"]?(\d+)",s)
    ct=re.search(r"create_time\s*[:=]\s*(?:JsDecode\()?['\"](\d{10})",s) or re.search(r'var ct = "(\d{10})"',s)
    body=""; imgs=0
    j=s.find('id="js_content"')
    if j>0:
        seg=s[j:]; seg=seg[:seg.find("<script")] if "<script" in seg else seg
        imgs=len(set(re.findall(r'data-src="(https://mmbiz[^"]+)"',seg)))
        body=re.sub(r"\s+"," ",html.unescape(re.sub(r"<[^>]+>"," ",seg))).strip()
    if len(body)<50:
        m=re.search(r"content_noencode\s*[:=]\s*(?:JsDecode\()?'((?:[^'\\]|\\.)*)'",s)
        if m: body=re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",dec(m.group(1)))).strip()
        imgs=max(imgs,len(set(re.findall(r"cdn_url\s*[:=]\s*(?:JsDecode\()?'(https?:[^']+?mmbiz[^']+)'",s))))
    out.append({"n":n,"id":i,"title":html.unescape(t.group(1)) if t else None,"type":{"0":"普通图文","8":"贴图文"}.get(st.group(1) if st else "", st.group(1) if st else "?"),
                "date":datetime.datetime.fromtimestamp(int(ct.group(1))).strftime("%Y-%m-%d") if ct else None,"chars":len(body),"images":imgs,"blocked":blocked,"head":body[:70]})
json.dump(out,open(f"{D}/inventory.json","w"),ensure_ascii=False,indent=1)
for o in out: print(f'{o["n"]:2} {o["date"]} {o["type"]:5} 正文{o["chars"]:5}字 图{o["images"]:3} {"被拦截" if o["blocked"] else ""} | {o["title"]}')
