// ============================================================
// Editor parts: StageDrawer · ValidateModal
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

function StageDrawer({ stage, allStages, draft, readonly, roleName, onClose, onSave }) {
  const [s, setS] = useState(() => JSON.parse(JSON.stringify(stage)));
  const [tab, setTab] = useState("inst");
  const [preview, setPreview] = useState(false);
  const set = (patch) => setS((x) => ({ ...x, ...patch }));
  const others = allStages.filter((x) => x.id !== s.id);
  const customGates = !!s.gate_override;

  const toggleDep = (id) => set({ deps: s.deps.includes(id) ? s.deps.filter((x) => x !== id) : [...s.deps, id] });
  const setGateMode = (custom) => set({ gate_override: custom ? (s.gate_override || draft.default_gates.map((g) => ({ ...g }))) : null });
  const updGate = (i, patch) => set({ gate_override: s.gate_override.map((g, j) => (j === i ? { ...g, ...patch } : g)) });
  const addGate = () => set({ gate_override: [...s.gate_override, { gate_order: s.gate_override.length + 1, reviewer_role: "editor", relayed: false, name: "新审核" }] });
  const delGate = (i) => set({ gate_override: s.gate_override.filter((_, j) => j !== i).map((g, k) => ({ ...g, gate_order: k + 1 })) });

  const TABS = [{ value: "inst", label: "作业手册" }, { value: "self", label: "自检标准" }, { value: "acc", label: "验收标准" }];
  const tabKey = { inst: "instructions", self: "self_check", acc: "acceptance" }[tab];
  const tabHint = { inst: "Markdown：任务内容——做什么 / 工具 / 产出要求。派工时随单下发。", self: "Markdown：交付前自查清单，全过才交。结果随 submit 申报。", acc: "Markdown：主编 / Van 审核依据（与自检不同）。审核时随附。" }[tab];

  return (
    <Drawer open onClose={onClose} width={620} title={`阶段：${s.name}`} subtitle={`${s.code} · 责任角色 ${roleName(s.role_code)}`}
      badge={s.is_merge && <Badge tone="violet" sm>合流</Badge>}
      footer={<><Btn variant="ghost" onClick={onClose}>取消</Btn>{!readonly && <Btn variant="primary" onClick={() => onSave(s)}>保存阶段</Btn>}</>}>
      <div className="col gap24">
        {/* basic toggles */}
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 14 }}>
          <Field label="名称"><TextInput value={s.name} onChange={(v) => set({ name: v })} readOnly={readonly} /></Field>
          <Field label="产出类型"><Select value={s.output_type} onChange={(v) => set({ output_type: v })} options={OUTPUT_OPTS} disabled={readonly} /></Field>
        </div>
        <div className="row gap16">
          <div className="row gap8"><Switch checked={s.is_merge} onChange={(v) => set({ is_merge: v })} disabled={readonly} /><span className="sm">合流阶段（由中枢合成，自动派工给自己）</span></div>
        </div>

        {/* deps */}
        <Field label="依赖上游" hint="勾选构成 DAG 边。不能选自己，保存激活时校验不成环。">
          {others.length === 0 ? <span className="t3 sm">没有其他阶段</span> : (
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8, border: "1px solid var(--border)", borderRadius: 8, padding: 12 }}>
              {others.map((o) => (
                <Checkbox key={o.id} checked={s.deps.includes(o.id)} onChange={() => !readonly && toggleDep(o.id)} disabled={readonly}
                  label={<span><span className="mono t3" style={{ marginRight: 5 }}>{o.seq}</span>{o.name}</span>} />
              ))}
            </div>
          )}
        </Field>

        {/* gates */}
        <Field label="审核闸">
          <div className="col gap10">
            <div className="row gap20">
              <Radio checked={!customGates} onChange={() => !readonly && setGateMode(false)} label="用工作流默认闸" />
              <Radio checked={customGates} onChange={() => !readonly && setGateMode(true)} label="为本阶段单独配" />
            </div>
            {!customGates ? (
              <div className="col gap8" style={{ border: "1px dashed var(--border-2)", borderRadius: 8, padding: 12, background: "var(--surface-2)" }}>
                {draft.default_gates.map((g, i) => (
                  <div key={i} className="row gap10"><span className="mono b sm" style={{ color: "var(--accent-text)" }}>{g.gate_order}.</span><span className="sm">{g.name}</span><span className="t3 xs">（{roleName(g.reviewer_role)}{g.relayed ? " · 人工代录" : ""}）</span></div>
                ))}
                <span className="t3 xs">沿用默认闸，未覆盖。</span>
              </div>
            ) : (
              <div className="col gap8">
                {s.gate_override.map((g, i) => (
                  <div key={i} className="row gap8" style={{ border: "1px solid var(--border)", borderRadius: 8, padding: "9px 11px", alignItems: "center" }}>
                    <span className="mono b sm" style={{ width: 16, color: "var(--accent-text)" }}>{g.gate_order}</span>
                    <div style={{ width: 132 }}><Select value={g.reviewer_role} onChange={(v) => updGate(i, { reviewer_role: v })} options={ROLES.filter((r) => r.is_management || !r.is_human).map((r) => ({ value: r.code, label: r.name }))} disabled={readonly} sm /></div>
                    <div className="grow"><TextInput value={g.name} onChange={(v) => updGate(i, { name: v })} readOnly={readonly} style={{ padding: "5px 10px", fontSize: 12.5 }} /></div>
                    <label className="row gap5 sm" style={{ whiteSpace: "nowrap" }}><Checkbox checked={g.relayed} onChange={(v) => !readonly && updGate(i, { relayed: v })} disabled={readonly} /><span className="xs t2">人工代录</span></label>
                    {!readonly && <IconBtn icon={I.x} danger title="删除闸" onClick={() => delGate(i)} />}
                  </div>
                ))}
                {!readonly && <Btn variant="ghost" size="sm" icon={I.plus} onClick={addGate} style={{ width: "fit-content" }}>加一道闸</Btn>}
              </div>
            )}
          </div>
        </Field>

        {/* three markdown editors */}
        <div className="col gap10">
          <div className="row between">
            <Tabs tabs={TABS} value={tab} onChange={setTab} />
            <Btn variant="ghost" size="sm" icon={preview ? I.edit : I.eye} onClick={() => setPreview((p) => !p)}>{preview ? "编辑" : "预览"}</Btn>
          </div>
          <div className="t3 xs">{tabHint}</div>
          {preview || readonly ? (
            <div style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 14, background: "var(--surface-2)", minHeight: 180 }}>
              <MarkdownView text={s[tabKey] || "（空）"} />
            </div>
          ) : (
            <Textarea value={s[tabKey] || ""} onChange={(v) => set({ [tabKey]: v })} rows={10} mono />
          )}
        </div>
      </div>
    </Drawer>
  );
}

// ---------- Validate modal ----------
function runChecks(draft) {
  const checks = [];
  const { entries, terminals, hasCycle } = computeDag(draft.stages);
  const byId = {}; draft.stages.forEach((s) => (byId[s.id] = s));

  checks.push({ status: hasCycle ? "fail" : "pass", text: hasCycle ? "DAG 存在环依赖" : "DAG 无环" });

  // reachability from entries
  const adj = {}; draft.stages.forEach((s) => (adj[s.id] = []));
  draft.stages.forEach((s) => (s.deps || []).forEach((d) => { if (adj[d]) adj[d].push(s.id); }));
  const reach = new Set();
  const q = [...entries]; entries.forEach((e) => reach.add(e));
  while (q.length) { const u = q.shift(); (adj[u] || []).forEach((v) => { if (!reach.has(v)) { reach.add(v); q.push(v); } }); }
  const allReach = draft.stages.every((s) => reach.has(s.id));
  const entryArr = [...entries].map((id) => byId[id]?.name).join("、");
  if (entries.size === 1) checks.push({ status: allReach ? "pass" : "fail", text: `单一入口（${entryArr}）${allReach ? "可达所有阶段" : "无法到达全部阶段"}` });
  else if (entries.size === 0) checks.push({ status: "fail", text: "没有入口阶段（所有阶段都有依赖，构成环）" });
  else checks.push({ status: "warn", text: `存在 ${entries.size} 个入口（${entryArr}）——多入口，请确认是否预期` });

  const termArr = [...terminals].map((id) => byId[id]?.name).join("、");
  checks.push({ status: terminals.size ? "pass" : "fail", text: terminals.size ? `终点（${termArr}）可达` : "没有终点阶段" });

  const hub = ROLES.find((r) => r.code === draft.hub_role);
  checks.push({ status: hub && hub.is_management ? "pass" : "fail", text: hub && hub.is_management ? `中枢角色「${hub.name}」为管理类` : "中枢角色非管理类或不存在" });

  const allGates = [...draft.default_gates, ...draft.stages.flatMap((s) => s.gate_override || [])];
  const badGate = allGates.find((g) => !ROLES.find((r) => r.code === g.reviewer_role));
  checks.push({ status: badGate ? "fail" : "pass", text: badGate ? `审核角色「${badGate.reviewer_role}」不存在` : "各闸审核角色均存在" });

  const emptyStages = draft.stages.filter((s) => !((s.instructions || "").trim() && (s.self_check || "").trim() && (s.acceptance || "").trim()));
  if (emptyStages.length === 0) checks.push({ status: "pass", text: "每阶段 instructions / self_check / acceptance 均非空" });
  else emptyStages.forEach((s) => checks.push({ status: "fail", text: `${s.name}：作业手册 / 自检 / 验收 存在空项`, stageId: s.id }));

  const codes = draft.stages.map((s) => s.code);
  const dup = codes.find((c, i) => codes.indexOf(c) !== i);
  if (dup) checks.push({ status: "fail", text: `阶段 code「${dup}」重复` });

  // merge + hub warning
  const mergeHub = draft.stages.filter((s) => s.is_merge && s.role_code === draft.hub_role);
  if (mergeHub.length) checks.push({ status: "warn", text: `${mergeHub.map((s) => s.seq).join("、")} 为合流且责任=中枢，将自动派工给中枢自己（提示）` });

  return checks;
}

function ValidateModal({ open, onClose, draft, onActivate, gotoStage }) {
  const checks = useMemo(() => (open ? runChecks(draft) : []), [open, draft]);
  const fails = checks.filter((c) => c.status === "fail").length;
  const warns = checks.filter((c) => c.status === "warn").length;
  const icon = { pass: I.checkCircle, warn: I.warn, fail: I.xCircle };
  const color = { pass: "var(--green)", warn: "var(--amber)", fail: "var(--red)" };
  const bg = { pass: "transparent", warn: "var(--amber-bg)", fail: "var(--red-bg)" };
  return (
    <Modal open={open} onClose={onClose} width={540} icon={fails ? I.xCircle : I.checkCircle} iconTone={fails ? "red" : "green"}
      title={`校验：${draft.name} v${draft.version}`}
      subtitle={fails ? `${fails} 项未通过，修正后才能激活` : warns ? `全部通过，${warns} 条提示` : "全部检查通过，可以激活"}
      footer={<>
        <span className="t3 sm">激活后同 key 的旧 active 版本将自动归档</span>
        <div className="row gap8">
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" icon={I.bolt} disabled={fails > 0} onClick={onActivate}>确认激活（旧版归档）</Btn>
        </div>
      </>}>
      <div className="col gap6" style={{ padding: "4px 0 14px" }}>
        {checks.map((c, i) => {
          const IconC = icon[c.status];
          return (
            <div key={i} className="row gap10" onClick={() => c.stageId && gotoStage(c.stageId)}
              style={{ padding: "9px 11px", borderRadius: 8, background: bg[c.status], cursor: c.stageId ? "pointer" : "default", alignItems: "flex-start" }}>
              <span style={{ color: color[c.status], display: "flex", marginTop: 1 }}><IconC size={17} /></span>
              <span className="grow" style={{ fontSize: 13, lineHeight: 1.5 }}>{c.text}</span>
              {c.stageId && <span className="row gap4 xs" style={{ color: "var(--accent-text)", fontWeight: 550, whiteSpace: "nowrap" }}>定位<I.chevRight size={12} /></span>}
            </div>
          );
        })}
      </div>
    </Modal>
  );
}

Object.assign(window, { StageDrawer, ValidateModal, runChecks });
