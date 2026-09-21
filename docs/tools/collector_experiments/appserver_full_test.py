# -*- coding: utf-8 -*-
"""codex app-server 完整实测：csw-subapi 网关 + gpt-6-astra。
七项：握手 / MCP 挂载 / 建线程 / 通路 / 结构化输出 / 图片 / 只读沙箱 / MCP 工具调用 / 白名单"""
import json, os, subprocess, threading, time, queue, sys

CODEX = "/root/.hermes/node/bin/codex"
HOME = "/opt/csw-collector/codex-home"
PROFILE_ENV = "/root/.hermes/profiles/csw-agent/.env"   # csw-subapi 凭据 + CSW MCP 凭据都在这里
MODEL = "gpt-6-astra"

def load_env():
    """只用 csw-agent profile 的凭据（csw-subapi.833233.xyz）。"""
    env = dict(os.environ)
    want = {"SUB2API_API_KEY", "SUB2API_BASE_URL", "CSW_API_KEY", "CSW_API_URL"}
    for line in open(PROFILE_ENV):
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, _, v = line.partition("=")
            k = k.strip()
            if k in want:
                env[k] = v.strip().strip('"').strip("'")
    env["CODEX_HOME"] = HOME
    env["RUST_LOG"] = "error"
    return env

class C:
    def __init__(self, env):
        self.p = subprocess.Popen([CODEX, "app-server"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, cwd="/tmp", text=True, bufsize=1)
        self.q = queue.Queue(); self.notes = []; self._id = 0
        threading.Thread(target=self._r, daemon=True).start()
        threading.Thread(target=self._e, daemon=True).start()
    def _r(self):
        for l in self.p.stdout:
            l = l.strip()
            if l:
                try: self.q.put(json.loads(l))
                except Exception: self.notes.append("RAW " + l[:200])
    def _e(self):
        for l in self.p.stderr:
            if l.strip(): self.notes.append("ERR " + l.strip()[:250])
    def send(self, m, p=None):
        self._id += 1
        d = {"jsonrpc": "2.0", "id": self._id, "method": m}
        if p is not None: d["params"] = p
        self.p.stdin.write(json.dumps(d) + "\n"); self.p.stdin.flush(); return self._id
    def notify(self, m, p=None):
        d = {"jsonrpc": "2.0", "method": m}
        if p is not None: d["params"] = p
        self.p.stdin.write(json.dumps(d) + "\n"); self.p.stdin.flush()
    def reply(self, i, r):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "result": r}) + "\n"); self.p.stdin.flush()
    def pump(self, until_id=None, until_sub=None, timeout=180):
        end = time.time() + timeout; resp = None; ev = []
        while time.time() < end:
            try: m = self.q.get(timeout=1)
            except queue.Empty: continue
            if until_id is not None and m.get("id") == until_id and ("result" in m or "error" in m):
                resp = m
                if until_sub is None: return resp, ev
                continue
            meth = m.get("method")
            if not meth: continue
            ev.append(m)
            if "id" in m:
                if any(s in meth for s in ("equestApproval", "licitation", "equestUserInput")):
                    self.reply(m["id"], {"decision": "denied"})
                else:
                    self.reply(m["id"], {})
            if until_sub and until_sub in meth:
                return resp, ev
        return resp, ev
    def close(self):
        try: self.p.stdin.close(); self.p.wait(timeout=5)
        except Exception: self.p.kill()

def items(ev, typ):
    return [(e.get("params") or {}).get("item") or {} for e in ev
            if ((e.get("params") or {}).get("item") or {}).get("type") == typ]

def answer(ev):
    return " | ".join(i.get("text", "") for i in items(ev, "agentMessage") if i.get("text"))

def errors(ev):
    return [e for e in ev if e.get("method") == "error"]

def warns(ev):
    return [e for e in ev if e.get("method") == "warning"]

def tokens(ev):
    tot = 0
    for e in ev:
        tu = (e.get("params") or {}).get("tokenUsage") or {}
        t = (tu.get("total") or {}).get("totalTokens")
        if t: tot = max(tot, t)
    return tot

RESULT = {}
def mark(name, ok, note=""):
    RESULT[name] = ok
    print("  %s %s %s" % ("[OK]  " if ok else "[FAIL]", name, ("— " + note) if note else ""), flush=True)

env = load_env()
print("网关:", env.get("SUB2API_BASE_URL"), "| 模型:", MODEL, flush=True)
print("CSW:", env.get("CSW_API_URL"), flush=True)
c = C(env)

print("\n=== 1 握手 ===", flush=True)
i = c.send("initialize", {"clientInfo": {"name": "csw-collector", "title": "CSW 采集服务", "version": "0.0.1"}})
r, _ = c.pump(until_id=i, timeout=30)
mark("握手", bool(r and "result" in r), json.dumps((r or {}).get("result", {}), ensure_ascii=False)[:120])
c.notify("initialized", {})

print("\n=== 2 MCP 挂载（csw 只读白名单）===", flush=True)
names = []
for attempt in range(6):
    i = c.send("mcpServerStatus/list", {})
    r, _ = c.pump(until_id=i, timeout=90)
    data = ((r or {}).get("result") or {}).get("data") or []
    if data:
        for s in data:
            tl = s.get("tools") or {}
            names = sorted(tl.keys()) if isinstance(tl, dict) else []
            print("   服务器 %s v%s，工具 %d 个" % (s.get("name"),
                (s.get("serverInfo") or {}).get("version"), len(names)), flush=True)
            print("   工具:", names, flush=True)
        break
    time.sleep(15)
mark("MCP 挂载", bool(names), "%d 个工具" % len(names))
mark("白名单生效（无推送类工具）", all("push" not in n and "create" not in n and "update" not in n for n in names),
     "危险工具: " + str([n for n in names if any(x in n for x in ("push", "create", "update", "delete"))]))

print("\n=== 3 建线程（gpt-6-astra）===", flush=True)
i = c.send("thread/start", {"model": MODEL, "modelProvider": "sub2api", "cwd": "/tmp",
    "sandbox": "read-only", "approvalPolicy": "never",
    "developerInstructions": "你是 CSW 情报深核助手。需要查贴文用 csw 的 MCP 工具。回答简短。"})
r, ev = c.pump(until_id=i, timeout=120)
res = (r or {}).get("result", {})
tid = (res.get("thread") or {}).get("id")
mark("建线程", bool(tid), "threadId=" + str(tid))
if not tid:
    print(json.dumps(r, ensure_ascii=False)[:500], flush=True)
    for n in c.notes[:10]: print("   ", n, flush=True)
    c.close(); sys.exit(1)

def turn(label, text, schema=None, extra_input=None, timeout=240):
    print("\n=== %s ===" % label, flush=True)
    inp = list(extra_input or []) + [{"type": "text", "text": text}]
    params = {"threadId": tid, "input": inp}
    if schema: params["outputSchema"] = schema
    t = time.time()
    i = c.send("turn/start", params)
    r, ev = c.pump(until_id=i, until_sub="turn/completed", timeout=timeout)
    dur = time.time() - t
    errs = errors(ev); ws = warns(ev)
    if r and "error" in r:
        print("   请求被拒:", json.dumps(r["error"], ensure_ascii=False)[:300], flush=True)
        return False, dur, ev
    if errs:
        print("   错误 %d 条: %s" % (len(errs),
            json.dumps(errs[0].get("params", {}), ensure_ascii=False)[:280]), flush=True)
    if ws:
        print("   警告:", [w["params"].get("message", "")[:70] for w in ws], flush=True)
    a = answer(ev)
    print("   用时 %.1f 秒，token 累计 %s" % (dur, tokens(ev) or "n/a"), flush=True)
    print("   回答:", a[:400] if a else "(空)", flush=True)
    return (not errs and bool(a)), dur, ev

ok, d, ev = turn("4 通路验证", "只回答两个字：收到")
mark("通路（csw-subapi + gpt-6-astra）", ok, "%.1f 秒" % d)
mark("无模型元数据警告", not warns(ev))

schema = {"type": "object", "additionalProperties": False, "required": ["tier", "reason"],
    "properties": {"tier": {"type": "string", "enum": ["recommend", "alternate", "not_recommend", "pending_check"]},
                   "reason": {"type": "string"}}}
ok, d, ev = turn("5 结构化输出", "某户外品牌只出了新配色，没有其他信息。按 CSW 标准给结论档与一句理由。", schema=schema)
a = answer(ev); valid = False
try:
    o = json.loads(a); valid = o.get("tier") in ("recommend", "alternate", "not_recommend", "pending_check") and bool(o.get("reason"))
except Exception: pass
mark("结构化输出合 schema", valid, a[:200])

ok, d, ev = turn("6 本地图片输入", "这张图里有什么？只说图上看到的，一句话。",
    extra_input=[{"type": "localImage", "path": "/tmp/csw_probe_image.png"}])
mark("本地图片输入", ok, "%.1f 秒" % d)

ok, d, ev = turn("7 只读沙箱", "用 shell 执行：touch /tmp/codex_sandbox_probe_file 。告诉我成功还是被拒绝。")
cmds = items(ev, "commandExecution")
blocked = any("only" in json.dumps(x, ensure_ascii=False).lower() or x.get("exitCode") not in (0, None) for x in cmds)
mark("只读沙箱拦住写", blocked or (not os.path.exists("/tmp/codex_sandbox_probe_file")),
     "执行了 %d 条命令" % len(cmds))

ok, d, ev = turn("8 MCP 工具真实调用",
    "用 csw 的工具 csw_accounts_list 查账号清单，只报前 3 个账号名和总数。", timeout=300)
blob = json.dumps(ev, ensure_ascii=False)
used = "csw_accounts_list" in blob or "mcpTool" in blob or "toolCall" in blob
mark("MCP 工具可调用", ok and used, "用时 %.1f 秒" % d)

print("\n" + "=" * 50, flush=True)
print("汇总:", json.dumps(RESULT, ensure_ascii=False), flush=True)
print("通过 %d / %d" % (sum(1 for v in RESULT.values() if v), len(RESULT)), flush=True)
if c.notes:
    print("\n--- stderr ---", flush=True)
    for n in c.notes[-8:]: print("  ", n, flush=True)
c.close()
print("完成", flush=True)
