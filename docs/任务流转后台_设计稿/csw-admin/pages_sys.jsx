// ============================================================
// Pages: Audit log · Settings
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

function Audit({ user }) {
  const [fUser, setFUser] = useState("");
  const [fAction, setFAction] = useState("");
  const [fDate, setFDate] = useState("");
  const [detail, setDetail] = useState(null);
  const rows = AUDIT
    .filter((a) => !fUser || a.user === fUser)
    .filter((a) => !fAction || a.action === fAction)
    .filter((a) => !fDate || a.t.startsWith(fDate));
  const users = [...new Set(AUDIT.map((a) => a.user))];
  const actions = [...new Set(AUDIT.map((a) => a.action))];
  return (
    <>
      <PageHeader title="审计日志" desc="后台操作留痕：登录、定义增改、激活、token 签发 / 吊销、用户管理。只读、可筛选、可导出。"
        actions={<Btn variant="default" icon={I.download} onClick={() => toast("已导出 audit.csv（演示）", "success")}>导出</Btn>} />
      <Card noPad>
        <div className="row gap12" style={{ padding: "12px 14px", borderBottom: "1px solid var(--border)", flexWrap: "wrap" }}>
          <div style={{ width: 130 }}><Select value={fUser} onChange={setFUser} placeholder="全部用户" options={[{ value: "", label: "全部用户" }, ...users.map((u) => ({ value: u, label: u }))]} sm /></div>
          <div style={{ width: 160 }}><Select value={fAction} onChange={setFAction} placeholder="全部动作" options={[{ value: "", label: "全部动作" }, ...actions.map((a) => ({ value: a, label: (ACTION_META[a] || {}).label || a }))]} sm /></div>
          <div style={{ width: 130 }}><TextInput value={fDate} onChange={setFDate} placeholder="2026-06-09" icon={I.clock} style={{ padding: "5px 10px 5px 32px", fontSize: 12.5 }} /></div>
          {(fUser || fAction || fDate) && <Btn variant="ghost" size="sm" onClick={() => { setFUser(""); setFAction(""); setFDate(""); }}>清除</Btn>}
          <span className="grow" /><span className="t3 sm">{rows.length} 条记录</span>
        </div>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
          <thead><tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
            <Th style={{ paddingLeft: 18 }}>时间</Th><Th>用户</Th><Th>动作</Th><Th>目标</Th><Th right style={{ paddingRight: 18 }}>详情</Th>
          </tr></thead>
          <tbody>
            {rows.map((a, i) => (
              <tr key={i} className="hover-row" style={{ borderTop: "1px solid var(--border)" }}>
                <Td style={{ paddingLeft: 18 }}><span className="t2 mono sm">{a.t}</span></Td>
                <Td><span className="row gap6"><Avatar name={a.user} size={20} /><span className="b sm">{a.user}</span></span></Td>
                <Td><ActionTag action={a.action} /></Td>
                <Td><span className="mono t2 sm">{a.target}</span></Td>
                <Td right style={{ paddingRight: 14 }}><IconBtn icon={I.eye} title="查看详情" onClick={() => setDetail(a)} /></Td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <EmptyState icon={I.audit} title="没有匹配的记录" desc="调整筛选条件" compact />}
      </Card>

      <Modal open={!!detail} onClose={() => setDetail(null)} width={460} icon={I.audit} title="审计详情"
        subtitle={detail ? `${detail.t} · ${detail.user}` : ""}>
        {detail && (
          <div className="col gap14" style={{ paddingBottom: 14 }}>
            <div className="row gap8"><ActionTag action={detail.action} /><span className="mono t2 sm">{detail.target}</span></div>
            <div>
              <div className="t3 xs b" style={{ marginBottom: 6, textTransform: "uppercase", letterSpacing: ".03em" }}>detail_json</div>
              <pre className="mono" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 8, padding: 13, fontSize: 12, lineHeight: 1.6, margin: 0, overflow: "auto" }}>
{JSON.stringify(detail.detail, null, 2)}
              </pre>
            </div>
          </div>
        )}
      </Modal>
    </>
  );
}

// ---------------- SETTINGS ----------------
function SettingRow({ label, hint, children, last }) {
  return (
    <div className="row between" style={{ padding: "13px 0", borderBottom: last ? "none" : "1px solid var(--border)", gap: 20, alignItems: "center" }}>
      <div className="col" style={{ gap: 2, maxWidth: 420 }}>
        <span style={{ fontSize: 13.5, fontWeight: 550 }}>{label}</span>
        {hint && <span className="t3 xs" style={{ lineHeight: 1.5 }}>{hint}</span>}
      </div>
      <div style={{ flexShrink: 0 }}>{children}</div>
    </div>
  );
}

function Settings() {
  const [maxSize, setMaxSize] = useState("200");
  const [backend, setBackend] = useState("local");
  const [defTrigger, setDefTrigger] = useState(["editor", "scheduler"]);
  const toggleDt = (c) => setDefTrigger((x) => x.includes(c) ? x.filter((y) => y !== c) : [...x, c]);
  return (
    <>
      <PageHeader title="设置" desc="系统级配置。多数为只读展示，少量可改（改动会写入审计日志）。仅超级管理员可访问。"
        badge={<Badge tone="violet" sm><I.lock size={11} />superadmin</Badge>} />
      <div className="col gap16">
        <SectionCard title="JWT 会话" subtitle="access 无状态短期，refresh 存哈希、支持轮换 / 吊销">
          <SettingRow label="Access Token TTL" hint="无状态短期令牌，每请求验签不查库"><span className="mono t2" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 6, padding: "4px 10px", fontSize: 12.5 }}>15 min</span></SettingRow>
          <SettingRow label="Refresh Token TTL" hint="改动需重启服务，请谨慎" last><span className="mono t2" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 6, padding: "4px 10px", fontSize: 12.5 }}>7 d</span></SettingRow>
        </SectionCard>

        <SectionCard title="文件存储" actions={<Btn variant="default" size="sm" onClick={() => toast("文件设置已保存", "success")}>保存</Btn>}>
          <SettingRow label="单文件大小上限" hint="超过将拒绝上传"><div className="row gap6"><div style={{ width: 90 }}><TextInput value={maxSize} onChange={setMaxSize} mono style={{ textAlign: "right", padding: "6px 10px" }} /></div><span className="t2 sm">MB</span></div></SettingRow>
          <SettingRow label="允许类型" hint="content_type 白名单"><div className="row gap6"><Badge tone="gray" sm>zip</Badge><Badge tone="gray" sm>pdf</Badge><Badge tone="gray" sm>png</Badge></div></SettingRow>
          <SettingRow label="存储后端" hint="内容寻址：data/blobs/<分片>/<sha256>；BlobStore 接口预留 OSS" last><div style={{ width: 160 }}><Select value={backend} onChange={setBackend} options={[{ value: "local", label: "本地磁盘 local" }, { value: "oss", label: "对象存储 OSS" }]} sm /></div></SettingRow>
        </SectionCard>

        <SectionCard title="触发默认" subtitle="新建工作流时默认可触发的角色">
          <div className="row wrap gap8" style={{ paddingTop: 2 }}>
            {ROLES.filter((r) => r.is_management || r.code === "scheduler").map((r) => {
              const on = defTrigger.includes(r.code);
              return (
                <button key={r.code} onClick={() => toggleDt(r.code)}
                  style={{ display: "flex", alignItems: "center", gap: 7, padding: "6px 12px", borderRadius: 999, fontSize: 12.5, fontWeight: 540,
                    border: `1px solid ${on ? "var(--accent)" : "var(--border-strong)"}`, background: on ? "var(--accent-weak)" : "var(--surface)", color: on ? "var(--accent-text)" : "var(--text-2)" }}>
                  {on ? <I.check size={13} /> : <I.plus size={13} />}{r.name}
                </button>
              );
            })}
          </div>
        </SectionCard>

        <SectionCard title="关于" subtitle="版本与健康检查">
          <SettingRow label="服务版本"><span className="mono t2 sm">csw-task-svc v0.4.1</span></SettingRow>
          <SettingRow label="数据库"><span className="mono t2 sm">SQLite · WAL · data/csw.db</span></SettingRow>
          <SettingRow label="健康检查" last><span className="row gap6"><span style={{ width: 8, height: 8, borderRadius: 999, background: "var(--green)" }} /><span className="sm" style={{ color: "var(--green)" }}>运行面 + 后台 API 正常</span></span></SettingRow>
        </SectionCard>
      </div>
    </>
  );
}

Object.assign(window, { Audit, Settings, SettingRow });
