// ============================================================
// Pages: Login · Dashboard · Workflows list
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

// ---------------- LOGIN ----------------
function LoginPage({ onLogin }) {
  const [username, setUsername] = useState("zhang");
  const [password, setPassword] = useState("········");
  const [remember, setRemember] = useState(true);
  const [err, setErr] = useState("");
  const [loading, setLoading] = useState(false);
  const submit = () => {
    if (!username || !password) { setErr("请输入用户名和密码"); return; }
    setErr(""); setLoading(true);
    setTimeout(() => { setLoading(false); onLogin(username); }, 650);
  };
  return (
    <div style={{ height: "100vh", display: "flex", alignItems: "center", justifyContent: "center", background: "radial-gradient(900px 500px at 50% -10%, #eeeefb, var(--bg))" }}>
      <div className="col" style={{ width: 380, gap: 22, animation: "popIn .25s" }}>
        <div className="col" style={{ alignItems: "center", gap: 12 }}>
          <div style={{ width: 46, height: 46, borderRadius: 12, background: "linear-gradient(150deg,#6d6df0,#5151c4)", display: "flex", alignItems: "center", justifyContent: "center", boxShadow: "0 6px 16px -4px rgba(80,80,180,.5)" }}>
            <I.flow size={26} style={{ color: "#fff" }} />
          </div>
          <div className="col" style={{ alignItems: "center", gap: 2 }}>
            <div style={{ fontSize: 18, fontWeight: 650 }}>任务流转后台</div>
            <div className="t2 sm">营事编集室 · 工作流控制台</div>
          </div>
        </div>
        <Card style={{ padding: 26 }}>
          <div className="col gap16">
            <Field label="用户名" htmlFor="u">
              <TextInput value={username} onChange={setUsername} placeholder="请输入用户名" icon={I.user} autoFocus
                onKeyDown={(e) => e.key === "Enter" && submit()} />
            </Field>
            <Field label="密码" htmlFor="p">
              <TextInput value={password} onChange={(v) => { setPassword(v); setErr(""); }} type="password" placeholder="请输入密码" icon={I.lock}
                onKeyDown={(e) => e.key === "Enter" && submit()} />
            </Field>
            <div className="row between">
              <Checkbox checked={remember} onChange={setRemember} label="记住我" />
              <a className="sm" style={{ color: "var(--accent-text)", fontWeight: 550 }} onClick={() => toast("请联系管理员重置密码", "warn")}>忘记密码？</a>
            </div>
            {err && (
              <div className="row gap8" style={{ background: "var(--red-bg)", border: "1px solid var(--red-bd)", color: "var(--red)", padding: "8px 11px", borderRadius: 7, fontSize: 12.5 }}>
                <I.xCircle size={15} />{err}
              </div>
            )}
            <Btn variant="primary" size="lg" full onClick={submit} disabled={loading}>
              {loading ? "登录中…" : "登 录"}
            </Btn>
          </div>
        </Card>
        <div className="t3 xs" style={{ textAlign: "center", lineHeight: 1.6 }}>
          演示账号已预填 · 登录后可在右上角切换角色（superadmin / operator / viewer）<br />体验不同权限下的页面与按钮可见性
        </div>
      </div>
    </div>
  );
}

// ---------------- DASHBOARD ----------------
function StatCard({ label, value, icon: IconC, tone = "violet", sub, onClick }) {
  const t = TONES[tone];
  return (
    <Card hover={!!onClick} onClick={onClick} style={{ padding: "16px 18px" }}>
      <div className="row between" style={{ alignItems: "flex-start" }}>
        <div className="col" style={{ gap: 6 }}>
          <span className="t2 sm" style={{ fontWeight: 550 }}>{label}</span>
          <span style={{ fontSize: 27, fontWeight: 680, letterSpacing: "-.02em", lineHeight: 1 }}>{value}</span>
          {sub && <span className="t3 xs">{sub}</span>}
        </div>
        <span style={{ width: 34, height: 34, borderRadius: 9, background: t.bg, color: t.c, display: "flex", alignItems: "center", justifyContent: "center" }}>
          <IconC size={18} />
        </span>
      </div>
    </Card>
  );
}

function Dashboard({ navigate }) {
  const o = OVERVIEW;
  return (
    <>
      <PageHeader title="仪表盘" desc="一眼看清后台与运行健康度。卡片可点击跳转对应列表 / 详情。"
        actions={<Btn variant="default" icon={I.refresh} onClick={() => toast("已刷新概览数据", "success")}>刷新</Btn>} />

      <div style={{ display: "grid", gridTemplateColumns: "repeat(4,1fr)", gap: 14, marginBottom: 18 }}>
        <StatCard label="已激活工作流" value={o.active_workflows} icon={I.workflow} tone="green" sub="共 3 个工作流类型" onClick={() => navigate({ page: "workflows" })} />
        <StatCard label="在跑实例" value={o.running} icon={I.runs} tone="blue" sub="实时运行中" onClick={() => navigate({ page: "runs" })} />
        <StatCard label="今日完成" value={o.done_today} icon={I.checkCircle} tone="violet" sub="2026-06-09" />
        <StatCard label="待处理告警" value={o.alerts} icon={I.warn} tone={o.alerts ? "amber" : "gray"} sub={o.alerts ? "1 个 token 已过期" : "一切正常"} onClick={() => navigate({ page: "members" })} />
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "1.6fr 1fr", gap: 14, marginBottom: 14 }}>
        {/* running instances */}
        <SectionCard title="在跑实例" subtitle="点击进入运行详情（只读）" noPad
          actions={<Btn variant="ghost" size="sm" iconRight={I.chevRight} onClick={() => navigate({ page: "runs" })}>全部</Btn>}>
          {o.running_list.length === 0 ? (
            <EmptyState icon={I.runs} title="今日暂无运行实例" desc="工作流被触发后，实例会出现在这里" compact />
          ) : (
            <div className="col">
              {o.running_list.map((r, i) => (
                <div key={r.id} className="row between" onClick={() => navigate({ page: "run-detail", id: r.id })}
                  style={{ padding: "14px 16px", borderTop: i ? "1px solid var(--border)" : "none", cursor: "pointer" }}
                  onMouseEnter={(e) => e.currentTarget.style.background = "var(--surface-2)"}
                  onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}>
                  <div className="row gap12">
                    <span className="mono t3" style={{ fontSize: 12.5, width: 34 }}>#{r.id}</span>
                    <div className="col" style={{ gap: 3 }}>
                      <div className="row gap8"><span className="b" style={{ fontSize: 13.5 }}>{r.name}</span><span className="t3 mono sm">{r.subject}</span></div>
                      <div className="row gap8 t2 sm"><span>当前：{r.cur}</span></div>
                    </div>
                  </div>
                  <div className="row gap12">
                    <ProgressPill text={r.progress} />
                    <StatusBadge status={r.status} map={(s) => s === "active" ? "run_active" : s} />
                    <I.chevRight size={16} style={{ color: "var(--text-3)" }} />
                  </div>
                </div>
              ))}
            </div>
          )}
        </SectionCard>

        {/* recent changes */}
        <SectionCard title="最近激活 / 改动" noPad>
          <div className="col">
            {o.recent_changes.map((c, i) => (
              <div key={i} className="row between" style={{ padding: "12px 16px", borderTop: i ? "1px solid var(--border)" : "none" }}>
                <div className="row gap10">
                  <span style={{ width: 30, height: 30, borderRadius: 7, background: "var(--surface-2)", border: "1px solid var(--border)", display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-2)" }}><I.workflow size={15} /></span>
                  <div className="col" style={{ gap: 2 }}>
                    <div className="row gap6"><span className="mono b" style={{ fontSize: 12.5 }}>{c.name}</span><span className="t3 mono xs">{c.ver}</span></div>
                    <span className="t2 xs">{c.what}</span>
                  </div>
                </div>
                <div className="col" style={{ alignItems: "flex-end", gap: 4 }}>
                  <StatusBadge status={c.status} sm />
                  <span className="t3 xs mono">{c.t}</span>
                </div>
              </div>
            ))}
          </div>
        </SectionCard>
      </div>

      {/* recent audit */}
      <SectionCard title="最近审计" subtitle="后台操作留痕" noPad
        actions={<Btn variant="ghost" size="sm" iconRight={I.chevRight} onClick={() => navigate({ page: "audit" })}>全部</Btn>}>
        <div className="col">
          {o.recent_audit.map((a, i) => (
            <div key={i} className="row gap12" style={{ padding: "11px 16px", borderTop: i ? "1px solid var(--border)" : "none" }}>
              <span className="t3 mono sm" style={{ width: 110, flexShrink: 0 }}>{a.t.slice(5)}</span>
              <Avatar name={a.user} size={22} />
              <span className="b sm" style={{ width: 54 }}>{a.user}</span>
              <ActionTag action={a.action} />
              <span className="t2 sm grow mono" style={{ textAlign: "right" }}>{a.target}</span>
            </div>
          ))}
        </div>
      </SectionCard>
    </>
  );
}

function ProgressPill({ text }) {
  return <span className="row gap6 mono" style={{ fontSize: 12, fontWeight: 600, color: "var(--accent-text)", background: "var(--accent-weak)", border: "1px solid var(--accent-weak-2)", padding: "2px 9px", borderRadius: 999 }}>{text}</span>;
}

const ACTION_META = {
  login: { label: "登录", tone: "gray" }, logout: { label: "登出", tone: "gray" },
  workflow_create: { label: "创建工作流", tone: "violet" }, workflow_activate: { label: "激活工作流", tone: "green" },
  workflow_archive: { label: "归档工作流", tone: "gray" }, token_issue: { label: "签发 token", tone: "blue" },
  token_revoke: { label: "吊销 token", tone: "amber" }, agent_disable: { label: "禁用成员", tone: "red" },
  role_update: { label: "修改角色", tone: "violet" }, user_create: { label: "创建用户", tone: "blue" },
};
function ActionTag({ action }) {
  const m = ACTION_META[action] || { label: action, tone: "gray" };
  return <Badge tone={m.tone} sm>{m.label}</Badge>;
}

// ---------------- WORKFLOWS LIST ----------------
function WorkflowsList({ navigate, user, state, dispatch }) {
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("");
  const [confirm, setConfirm] = useState(null);
  const [jsonView, setJsonView] = useState(null);
  const [newOpen, setNewOpen] = useState(false);
  const canWrite = user.role !== "viewer";

  const rows = state.workflows
    .filter((w) => (!search || w.name.includes(search) || w.wf_key.includes(search)))
    .filter((w) => (!status || w.status === status))
    .sort((a, b) => a.wf_key.localeCompare(b.wf_key) || b.version - a.version);

  const roleName = (c) => (ROLES.find((r) => r.code === c) || {}).name || c;

  return (
    <>
      <PageHeader title="工作流" desc="浏览所有工作流类型及其版本。仅草稿可编辑；激活 / 归档版本为只读查看。"
        actions={canWrite && <Btn variant="primary" icon={I.plus} onClick={() => setNewOpen(true)}>新建工作流</Btn>} />

      <Card noPad>
        <div className="row between" style={{ padding: "12px 14px", borderBottom: "1px solid var(--border)", gap: 12 }}>
          <div style={{ width: 280 }}><TextInput value={search} onChange={setSearch} placeholder="搜索名称或 key…" icon={I.search} /></div>
          <div className="row gap8">
            <span className="t3 sm row gap4"><I.filter size={14} />筛选</span>
            <div style={{ width: 130 }}><Select value={status} onChange={setStatus} placeholder="全部状态" options={[{ value: "", label: "全部状态" }, { value: "active", label: "已激活" }, { value: "draft", label: "草稿" }, { value: "archived", label: "已归档" }]} sm /></div>
          </div>
        </div>

        {rows.length === 0 ? (
          <EmptyState icon={I.workflow} title="还没有工作流" desc="点击右上角「新建工作流」创建第一个工作流定义" action={canWrite && <Btn variant="primary" icon={I.plus} onClick={() => setNewOpen(true)}>新建工作流</Btn>} />
        ) : (
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
            <thead>
              <tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
                <Th style={{ paddingLeft: 18 }}>名称</Th><Th>key</Th><Th>版本</Th><Th>状态</Th><Th>中枢</Th><Th>派工</Th><Th center>阶段数</Th><Th right style={{ paddingRight: 18 }}>操作</Th>
              </tr>
            </thead>
            <tbody>
              {rows.map((w) => {
                const editable = w.status === "draft";
                return (
                  <tr key={w.id} style={{ borderTop: "1px solid var(--border)" }} className="hover-row">
                    <Td style={{ paddingLeft: 18 }}><span className="b">{w.name}</span></Td>
                    <Td><span className="mono t2" style={{ fontSize: 12.5 }}>{w.wf_key}</span></Td>
                    <Td><span className="mono" style={{ fontSize: 12.5, fontWeight: 600 }}>v{w.version}</span></Td>
                    <Td><StatusBadge status={w.status} /></Td>
                    <Td><span className="t2">{roleName(w.hub_role)}</span></Td>
                    <Td><Badge tone={w.dispatch_mode === "auto" ? "blue" : "gray"} sm>{w.dispatch_mode === "auto" ? <><I.bolt size={11} /> auto</> : <><I.hand size={11} /> manual</>}</Badge></Td>
                    <Td center><span className="mono t2">{w.stages.length}</span></Td>
                    <Td right style={{ paddingRight: 14 }}>
                      <div className="row gap4" style={{ justifyContent: "flex-end" }}>
                        {canWrite && editable ? (
                          <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => navigate({ page: "workflow-edit", id: w.id })}>编辑</Btn>
                        ) : (
                          <Btn variant="ghost" size="sm" icon={I.eye} onClick={() => navigate({ page: "workflow-edit", id: w.id })}>查看</Btn>
                        )}
                        {canWrite && (
                          <Menu trigger={<IconBtn icon={I.more} title="更多" />} items={[
                            { icon: I.copy, label: "复制为新版本", onClick: () => { dispatch({ type: "clone", id: w.id }); toast("已复制为新草稿版本", "success", `${w.name} v${w.version} → 新草稿`); } },
                            ...(w.status === "draft" ? [{ icon: I.checkCircle, label: "校验并激活", onClick: () => navigate({ page: "workflow-edit", id: w.id, activate: true }) }] : []),
                            ...(w.status === "active" ? [{ icon: I.doc, label: "归档", onClick: () => setConfirm({ type: "archive", w }) }] : []),
                            { icon: I.doc, label: "查看 JSON", onClick: () => setJsonView(w) },
                          ]} />
                        )}
                      </div>
                    </Td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Card>

      <ConfirmDialog open={!!confirm} onClose={() => setConfirm(null)} danger confirmText="确认归档"
        title="归档工作流？" message={confirm && <>归档后 <b>{confirm.w.name} v{confirm.w.version}</b> 将不可被触发，但在跑实例不受影响（已快照）。此操作会写入审计日志。</>}
        onConfirm={() => { dispatch({ type: "archive", id: confirm.w.id }); toast("已归档", "success"); setConfirm(null); }} />

      <Modal open={!!jsonView} onClose={() => setJsonView(null)} title={`工作流 JSON · ${jsonView?.name} v${jsonView?.version}`} width={620} icon={I.doc}>
        <pre className="mono" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 8, padding: 14, fontSize: 11.5, lineHeight: 1.6, overflow: "auto", maxHeight: 460, margin: "4px 0 14px" }}>
{jsonView && JSON.stringify({ wf_key: jsonView.wf_key, name: jsonView.name, version: jsonView.version, status: jsonView.status, hub_role: jsonView.hub_role, dispatch_mode: jsonView.dispatch_mode, trigger_roles: jsonView.trigger_roles, default_gates: jsonView.default_gates, stages: jsonView.stages.map((s) => ({ seq: s.seq, code: s.code, name: s.name, role_code: s.role_code, output_type: s.output_type, is_merge: s.is_merge, deps: s.deps })) }, null, 2)}
        </pre>
      </Modal>

      <NewWorkflowModal open={newOpen} onClose={() => setNewOpen(false)} onCreate={(wf) => { const id = dispatch({ type: "create", wf }); setNewOpen(false); toast("草稿已创建", "success", "进入编辑器配置阶段"); navigate({ page: "workflow-edit", id }); }} />
    </>
  );
}

function NewWorkflowModal({ open, onClose, onCreate }) {
  const [key, setKey] = useState("");
  const [name, setName] = useState("");
  const [hub, setHub] = useState("editor");
  const [mode, setMode] = useState("manual");
  const mgmtRoles = ROLES.filter((r) => r.is_management);
  const valid = key && name;
  return (
    <Modal open={open} onClose={onClose} title="新建工作流" subtitle="创建一个草稿版本，随后进入编辑器配置阶段与依赖" width={460} icon={I.plus}
      footer={<><span /><div className="row gap8"><Btn onClick={onClose}>取消</Btn><Btn variant="primary" disabled={!valid} onClick={() => onCreate({ wf_key: key, name, hub_role: hub, dispatch_mode: mode })}>创建草稿</Btn></div></>}>
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="key（机器键）" hint="创建后只读，建议小写下划线，如 daily_news" required>
          <TextInput value={key} onChange={(v) => setKey(v.replace(/[^a-z0-9_]/g, ""))} placeholder="daily_news" mono />
        </Field>
        <Field label="名称" required><TextInput value={name} onChange={setName} placeholder="资讯日更" /></Field>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
          <Field label="中枢角色" hint="须为管理类角色"><Select value={hub} onChange={setHub} options={mgmtRoles.map((r) => ({ value: r.code, label: r.name }))} /></Field>
          <Field label="派工模式"><Segmented value={mode} onChange={setMode} options={[{ value: "manual", label: "手动" }, { value: "auto", label: "自动" }]} /></Field>
        </div>
      </div>
    </Modal>
  );
}

// table cell helpers
function Th({ children, center, right, style }) {
  return <th style={{ textAlign: center ? "center" : right ? "right" : "left", fontWeight: 550, padding: "10px 12px", ...style }}>{children}</th>;
}
function Td({ children, center, right, style }) {
  return <td style={{ textAlign: center ? "center" : right ? "right" : "left", padding: "11px 12px", verticalAlign: "middle", ...style }}>{children}</td>;
}

Object.assign(window, { LoginPage, Dashboard, WorkflowsList, StatCard, ProgressPill, ActionTag, ACTION_META, Th, Td });
