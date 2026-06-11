// ============================================================
// Workflow Editor (hero) — basic / stages / DAG / gates / common
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

const OUTPUT_OPTS = ["资讯包", "选题成品", "文章", "封面", "成品", "发布物", "小红书文本", "卡片", "素材包", "提纲", "初稿", "配图"];

function WorkflowEditor({ wf, navigate, dispatch, user, openActivate }) {
  const readonly = wf.status !== "draft" || user.role === "viewer";
  const [draft, setDraft] = useState(() => JSON.parse(JSON.stringify(wf)));
  const [section, setSection] = useState("basic");
  const [editStage, setEditStage] = useState(null);
  const [dirty, setDirty] = useState(false);
  const [validateOpen, setValidateOpen] = useState(false);
  const [discardOpen, setDiscardOpen] = useState(false);
  const dragIdx = useRef(null);
  const [dragOver, setDragOver] = useState(null);
  const scrollRef = useRef(null);

  useEffect(() => { if (openActivate) setValidateOpen(true); }, [openActivate]);

  const update = (patch) => { setDraft((d) => ({ ...d, ...patch })); setDirty(true); };
  const updateStage = (id, patch) => {
    setDraft((d) => ({ ...d, stages: d.stages.map((s) => (s.id === id ? { ...s, ...patch } : s)) }));
    setDirty(true);
  };
  const roleName = (c) => (ROLES.find((r) => r.code === c) || {}).name || c;

  // ---- stage table actions ----
  const reorder = (from, to) => {
    if (from === to) return;
    setDraft((d) => {
      const arr = [...d.stages];
      const [m] = arr.splice(from, 1); arr.splice(to, 0, m);
      arr.forEach((s, i) => (s.seq = i + 1));
      return { ...d, stages: arr };
    });
    setDirty(true);
  };
  const addStage = () => {
    const maxId = Math.max(1000, ...draft.stages.map((s) => s.id));
    const ns = { id: maxId + 1, seq: draft.stages.length + 1, code: `stage_${draft.stages.length + 1}`, name: `${String(draft.stages.length + 1).padStart(2, "0")}-新阶段`, role_code: "writer", output_type: "成品", is_merge: false, deps: draft.stages.length ? [draft.stages[draft.stages.length - 1].id] : [], instructions: MD.generic_inst, self_check: MD.generic_self, acceptance: MD.generic_acc, gate_override: null };
    setDraft((d) => ({ ...d, stages: [...d.stages, ns] })); setDirty(true);
    setEditStage(ns.id);
  };
  const delStage = (id) => {
    setDraft((d) => ({ ...d, stages: d.stages.filter((s) => s.id !== id).map((s) => ({ ...s, deps: s.deps.filter((x) => x !== id) })) }));
    setDirty(true);
  };

  const SECTIONS = [
    { key: "basic", label: "基本信息", icon: I.doc },
    { key: "stages", label: "阶段", icon: I.workflow, count: draft.stages.length },
    { key: "dag", label: "依赖关系 (DAG)", icon: I.flow },
    { key: "gates", label: "审核闸", icon: I.checkCircle },
    { key: "common", label: "通用约定", icon: I.audit },
  ];

  const stageById = (id) => draft.stages.find((s) => s.id === id);

  const doSave = () => { dispatch({ type: "save", id: wf.id, draft }); setDirty(false); toast("草稿已保存", "success", "整份阶段 / 依赖 / 闸 / 文本已提交"); };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* scroll body */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: "auto" }}>
        <div style={{ maxWidth: 1180, margin: "0 auto", padding: "24px 32px 120px" }}>
          {/* header */}
          <div className="row between wrap" style={{ gap: 14, marginBottom: 8 }}>
            <div className="col" style={{ gap: 6 }}>
              <button className="row gap6 t3 sm" style={{ border: "none", background: "transparent", padding: 0, width: "fit-content" }} onClick={() => maybeLeave(dirty, () => navigate({ page: "workflows" }), setDiscardOpen)}>
                <I.chevLeft size={14} />工作流
              </button>
              <div className="row gap12" style={{ alignItems: "center" }}>
                <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650 }}>{draft.name}</h1>
                <StatusBadge status={draft.status} />
                <span className="t3 mono sm">v{draft.version}{wf.status === "draft" && wf.version > 1 ? " · 基于 v" + (wf.version - 1) + " 复制" : ""}</span>
              </div>
            </div>
          </div>

          {readonly && (
            <div className="row gap8" style={{ background: "var(--amber-bg)", border: "1px solid var(--amber-bd)", color: "var(--amber)", padding: "9px 13px", borderRadius: 8, fontSize: 12.5, marginBottom: 18 }}>
              <I.lock size={15} />
              {user.role === "viewer" ? "访客为只读权限，无法编辑工作流。" : <>该版本已 <b>{draft.status === "active" ? "激活" : "归档"}</b>，为只读查看。如需修改，请「复制为新版本」生成草稿。</>}
            </div>
          )}

          <div style={{ display: "grid", gridTemplateColumns: "190px 1fr", gap: 26, alignItems: "start", marginTop: 14 }}>
            {/* anchor nav */}
            <div style={{ position: "sticky", top: 14 }}>
              <div className="col gap2">
                {SECTIONS.map((s) => (
                  <button key={s.key} onClick={() => setSection(s.key)}
                    style={{ display: "flex", alignItems: "center", gap: 9, padding: "8px 10px", borderRadius: 7, border: "none", textAlign: "left",
                      background: section === s.key ? "var(--accent-weak)" : "transparent",
                      color: section === s.key ? "var(--accent-text)" : "var(--text-2)", fontSize: 13, fontWeight: section === s.key ? 600 : 500 }}
                    onMouseEnter={(e) => { if (section !== s.key) e.currentTarget.style.background = "var(--surface-2)"; }}
                    onMouseLeave={(e) => { if (section !== s.key) e.currentTarget.style.background = "transparent"; }}>
                    <s.icon size={16} /><span className="grow">{s.label}</span>
                    {s.count != null && <span className="mono xs t3">{s.count}</span>}
                  </button>
                ))}
              </div>
            </div>

            {/* section content */}
            <div className="col gap20" style={{ minWidth: 0 }}>
              {section === "basic" && <BasicSection draft={draft} update={update} readonly={readonly} roleName={roleName} />}
              {section === "stages" && (
                <StagesSection draft={draft} readonly={readonly} roleName={roleName} stageById={stageById} updateStage={updateStage}
                  reorder={reorder} addStage={addStage} delStage={delStage} setEditStage={setEditStage}
                  dragIdx={dragIdx} dragOver={dragOver} setDragOver={setDragOver} />
              )}
              {section === "dag" && (
                <SectionCard title="依赖关系（DAG）" subtitle="由阶段 + 依赖实时渲染。高亮入口 / 终点，有环时红色标出。">
                  <DagView stages={draft.stages} />
                </SectionCard>
              )}
              {section === "gates" && <GatesSection draft={draft} update={update} readonly={readonly} roleName={roleName} setEditStage={setEditStage} />}
              {section === "common" && <CommonSection draft={draft} update={update} readonly={readonly} />}
            </div>
          </div>
        </div>
      </div>

      {/* sticky action bar */}
      <div className="row between" style={{ flexShrink: 0, padding: "12px 32px", borderTop: "1px solid var(--border)", background: "rgba(255,255,255,.86)", backdropFilter: "blur(8px)" }}>
        <div className="row gap8">
          {!readonly && <Btn variant="ghost" onClick={() => maybeLeave(dirty, () => navigate({ page: "workflows" }), setDiscardOpen)}>放弃修改</Btn>}
          {readonly && <Btn variant="default" icon={I.chevLeft} onClick={() => navigate({ page: "workflows" })}>返回列表</Btn>}
          {dirty && <span className="row gap6 t3 sm"><span style={{ width: 6, height: 6, borderRadius: 999, background: "var(--amber)" }} />有未保存的修改</span>}
        </div>
        {!readonly && (
          <div className="row gap8">
            <Btn variant="default" onClick={doSave}>保存草稿</Btn>
            <Btn variant="default" icon={I.checkCircle} onClick={() => setValidateOpen(true)}>校验</Btn>
            <Btn variant="primary" icon={I.bolt} onClick={() => setValidateOpen(true)}>校验并激活</Btn>
          </div>
        )}
        {readonly && wf.status === "active" && user.role !== "viewer" && (
          <Btn variant="default" icon={I.copy} onClick={() => { dispatch({ type: "clone", id: wf.id }); toast("已复制为新草稿版本", "success"); navigate({ page: "workflows" }); }}>复制为新版本</Btn>
        )}
      </div>

      {/* stage detail drawer */}
      {editStage != null && (
        <StageDrawer stage={stageById(editStage)} allStages={draft.stages} draft={draft} readonly={readonly}
          roleName={roleName} onClose={() => setEditStage(null)} onSave={(patch) => { updateStage(editStage, patch); setEditStage(null); toast("阶段已更新", "success"); }} />
      )}

      <ValidateModal open={validateOpen} onClose={() => setValidateOpen(false)} draft={draft}
        onActivate={() => { dispatch({ type: "activate", id: wf.id, draft }); setValidateOpen(false); toast("工作流已激活", "success", `${draft.name} v${draft.version} · 旧版自动归档`); navigate({ page: "workflows" }); }}
        gotoStage={(id) => { setValidateOpen(false); setSection("stages"); setEditStage(id); }} />

      <ConfirmDialog open={discardOpen} onClose={() => setDiscardOpen(false)} danger title="放弃未保存的修改？" confirmText="放弃并离开"
        message="当前草稿有未保存的改动，离开后将丢失。确定要离开吗？" onConfirm={() => { setDiscardOpen(false); navigate({ page: "workflows" }); }} />
    </div>
  );
}

function maybeLeave(dirty, go, setDiscard) { if (dirty) setDiscard(true); else go(); }

// ---------- Basic section ----------
function BasicSection({ draft, update, readonly, roleName }) {
  const mgmtRoles = ROLES.filter((r) => r.is_management);
  const triggerable = ROLES.filter((r) => r.is_management || r.code === "scheduler");
  const tr = draft.trigger_roles || [];
  const toggleTrigger = (code) => update({ trigger_roles: tr.includes(code) ? tr.filter((x) => x !== code) : [...tr, code] });
  return (
    <SectionCard title="基本信息">
      <div className="col gap20">
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
          <Field label="key（机器键）" hint="创建后只读"><TextInput value={draft.wf_key} readOnly mono /></Field>
          <Field label="名称" required><TextInput value={draft.name} onChange={(v) => update({ name: v })} readOnly={readonly} /></Field>
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16 }}>
          <Field label="中枢角色" hint="仅管理类角色可选；负责派工 / 代录人工闸 / 合流">
            <Select value={draft.hub_role} onChange={(v) => update({ hub_role: v })} options={mgmtRoles.map((r) => ({ value: r.code, label: `${r.name}（${r.code}）` }))} disabled={readonly} />
          </Field>
          <Field label="派工模式" hint="manual=中枢手动派工；auto=依赖就绪自动建任务">
            <Segmented value={draft.dispatch_mode} onChange={(v) => !readonly && update({ dispatch_mode: v })} options={[{ value: "manual", label: "手动 manual" }, { value: "auto", label: "自动 auto" }]} />
          </Field>
        </div>
        <Field label="可触发角色" hint="至少一项；定义谁能触发本工作流的实例">
          <div className="row wrap gap8">
            {triggerable.map((r) => {
              const on = tr.includes(r.code);
              return (
                <button key={r.code} disabled={readonly} onClick={() => toggleTrigger(r.code)}
                  style={{ display: "flex", alignItems: "center", gap: 7, padding: "6px 12px", borderRadius: 999, fontSize: 12.5, fontWeight: 540,
                    border: `1px solid ${on ? "var(--accent)" : "var(--border-strong)"}`, background: on ? "var(--accent-weak)" : "var(--surface)",
                    color: on ? "var(--accent-text)" : "var(--text-2)", cursor: readonly ? "default" : "pointer" }}>
                  {on ? <I.check size={13} /> : <I.plus size={13} />}{r.name}
                </button>
              );
            })}
          </div>
        </Field>
      </div>
    </SectionCard>
  );
}

// ---------- Stages section ----------
function StagesSection({ draft, readonly, roleName, stageById, updateStage, reorder, addStage, delStage, setEditStage, dragIdx, dragOver, setDragOver }) {
  const depNames = (deps) => deps.length ? deps.map((d) => { const s = stageById(d); return s ? s.seq : "?"; }).join(", ") : "—";
  return (
    <SectionCard title="阶段" subtitle={readonly ? "只读" : "拖动手柄排序 · 点行末 ✎ 编辑阶段详情（依赖 / 闸 / 三栏文档）"} noPad
      actions={!readonly && <Btn variant="default" size="sm" icon={I.plus} onClick={addStage}>加阶段</Btn>}>
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
        <thead>
          <tr style={{ color: "var(--text-3)", fontSize: 11, textTransform: "uppercase", letterSpacing: ".03em" }}>
            <Th style={{ paddingLeft: 14, width: 28 }}></Th><Th style={{ width: 40 }}>#</Th><Th>code</Th><Th>名称</Th><Th>责任角色</Th><Th>产出</Th><Th center>合流</Th><Th>依赖</Th><Th right style={{ paddingRight: 14 }}></Th>
          </tr>
        </thead>
        <tbody>
          {draft.stages.map((s, idx) => (
            <tr key={s.id} draggable={!readonly}
              onDragStart={() => (dragIdx.current = idx)}
              onDragOver={(e) => { e.preventDefault(); setDragOver(idx); }}
              onDragEnd={() => { if (dragIdx.current != null && dragOver != null) reorder(dragIdx.current, dragOver); dragIdx.current = null; setDragOver(null); }}
              style={{ borderTop: "1px solid var(--border)", background: dragOver === idx && dragIdx.current != null ? "var(--accent-weak)" : "transparent" }} className="hover-row">
              <Td style={{ paddingLeft: 14 }}>
                {!readonly && <span style={{ cursor: "grab", color: "var(--text-3)", display: "flex" }}><I.drag size={16} /></span>}
              </Td>
              <Td><span className="mono" style={{ fontWeight: 600 }}>{s.seq}</span></Td>
              <Td><span className="mono t2" style={{ fontSize: 12 }}>{s.code}</span></Td>
              <Td><span className="b">{s.name}</span></Td>
              <Td>
                {readonly ? <span className="t2">{roleName(s.role_code)}</span> : (
                  <div style={{ width: 120 }}><Select value={s.role_code} onChange={(v) => updateStage(s.id, { role_code: v })} options={ROLES.filter((r) => r.code !== "scheduler").map((r) => ({ value: r.code, label: r.name }))} sm /></div>
                )}
              </Td>
              <Td>
                {readonly ? <span className="t2 sm">{s.output_type}</span> : (
                  <div style={{ width: 104 }}><Select value={s.output_type} onChange={(v) => updateStage(s.id, { output_type: v })} options={OUTPUT_OPTS} sm /></div>
                )}
              </Td>
              <Td center>{s.is_merge ? <Badge tone="violet" sm>合流</Badge> : <span className="t3">—</span>}</Td>
              <Td><span className="mono t2 sm">{depNames(s.deps)}</span></Td>
              <Td right style={{ paddingRight: 10 }}>
                <div className="row gap2" style={{ justifyContent: "flex-end" }}>
                  <IconBtn icon={I.edit} title="编辑阶段" onClick={() => setEditStage(s.id)} />
                  {!readonly && draft.stages.length > 1 && <IconBtn icon={I.x} title="删除阶段" danger onClick={() => delStage(s.id)} />}
                </div>
              </Td>
            </tr>
          ))}
        </tbody>
      </table>
      {draft.stages.length === 0 && <EmptyState icon={I.workflow} title="还没有阶段" desc="至少需要 1 个阶段" compact action={!readonly && <Btn size="sm" variant="primary" icon={I.plus} onClick={addStage}>加阶段</Btn>} />}
    </SectionCard>
  );
}

// ---------- Gates section ----------
function GatesSection({ draft, update, readonly, roleName, setEditStage }) {
  const overrides = draft.stages.filter((s) => s.gate_override);
  return (
    <div className="col gap20">
      <SectionCard title="默认闸" subtitle="套用到所有未单独配闸的阶段">
        <div className="col gap10">
          {draft.default_gates.map((g, i) => (
            <div key={i} className="row between" style={{ padding: "11px 14px", border: "1px solid var(--border)", borderRadius: 8, background: "var(--surface-2)" }}>
              <div className="row gap12">
                <span className="mono b" style={{ width: 22, height: 22, borderRadius: 6, background: "var(--accent-weak)", color: "var(--accent-text)", display: "flex", alignItems: "center", justifyContent: "center", fontSize: 12 }}>{g.gate_order}</span>
                <div className="col" style={{ gap: 1 }}>
                  <span className="b" style={{ fontSize: 13 }}>{g.name}</span>
                  <span className="t3 xs">审核角色：{roleName(g.reviewer_role)}</span>
                </div>
              </div>
              {g.relayed && <Badge tone="amber" sm><I.hand size={11} />人工代录</Badge>}
            </div>
          ))}
        </div>
      </SectionCard>
      <SectionCard title="阶段覆盖" subtitle="个别阶段可单独配闸，覆盖默认；其余沿用默认闸">
        {overrides.length === 0 ? (
          <EmptyState icon={I.checkCircle} title="无阶段覆盖" desc="所有阶段均使用默认闸。在「阶段」中点 ✎ 可为某阶段单独配闸。" compact />
        ) : (
          <div className="col gap10">
            {overrides.map((s) => (
              <div key={s.id} className="row between" style={{ padding: "11px 14px", border: "1px solid var(--border)", borderRadius: 8 }}>
                <div className="col" style={{ gap: 3 }}>
                  <span className="b" style={{ fontSize: 13 }}>{s.name}</span>
                  <div className="row gap6 wrap">
                    {s.gate_override.map((g, i) => <Badge key={i} tone="violet" sm>{g.gate_order}. {g.name}{g.relayed ? " ·代录" : ""}</Badge>)}
                  </div>
                </div>
                <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => setEditStage(s.id)}>编辑</Btn>
              </div>
            ))}
          </div>
        )}
      </SectionCard>
    </div>
  );
}

// ---------- Common section ----------
function CommonSection({ draft, update, readonly }) {
  const [tab, setTab] = useState("inst");
  return (
    <SectionCard title="通用约定" subtitle="作为各阶段作业手册 / 验收标准的公共部分，随派工 / 审核一并下发">
      <Tabs tabs={[{ value: "inst", label: "通用约定 common_instructions" }, { value: "acc", label: "通用合规项 common_acceptance" }]} value={tab} onChange={setTab} />
      <div style={{ marginTop: 14 }}>
        {tab === "inst" ? (
          readonly ? <div style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 14, background: "var(--surface-2)" }}><MarkdownView text={draft.common_instructions || "（空）"} /></div>
            : <Textarea value={draft.common_instructions} onChange={(v) => update({ common_instructions: v })} rows={7} mono placeholder="Markdown：交付通用约定…" />
        ) : (
          readonly ? <div style={{ border: "1px solid var(--border)", borderRadius: 8, padding: 14, background: "var(--surface-2)" }}><MarkdownView text={draft.common_acceptance || "（空）"} /></div>
            : <Textarea value={draft.common_acceptance} onChange={(v) => update({ common_acceptance: v })} rows={7} mono placeholder="Markdown：通用合规项（所有阶段审核都附）…" />
        )}
      </div>
    </SectionCard>
  );
}

Object.assign(window, { WorkflowEditor, OUTPUT_OPTS });
