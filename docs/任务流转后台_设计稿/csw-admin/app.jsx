// ============================================================
// App — store, routing, role switching, mount
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

function useStore() {
  const [state, setState] = useState(() => ({
    workflows: JSON.parse(JSON.stringify(WORKFLOWS)),
    agents: JSON.parse(JSON.stringify(AGENTS)),
    users: JSON.parse(JSON.stringify(ADMIN_USERS)),
  }));
  const idRef = useRef(100);

  const dispatch = useCallback((action) => {
    switch (action.type) {
      case "create": {
        const id = ++idRef.current;
        setState((p) => ({ ...p, workflows: [{
          id, wf_key: action.wf.wf_key, name: action.wf.name, version: 1, status: "draft",
          hub_role: action.wf.hub_role, dispatch_mode: action.wf.dispatch_mode, trigger_roles: ["editor", "scheduler"],
          common_instructions: "", common_acceptance: "",
          default_gates: [ { gate_order: 1, reviewer_role: "editor", relayed: false, name: "主编审" }, { gate_order: 2, reviewer_role: "van", relayed: true, name: "Van审" } ],
          stages: [{ id: id * 10 + 1, seq: 1, code: "stage_1", name: "01-阶段", role_code: "collector", output_type: "资讯包", is_merge: false, deps: [], instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null }],
          created_at: "刚刚",
        }, ...p.workflows] }));
        return id;
      }
      case "clone": {
        const id = ++idRef.current;
        setState((p) => {
          const src = p.workflows.find((w) => w.id === action.id);
          const maxV = Math.max(...p.workflows.filter((w) => w.wf_key === src.wf_key).map((w) => w.version));
          const copy = JSON.parse(JSON.stringify(src));
          copy.id = id; copy.version = maxV + 1; copy.status = "draft"; copy.created_at = "刚刚";
          return { ...p, workflows: [copy, ...p.workflows] };
        });
        return id;
      }
      case "archive":
        setState((p) => ({ ...p, workflows: p.workflows.map((w) => w.id === action.id ? { ...w, status: "archived" } : w) }));
        break;
      case "save":
        setState((p) => ({ ...p, workflows: p.workflows.map((w) => w.id === action.id ? { ...action.draft } : w) }));
        break;
      case "activate":
        setState((p) => ({ ...p, workflows: p.workflows.map((w) => {
          if (w.id === action.id) return { ...action.draft, status: "active" };
          if (w.wf_key === action.draft.wf_key && w.status === "active") return { ...w, status: "archived" };
          return w;
        }) }));
        break;
      case "issue-token":
        setState((p) => ({ ...p, agents: p.agents.map((a) => a.id === action.id ? { ...a, tokens: [{ id: Math.floor(Math.random() * 900 + 100), label: action.label, status: "active", last_used_at: "—", expires_at: action.exp, created_at: "刚刚" }, ...a.tokens] } : a) }));
        break;
      case "revoke-token":
        setState((p) => ({ ...p, agents: p.agents.map((a) => a.id === action.aid ? { ...a, tokens: a.tokens.map((t) => t.id === action.tid ? { ...t, status: "revoked" } : t) } : a) }));
        break;
      case "toggle-agent":
        setState((p) => ({ ...p, agents: p.agents.map((a) => a.id === action.id ? { ...a, active: !a.active } : a) }));
        break;
      case "rename-agent":
        setState((p) => ({ ...p, agents: p.agents.map((a) => a.id === action.id ? { ...a, name: action.name } : a) }));
        break;
      case "agent-role":
        setState((p) => ({ ...p, agents: p.agents.map((a) => a.id === action.id ? { ...a, role_code: action.role } : a) }));
        break;
      case "new-agent": {
        const id = ++idRef.current;
        setState((p) => ({ ...p, agents: [...p.agents, { id, name: action.name, role_code: action.role, active: true, tokens: [] }] }));
        break;
      }
      case "toggle-user":
        setState((p) => ({ ...p, users: p.users.map((u) => u.id === action.id ? { ...u, status: u.status === "active" ? "disabled" : "active" } : u) }));
        break;
      case "new-user": {
        const id = ++idRef.current;
        setState((p) => ({ ...p, users: [...p.users, { id, ...action.u, status: "active", last_login_at: "—" }] }));
        break;
      }
      default: break;
    }
  }, []);

  return [state, dispatch];
}

function App() {
  const [loggedIn, setLoggedIn] = useState(false);
  const [user, setUser] = useState(CURRENT_USER);
  const [route, setRoute] = useState({ page: "dashboard" });
  const [collapsed, setCollapsed] = useState(false);
  const [state, dispatch] = useStore();

  const navigate = useCallback((r) => { setRoute(r); }, []);

  // guard: viewer can't reach restricted pages
  useEffect(() => {
    const nav = NAV.find((n) => n.key === route.page);
    if (nav && !nav.roles.includes(user.role)) { setRoute({ page: "dashboard" }); toast("无权限访问该页面", "error"); }
  }, [route.page, user.role]);

  if (!loggedIn) return (<><LoginPage onLogin={(u) => { setLoggedIn(true); setRoute({ page: "dashboard" }); }} /><ToastHost /></>);

  const wf = route.id != null ? state.workflows.find((w) => w.id === route.id) : null;
  const isEditor = route.page === "workflow-edit" && wf;
  const isRunDetail = route.page === "run-detail";

  let body;
  if (isEditor) body = <WorkflowEditor key={wf.id} wf={wf} navigate={navigate} dispatch={dispatch} user={user} openActivate={route.activate} />;
  else if (isRunDetail) body = <RunDetail id={route.id} navigate={navigate} />;
  else {
    let page;
    switch (route.page) {
      case "dashboard": page = <Dashboard navigate={navigate} />; break;
      case "workflows": page = <WorkflowsList navigate={navigate} user={user} state={state} dispatch={dispatch} />; break;
      case "members": page = <Members user={user} state={state} dispatch={dispatch} />; break;
      case "admin-users": page = <AdminUsers user={user} state={state} dispatch={dispatch} />; break;
      case "runs": page = <RunsList navigate={navigate} />; break;
      case "audit": page = <Audit user={user} />; break;
      case "settings": page = <Settings />; break;
      default: page = <Dashboard navigate={navigate} />;
    }
    body = <Content>{page}</Content>;
  }

  return (
    <div style={{ display: "flex", height: "100vh", overflow: "hidden" }}>
      <Sidebar route={route} navigate={navigate} collapsed={collapsed} setCollapsed={setCollapsed} user={user} />
      <div className="col" style={{ flex: 1, minWidth: 0, height: "100%" }}>
        <Topbar route={route} user={user}
          onLogout={() => { setLoggedIn(false); toast("已退出登录"); }}
          onSwitchRole={(r) => { setUser((u) => ({ ...u, role: r })); toast(`已切换到 ${ROLE_LABEL[r]}`, "success"); }} />
        <div className="col" style={{ flex: 1, minHeight: 0 }}>{body}</div>
      </div>
      <ToastHost />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
