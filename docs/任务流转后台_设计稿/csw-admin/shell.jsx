// ============================================================
// App shell — Sidebar + Topbar
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

const NAV = [
  { key: "dashboard", label: "仪表盘",     icon: I.dashboard, roles: ["superadmin", "operator", "viewer"] },
  { key: "workflows", label: "工作流",     icon: I.workflow,  roles: ["superadmin", "operator", "viewer"] },
  { key: "members",   label: "角色与成员", icon: I.members,   roles: ["superadmin", "operator", "viewer"] },
  { key: "admin-users", label: "后台用户", icon: I.adminUser, roles: ["superadmin"], badge: "superadmin" },
  { key: "runs",      label: "运行监控",   icon: I.runs,      roles: ["superadmin", "operator", "viewer"] },
  { key: "audit",     label: "审计日志",   icon: I.audit,     roles: ["superadmin", "operator", "viewer"] },
  { key: "settings",  label: "设置",       icon: I.settings,  roles: ["superadmin"], badge: "superadmin" },
];

const ROLE_LABEL = { superadmin: "超级管理员", operator: "运营", viewer: "访客（只读）" };

function Sidebar({ route, navigate, collapsed, setCollapsed, user }) {
  return (
    <aside style={{
      width: collapsed ? 64 : "var(--nav-w)", flexShrink: 0, height: "100%",
      background: "var(--sidebar)", borderRight: "1px solid var(--border)",
      display: "flex", flexDirection: "column", transition: "width .18s",
    }}>
      {/* brand */}
      <div className="row" style={{ height: "var(--top-h)", padding: collapsed ? "0 0" : "0 16px", gap: 10, justifyContent: collapsed ? "center" : "flex-start", borderBottom: "1px solid var(--border)" }}>
        <div style={{ width: 30, height: 30, borderRadius: 8, background: "linear-gradient(150deg,#6d6df0,#5151c4)", display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0, boxShadow: "0 2px 5px rgba(80,80,180,.3)" }}>
          <I.flow size={18} style={{ color: "#fff" }} />
        </div>
        {!collapsed && (
          <div className="col" style={{ gap: 0, lineHeight: 1.2 }}>
            <span style={{ fontWeight: 650, fontSize: 13.5, letterSpacing: "-.01em" }}>营事编集室</span>
            <span className="t3" style={{ fontSize: 11 }}>任务流转后台</span>
          </div>
        )}
      </div>

      {/* nav */}
      <nav className="col" style={{ gap: 2, padding: collapsed ? "10px 8px" : "10px 12px", flex: 1, overflowY: "auto" }}>
        {!collapsed && <div className="t3 xs b" style={{ padding: "6px 8px 4px", letterSpacing: ".04em", textTransform: "uppercase", fontSize: 10.5 }}>导航</div>}
        {NAV.map((n) => {
          const visible = n.roles.includes(user.role);
          const on = route.page === n.key;
          const locked = !visible;
          return (
            <button key={n.key} disabled={locked} onClick={() => !locked && navigate({ page: n.key })} title={collapsed ? n.label : undefined}
              style={{
                display: "flex", alignItems: "center", gap: 10, padding: collapsed ? "9px" : "7px 9px",
                justifyContent: collapsed ? "center" : "flex-start",
                borderRadius: 7, border: "none", background: on ? "var(--accent-weak)" : "transparent",
                color: locked ? "var(--text-3)" : on ? "var(--accent-text)" : "var(--text-2)",
                fontSize: 13.5, fontWeight: on ? 600 : 500, opacity: locked ? 0.5 : 1,
                cursor: locked ? "not-allowed" : "pointer", transition: "background .12s, color .12s", position: "relative",
              }}
              onMouseEnter={(e) => { if (!on && !locked) e.currentTarget.style.background = "var(--surface-2)"; }}
              onMouseLeave={(e) => { if (!on) e.currentTarget.style.background = "transparent"; }}>
              {on && !collapsed && <span style={{ position: "absolute", left: -12, top: 8, bottom: 8, width: 3, borderRadius: 3, background: "var(--accent)" }} />}
              <n.icon size={18} />
              {!collapsed && <span className="grow" style={{ textAlign: "left" }}>{n.label}</span>}
              {!collapsed && locked && <I.lock size={13} />}
            </button>
          );
        })}
      </nav>

      {/* collapse toggle */}
      <div style={{ padding: collapsed ? 8 : 12, borderTop: "1px solid var(--border)" }}>
        <button onClick={() => setCollapsed((c) => !c)}
          style={{ display: "flex", alignItems: "center", gap: 8, width: "100%", justifyContent: collapsed ? "center" : "flex-start", padding: "7px 9px", borderRadius: 7, border: "none", background: "transparent", color: "var(--text-3)", fontSize: 12.5 }}
          onMouseEnter={(e) => e.currentTarget.style.background = "var(--surface-2)"}
          onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}>
          <span style={{ transform: collapsed ? "none" : "rotate(180deg)", display: "flex", transition: "transform .2s" }}><I.chevRight size={16} /></span>
          {!collapsed && <span>收起侧栏</span>}
        </button>
      </div>
    </aside>
  );
}

function Topbar({ route, user, onLogout, onSwitchRole }) {
  const titles = {
    dashboard: "仪表盘", workflows: "工作流", "workflow-edit": "工作流编辑器",
    members: "角色与成员", "admin-users": "后台用户", runs: "运行监控",
    "run-detail": "运行详情", audit: "审计日志", settings: "设置",
  };
  return (
    <header className="row between" style={{
      height: "var(--top-h)", flexShrink: 0, padding: "0 22px",
      borderBottom: "1px solid var(--border)", background: "rgba(255,255,255,.8)",
      backdropFilter: "blur(8px)", position: "sticky", top: 0, zIndex: 40,
    }}>
      <div className="row gap8">
        <span className="t2" style={{ fontSize: 13.5, fontWeight: 550 }}>{titles[route.page] || "后台"}</span>
      </div>
      <div className="row gap16">
        <span className="row gap6" style={{ fontSize: 12, color: "var(--text-2)", padding: "3px 9px", background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 999 }}>
          <span style={{ width: 6, height: 6, borderRadius: 999, background: "var(--green)" }} />
          环境 · prod
        </span>
        <Menu align="right" width={216} trigger={
          <button className="row gap8" style={{ border: "none", background: "transparent", padding: "4px 6px", borderRadius: 8 }}
            onMouseEnter={(e) => e.currentTarget.style.background = "var(--surface-2)"} onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}>
            <Avatar name={user.display_name} size={28} />
            <div className="col" style={{ gap: 0, alignItems: "flex-start", lineHeight: 1.2 }}>
              <span style={{ fontSize: 13, fontWeight: 600 }}>{user.display_name}</span>
              <span className="t3" style={{ fontSize: 11 }}>{ROLE_LABEL[user.role]}</span>
            </div>
            <I.chevDown size={15} style={{ color: "var(--text-3)" }} />
          </button>
        } items={[
          { icon: I.user, label: "我的资料", onClick: () => toast("我的资料（演示）") },
          { divider: true },
          { icon: I.adminUser, label: "切到 超级管理员", onClick: () => onSwitchRole("superadmin") },
          { icon: I.user, label: "切到 运营 (operator)", onClick: () => onSwitchRole("operator") },
          { icon: I.eye, label: "切到 访客 (viewer)", onClick: () => onSwitchRole("viewer") },
          { divider: true },
          { icon: I.logout, label: "退出登录", danger: true, onClick: onLogout },
        ]} />
      </div>
    </header>
  );
}

// content container with max width
function Content({ children, wide }) {
  return (
    <div style={{ flex: 1, overflowY: "auto", background: "var(--bg)" }}>
      <div style={{ maxWidth: wide ? 1440 : 1200, margin: "0 auto", padding: "26px 32px 60px" }}>
        {children}
      </div>
    </div>
  );
}

Object.assign(window, { Sidebar, Topbar, Content, NAV, ROLE_LABEL });
