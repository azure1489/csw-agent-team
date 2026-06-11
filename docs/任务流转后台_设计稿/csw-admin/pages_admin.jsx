// ============================================================
// Pages: Members (roles + agents/tokens) · Admin users
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

function genToken() {
  const chars = "abcdef0123456789";
  let s = ""; for (let i = 0; i < 24; i++) s += chars[Math.floor(Math.random() * chars.length)];
  return "csw_live_" + s.slice(0, 8) + "…" + s.slice(-3);
}

function Members({ user, state, dispatch }) {
  const canWrite = user.role !== "viewer";
  const [tab, setTab] = useState("agents");
  const [detail, setDetail] = useState(null);       // agent id
  const [issueFor, setIssueFor] = useState(null);    // agent
  const [confirm, setConfirm] = useState(null);
  const [newRole, setNewRole] = useState(false);
  const [newAgent, setNewAgent] = useState(false);

  const memberCount = (code) => state.agents.filter((a) => a.role_code === code).length;
  const roleName = (c) => (ROLES.find((r) => r.code === c) || {}).name || c;
  const detailAgent = state.agents.find((a) => a.id === detail);

  return (
    <>
      <PageHeader title="角色与成员" desc="角色定义谁能做什么（管理 / 人工）；成员是具体 agent，bearer token 由后台签发 / 轮换 / 吊销。"
        actions={canWrite && (tab === "roles" ? <Btn variant="primary" icon={I.plus} onClick={() => setNewRole(true)}>新建角色</Btn> : <Btn variant="primary" icon={I.plus} onClick={() => setNewAgent(true)}>新建成员</Btn>)} />

      <div style={{ marginBottom: 16 }}>
        <Tabs tabs={[{ value: "agents", label: `成员 / Token (${state.agents.length})` }, { value: "roles", label: `角色 (${ROLES.length})` }]} value={tab} onChange={setTab} />
      </div>

      {tab === "roles" ? (
        <Card noPad>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
            <thead><tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
              <Th style={{ paddingLeft: 18 }}>code</Th><Th>名称</Th><Th center>管理类</Th><Th center>人工</Th><Th center>成员数</Th><Th right style={{ paddingRight: 18 }}></Th>
            </tr></thead>
            <tbody>
              {ROLES.map((r) => (
                <tr key={r.code} className="hover-row" style={{ borderTop: "1px solid var(--border)" }}>
                  <Td style={{ paddingLeft: 18 }}><span className="mono t2" style={{ fontSize: 12.5 }}>{r.code}</span></Td>
                  <Td><span className="b">{r.name}</span></Td>
                  <Td center>{r.is_management ? <I.check size={16} style={{ color: "var(--green)" }} /> : <span className="t3">—</span>}</Td>
                  <Td center>{r.is_human ? <Badge tone="amber" sm>人工</Badge> : <span className="t3">—</span>}</Td>
                  <Td center><span className="mono t2">{memberCount(r.code)}</span></Td>
                  <Td right style={{ paddingRight: 14 }}>{canWrite && <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => toast("编辑角色（演示）")}>编辑</Btn>}</Td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="t3 xs" style={{ padding: "10px 18px", borderTop: "1px solid var(--border)" }}>管理类 → 可任中枢 / 审核 / 管理工作流定义；人工 → 无 token、结论由中枢代录。被工作流 / agent 引用的角色不可删。</div>
        </Card>
      ) : (
        <Card noPad>
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
            <thead><tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
              <Th style={{ paddingLeft: 18 }}>名称</Th><Th>角色</Th><Th>有效 token</Th><Th>状态</Th><Th right style={{ paddingRight: 18 }}>操作</Th>
            </tr></thead>
            <tbody>
              {state.agents.map((a) => {
                const valid = a.tokens.filter((t) => t.status === "active").length;
                const human = ROLES.find((r) => r.code === a.role_code)?.is_human;
                return (
                  <tr key={a.id} className="hover-row" style={{ borderTop: "1px solid var(--border)" }}>
                    <Td style={{ paddingLeft: 18 }}><span className="row gap8"><Avatar name={a.name} size={24} /><span className="b">{a.name}</span></span></Td>
                    <Td><span className="t2">{roleName(a.role_code)}</span> <span className="mono t3 xs">{a.role_code}</span></Td>
                    <Td>{human ? <span className="t3 sm">无 token（结论代录）</span> : <span className="row gap6"><span className="mono b">{valid}</span><span className="t3 sm">枚{a.tokens.length > valid ? `（共 ${a.tokens.length}，轮换）` : ""}</span></span>}</Td>
                    <Td><StatusBadge status={a.active ? "enabled" : "disabled"} /></Td>
                    <Td right style={{ paddingRight: 14 }}>
                      <div className="row gap4" style={{ justifyContent: "flex-end" }}>
                        <Btn variant="ghost" size="sm" onClick={() => setDetail(a.id)}>详情</Btn>
                        {canWrite && !human && (
                          <Menu trigger={<IconBtn icon={I.more} />} items={[
                            { icon: I.key, label: "签发新 token", onClick: () => setIssueFor(a) },
                            { icon: a.active ? I.lock : I.check, label: a.active ? "禁用成员" : "启用成员", danger: a.active, onClick: () => a.active ? setConfirm({ type: "disable-agent", a }) : (dispatch({ type: "toggle-agent", id: a.id }), toast("已启用成员", "success")) },
                          ]} />
                        )}
                      </div>
                    </Td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </Card>
      )}

      {/* agent detail drawer */}
      <Drawer open={!!detailAgent} onClose={() => setDetail(null)} width={560}
        title={detailAgent ? `成员：${detailAgent.name}` : ""} subtitle={detailAgent ? `${roleName(detailAgent.role_code)} · ${detailAgent.role_code}` : ""}
        badge={detailAgent && <StatusBadge status={detailAgent.active ? "enabled" : "disabled"} sm />}>
        {detailAgent && (
          <div className="col gap24">
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
              <Field label="名称"><TextInput value={detailAgent.name} readOnly={!canWrite} onChange={(v) => dispatch({ type: "rename-agent", id: detailAgent.id, name: v })} /></Field>
              <Field label="角色"><Select value={detailAgent.role_code} disabled={!canWrite} onChange={(v) => dispatch({ type: "agent-role", id: detailAgent.id, role: v })} options={ROLES.filter((r) => r.code !== "scheduler" || detailAgent.role_code === "scheduler").map((r) => ({ value: r.code, label: r.name }))} /></Field>
            </div>
            {ROLES.find((r) => r.code === detailAgent.role_code)?.is_human ? (
              <div className="row gap8" style={{ background: "var(--amber-bg)", border: "1px solid var(--amber-bd)", color: "var(--amber)", padding: "10px 13px", borderRadius: 8, fontSize: 12.5 }}>
                <I.hand size={15} />人工角色不签发 token——其审核结论由中枢（主编）代录。
              </div>
            ) : (
              <div className="col gap10">
                <div className="row between"><span className="b" style={{ fontSize: 13.5 }}>Token</span>{canWrite && <Btn variant="default" size="sm" icon={I.key} onClick={() => setIssueFor(detailAgent)}>签发新 token</Btn>}</div>
                <div className="col gap8">
                  {detailAgent.tokens.length === 0 && <span className="t3 sm">暂无 token</span>}
                  {detailAgent.tokens.map((t) => (
                    <div key={t.id} className="row between" style={{ border: "1px solid var(--border)", borderRadius: 8, padding: "10px 13px", opacity: t.status === "revoked" ? 0.62 : 1 }}>
                      <div className="col" style={{ gap: 3 }}>
                        <div className="row gap8">
                          <span className="mono b sm" style={{ textDecoration: t.status === "revoked" ? "line-through" : "none" }}>#{t.id} {t.label}</span>
                          <StatusBadge status={t.status === "active" ? "token_active" : t.status} sm />
                        </div>
                        <span className="t3 xs">最近使用 {t.last_used_at} · 过期 {t.expires_at}</span>
                      </div>
                      {canWrite && t.status === "active" && <Btn variant="ghost" size="sm" danger onClick={() => setConfirm({ type: "revoke", a: detailAgent, t })} style={{ color: "var(--red)" }}>吊销</Btn>}
                    </div>
                  ))}
                </div>
                <div className="t3 xs">轮换 = 先签发新 token，确认生效后再吊销旧的。吊销即时失效该 token 的运行面调用。</div>
              </div>
            )}
          </div>
        )}
      </Drawer>

      <IssueTokenModal agent={issueFor} onClose={() => setIssueFor(null)} onIssue={(label, exp) => { dispatch({ type: "issue-token", id: issueFor.id, label, exp }); }} />

      <ConfirmDialog open={confirm?.type === "revoke"} onClose={() => setConfirm(null)} danger confirmText="确认吊销"
        title="吊销 token？" message={confirm?.t && <>吊销 <b>#{confirm.t.id} {confirm.t.label}</b> 后，使用该 token 的运行面调用将<b>即时失效</b>。此操作不可恢复，会写入审计日志。</>}
        onConfirm={() => { dispatch({ type: "revoke-token", aid: confirm.a.id, tid: confirm.t.id }); toast("token 已吊销", "success"); setConfirm(null); }} />
      <ConfirmDialog open={confirm?.type === "disable-agent"} onClose={() => setConfirm(null)} danger confirmText="确认禁用"
        title="禁用成员？" message={confirm?.a && <>禁用 <b>{confirm.a.name}</b> 后，其<b>所有 token 立即失效</b>，运行面将拒绝其调用。可随时重新启用。</>}
        onConfirm={() => { dispatch({ type: "toggle-agent", id: confirm.a.id }); toast("成员已禁用", "success"); setConfirm(null); }} />

      <NewRoleModal open={newRole} onClose={() => setNewRole(false)} onCreate={() => { setNewRole(false); toast("角色已创建（演示）", "success"); }} />
      <NewAgentModal open={newAgent} onClose={() => setNewAgent(false)} onCreate={(name, role) => { dispatch({ type: "new-agent", name, role }); setNewAgent(false); toast("成员已创建", "success"); }} />
    </>
  );
}

function IssueTokenModal({ agent, onClose, onIssue }) {
  const [label, setLabel] = useState("生产");
  const [exp, setExp] = useState("永不");
  const [issued, setIssued] = useState(null);
  useEffect(() => { if (agent) { setLabel("生产"); setExp("永不"); setIssued(null); } }, [agent]);
  if (!agent) return null;
  const doIssue = () => { const plain = "csw_live_" + Math.random().toString(36).slice(2, 10) + "…" + Math.random().toString(36).slice(2, 5); setIssued(plain); onIssue(label, exp); };
  return (
    <Modal open={!!agent} onClose={onClose} width={460} icon={I.key} title={`签发 token：${agent.name}`} subtitle="明文仅显示一次，关闭后服务器只存 sha256 哈希"
      footer={issued ? <><span /><Btn variant="primary" onClick={onClose}>完成</Btn></> : <><Btn onClick={onClose}>取消</Btn><Btn variant="primary" icon={I.key} onClick={doIssue}>签发</Btn></>}>
      {!issued ? (
        <div className="col gap16" style={{ paddingBottom: 8 }}>
          <Field label="备注 label" hint="便于识别用途，如「生产」「轮换-2026Q2」"><TextInput value={label} onChange={setLabel} /></Field>
          <Field label="过期"><Select value={exp} onChange={setExp} options={["永不", "30 天", "90 天", "自定义…"]} /></Field>
        </div>
      ) : (
        <div className="col gap14" style={{ paddingBottom: 8 }}>
          <div className="row gap8" style={{ background: "var(--amber-bg)", border: "1px solid var(--amber-bd)", color: "var(--amber)", padding: "10px 13px", borderRadius: 8, fontSize: 12.5 }}>
            <I.warn size={16} /><span>明文仅显示一次。关闭后无法再查看，请立即复制保存。</span>
          </div>
          <div className="row between" style={{ background: "#1f1f27", borderRadius: 9, padding: "13px 15px" }}>
            <span className="mono" style={{ color: "#9ef0c3", fontSize: 13.5, wordBreak: "break-all" }}>{issued}</span>
            <Btn variant="default" size="sm" icon={I.copy} onClick={() => { navigator.clipboard?.writeText(issued); toast("已复制到剪贴板", "success"); }}>复制</Btn>
          </div>
          <div className="t3 xs">备注「{label}」 · 过期 {exp} · 服务器已保存哈希。</div>
        </div>
      )}
    </Modal>
  );
}

function NewRoleModal({ open, onClose, onCreate }) {
  const [code, setCode] = useState(""); const [name, setName] = useState("");
  const [mgmt, setMgmt] = useState(false); const [human, setHuman] = useState(false);
  return (
    <Modal open={open} onClose={onClose} width={440} icon={I.plus} title="新建角色"
      footer={<><Btn onClick={onClose}>取消</Btn><Btn variant="primary" disabled={!code || !name} onClick={onCreate}>创建</Btn></>}>
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
          <Field label="code" hint="创建后只读"><TextInput value={code} onChange={(v) => setCode(v.replace(/[^a-z0-9_]/g, ""))} mono placeholder="reviewer" /></Field>
          <Field label="名称"><TextInput value={name} onChange={setName} placeholder="审核员" /></Field>
        </div>
        <div className="col gap10" style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 12 }}>
          <Checkbox checked={mgmt} onChange={setMgmt} label="管理类（可任中枢 / 审核 / 管理定义）" />
          <Checkbox checked={human} onChange={setHuman} label="人工角色（无 token，结论由中枢代录）" />
        </div>
      </div>
    </Modal>
  );
}

function NewAgentModal({ open, onClose, onCreate }) {
  const [name, setName] = useState(""); const [role, setRole] = useState("collector");
  return (
    <Modal open={open} onClose={onClose} width={420} icon={I.plus} title="新建成员"
      footer={<><Btn onClick={onClose}>取消</Btn><Btn variant="primary" disabled={!name} onClick={() => onCreate(name, role)}>创建</Btn></>}>
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="名称"><TextInput value={name} onChange={setName} placeholder="如 文案bot" /></Field>
        <Field label="角色"><Select value={role} onChange={setRole} options={ROLES.map((r) => ({ value: r.code, label: `${r.name}（${r.code}）` }))} /></Field>
      </div>
    </Modal>
  );
}

// ---------------- ADMIN USERS ----------------
function AdminUsers({ user, state, dispatch }) {
  const [confirm, setConfirm] = useState(null);
  const [reset, setReset] = useState(null);
  const [newUser, setNewUser] = useState(false);
  return (
    <>
      <PageHeader title="后台用户" desc="管理后台登录账号（JWT 登录，与运行面 agent token 完全分离）。仅超级管理员可访问。"
        badge={<Badge tone="violet" sm><I.lock size={11} />superadmin</Badge>}
        actions={<Btn variant="primary" icon={I.plus} onClick={() => setNewUser(true)}>新建用户</Btn>} />
      <Card noPad>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
          <thead><tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
            <Th style={{ paddingLeft: 18 }}>用户名</Th><Th>名称</Th><Th>角色</Th><Th>状态</Th><Th>最近登录</Th><Th right style={{ paddingRight: 18 }}>操作</Th>
          </tr></thead>
          <tbody>
            {state.users.map((u) => {
              const self = u.username === user.username;
              return (
                <tr key={u.id} className="hover-row" style={{ borderTop: "1px solid var(--border)" }}>
                  <Td style={{ paddingLeft: 18 }}><span className="row gap8"><Avatar name={u.display_name} size={24} /><span className="mono b">{u.username}</span>{self && <Badge tone="gray" sm>我</Badge>}</span></Td>
                  <Td>{u.display_name}</Td>
                  <Td><Badge tone={u.role === "superadmin" ? "violet" : u.role === "operator" ? "blue" : "gray"} sm>{ROLE_LABEL[u.role]}</Badge></Td>
                  <Td><StatusBadge status={u.status === "active" ? "enabled" : "disabled"} /></Td>
                  <Td><span className="t3 mono sm">{u.last_login_at}</span></Td>
                  <Td right style={{ paddingRight: 14 }}>
                    <Menu trigger={<IconBtn icon={I.more} />} items={[
                      { icon: I.edit, label: "编辑", onClick: () => toast("编辑用户（演示）") },
                      { icon: I.key, label: "重置密码", onClick: () => setReset(u) },
                      { divider: true },
                      { icon: u.status === "active" ? I.lock : I.check, label: u.status === "active" ? "禁用" : "启用", danger: u.status === "active", onClick: () => self ? toast("不能禁用 / 降级自己（防自锁）", "error") : (u.status === "active" ? setConfirm(u) : (dispatch({ type: "toggle-user", id: u.id }), toast("已启用", "success"))) },
                    ]} />
                  </Td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <div className="t3 xs" style={{ padding: "10px 18px", borderTop: "1px solid var(--border)" }}>安全约束：不能禁用 / 降级自己（防自锁）；系统至少保留一个超级管理员；禁用即时失效其 refresh token。</div>
      </Card>

      <ConfirmDialog open={!!confirm} onClose={() => setConfirm(null)} danger confirmText="确认禁用"
        title="禁用后台用户？" message={confirm && <>禁用 <b>{confirm.display_name}（{confirm.username}）</b> 后，其 refresh token 立即失效，将无法登录后台。</>}
        onConfirm={() => { dispatch({ type: "toggle-user", id: confirm.id }); toast("用户已禁用", "success"); setConfirm(null); }} />
      <ResetPasswordModal u={reset} onClose={() => setReset(null)} />
      <NewUserModal open={newUser} onClose={() => setNewUser(false)} onCreate={(u) => { dispatch({ type: "new-user", u }); setNewUser(false); toast("用户已创建", "success", "已设初始密码，强制首次修改"); }} />
    </>
  );
}

function ResetPasswordModal({ u, onClose }) {
  const [done, setDone] = useState(false);
  const temp = useMemo(() => "Tmp-" + Math.random().toString(36).slice(2, 8) + "!", [u]);
  useEffect(() => { setDone(false); }, [u]);
  if (!u) return null;
  return (
    <Modal open={!!u} onClose={onClose} width={440} icon={I.key} title={`重置密码：${u.display_name}`} subtitle={`@${u.username}`}
      footer={done ? <><span /><Btn variant="primary" onClick={onClose}>完成</Btn></> : <><Btn onClick={onClose}>取消</Btn><Btn variant="primary" onClick={() => setDone(true)}>生成临时密码</Btn></>}>
      {!done ? <div className="t2 sm" style={{ paddingBottom: 10, lineHeight: 1.6 }}>将为该用户生成一个临时密码，用户首次登录后强制修改。也可选择发送重置链接（演示从略）。</div> : (
        <div className="col gap12" style={{ paddingBottom: 8 }}>
          <div className="row between" style={{ background: "#1f1f27", borderRadius: 9, padding: "13px 15px" }}>
            <span className="mono" style={{ color: "#9ef0c3", fontSize: 14 }}>{temp}</span>
            <Btn variant="default" size="sm" icon={I.copy} onClick={() => { navigator.clipboard?.writeText(temp); toast("已复制", "success"); }}>复制</Btn>
          </div>
          <div className="t3 xs">请通过安全渠道转交给用户；首次登录将强制修改密码。</div>
        </div>
      )}
    </Modal>
  );
}

function NewUserModal({ open, onClose, onCreate }) {
  const [username, setUsername] = useState(""); const [name, setName] = useState(""); const [role, setRole] = useState("operator");
  return (
    <Modal open={open} onClose={onClose} width={440} icon={I.plus} title="新建后台用户"
      footer={<><Btn onClick={onClose}>取消</Btn><Btn variant="primary" disabled={!username || !name} onClick={() => onCreate({ username, display_name: name, role })}>创建</Btn></>}>
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
          <Field label="用户名" hint="唯一，创建后只读"><TextInput value={username} onChange={(v) => setUsername(v.replace(/[^a-z0-9_]/g, ""))} mono /></Field>
          <Field label="名称"><TextInput value={name} onChange={setName} placeholder="张管理" /></Field>
        </div>
        <Field label="角色"><Select value={role} onChange={setRole} options={[{ value: "superadmin", label: "超级管理员" }, { value: "operator", label: "运营 operator" }, { value: "viewer", label: "访客 viewer（只读）" }]} /></Field>
        <div className="t3 xs" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 7, padding: "9px 11px", lineHeight: 1.6 }}>创建后将设置初始密码（强制首次修改）。密码需满足强度要求。</div>
      </div>
    </Modal>
  );
}

Object.assign(window, { Members, AdminUsers });
