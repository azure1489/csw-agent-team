// ============================================================
// Pages: Runs list · Run detail (read-only monitoring)
// ============================================================
const { useState, useEffect, useRef, useMemo, useCallback } = React;

function RunsList({ navigate }) {
  const [wf, setWf] = useState("");
  const [status, setStatus] = useState("");
  const [date, setDate] = useState("");
  const rows = RUNS
    .filter((r) => !wf || r.wf_key === wf)
    .filter((r) => !status || r.status === status)
    .filter((r) => !date || r.subject === date);
  return (
    <>
      <PageHeader title="运行监控" desc="查看所有运行实例及状态。后台不操作流程——派工 / 审核 / 提交均由 agent 经 csw-task skill 完成，此处全只读。"
        actions={<Btn variant="default" icon={I.refresh} onClick={() => toast("已刷新运行列表", "success")}>刷新</Btn>} />

      <Card noPad>
        <div className="row gap12" style={{ padding: "12px 14px", borderBottom: "1px solid var(--border)", flexWrap: "wrap" }}>
          <div style={{ width: 150 }}><Select value={wf} onChange={setWf} placeholder="全部工作流" options={[{ value: "", label: "全部工作流" }, { value: "daily_news", label: "资讯日更" }, { value: "event_recap", label: "活动复盘" }]} sm /></div>
          <div style={{ width: 130 }}><Select value={status} onChange={setStatus} placeholder="全部状态" options={[{ value: "", label: "全部状态" }, { value: "active", label: "进行中" }, { value: "done", label: "已完成" }, { value: "paused", label: "已暂停" }, { value: "aborted", label: "已中止" }]} sm /></div>
          <div style={{ width: 140 }}><TextInput value={date} onChange={setDate} placeholder="日期 subject" icon={I.clock} style={{ padding: "5px 10px 5px 32px", fontSize: 12.5 }} /></div>
          {(wf || status || date) && <Btn variant="ghost" size="sm" onClick={() => { setWf(""); setStatus(""); setDate(""); }}>清除筛选</Btn>}
        </div>
        <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13.5 }}>
          <thead>
            <tr style={{ color: "var(--text-3)", fontSize: 11.5, textTransform: "uppercase", letterSpacing: ".03em" }}>
              <Th style={{ paddingLeft: 18 }}>run</Th><Th>工作流</Th><Th>subject</Th><Th>状态</Th><Th>当前阶段</Th><Th>触发者</Th><Th>创建时间</Th><Th right style={{ paddingRight: 18 }}></Th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id} className="hover-row" style={{ borderTop: "1px solid var(--border)", cursor: "pointer" }} onClick={() => navigate({ page: "run-detail", id: r.id })}>
                <Td style={{ paddingLeft: 18 }}><span className="mono b">#{r.id}</span></Td>
                <Td><span className="b">{r.wf_name}</span></Td>
                <Td><span className="mono t2 sm">{r.subject}</span></Td>
                <Td><StatusBadge status={r.status} map={(s) => s === "active" ? "run_active" : s} /></Td>
                <Td><span className="t2">{r.cur_stage}</span></Td>
                <Td><span className="row gap6"><Avatar name={r.trigger} size={20} /><span className="sm">{r.trigger}</span></span></Td>
                <Td><span className="t3 mono sm">{r.created_at}</span></Td>
                <Td right style={{ paddingRight: 14 }}><Btn variant="ghost" size="sm" iconRight={I.chevRight}>查看</Btn></Td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <EmptyState icon={I.runs} title="没有匹配的运行实例" desc="调整筛选条件后再试" compact />}
      </Card>
    </>
  );
}

// ---------------- RUN DETAIL ----------------
const STAGE_GLYPH = {
  passed: { ch: "✓", tone: "green" }, review: { ch: "▣", tone: "amber" },
  dispatched: { ch: "▸", tone: "blue" }, ready: { ch: "▸", tone: "blue" },
  in_progress: { ch: "▸", tone: "blue" }, returned: { ch: "↩", tone: "red" }, blocked: { ch: "○", tone: "gray" },
};

function RunDetail({ id, navigate }) {
  const run = RUNS.find((r) => r.id === id) || RUNS.find((r) => r.id === 10);
  const isTen = run.id === 10;
  const stages = isTen ? RUN10_STAGES : syntheticStages(run);
  const timeline = isTen ? RUN10_TIMELINE : syntheticTimeline(run);
  const [order, setOrder] = useState("desc");
  const [expanded, setExpanded] = useState(isTen ? 3 : null);
  const passedCount = stages.filter((s) => s.status === "passed").length;
  const tl = order === "desc" ? [...timeline].reverse() : timeline;

  return (
    <Content wide>
      {/* header */}
      <div className="col gap4" style={{ marginBottom: 18 }}>
        <button className="row gap6 t3 sm" style={{ border: "none", background: "transparent", padding: 0, width: "fit-content" }} onClick={() => navigate({ page: "runs" })}><I.chevLeft size={14} />运行监控</button>
        <div className="row between wrap" style={{ gap: 12 }}>
          <div className="row gap12" style={{ alignItems: "center" }}>
            <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650 }}><span className="mono">run #{run.id}</span></h1>
            <span className="t2">·</span><span style={{ fontSize: 16, fontWeight: 600 }}>{run.wf_name}</span>
            <span className="t3 mono sm">{run.subject}</span>
            <StatusBadge status={run.status} map={(s) => s === "active" ? "run_active" : s} />
          </div>
          <span className="row gap6 sm" style={{ color: "var(--text-2)", background: "var(--surface-2)", border: "1px solid var(--border)", padding: "4px 11px", borderRadius: 999 }}>
            <I.eye size={14} />全只读 · 无派工 / 审核 / 提交
          </span>
        </div>
      </div>

      {/* progress strip */}
      <SectionCard title="进度" subtitle={`${passedCount} / ${stages.length} 阶段已完成`}>
        <div className="row gap8 wrap">
          {stages.map((s) => {
            const g = STAGE_GLYPH[s.status] || STAGE_GLYPH.blocked;
            const t = TONES[g.tone];
            return (
              <div key={s.seq} className="row gap6" title={`${s.name} · ${(STATUS[s.status] || {}).label || s.status}`}
                style={{ display: "flex", alignItems: "center", gap: 6, padding: "5px 10px 5px 6px", borderRadius: 999, border: `1px solid ${t.bd}`, background: t.bg }}>
                <span style={{ width: 19, height: 19, borderRadius: 999, background: t.c, color: "#fff", display: "flex", alignItems: "center", justifyContent: "center", fontSize: 11, fontWeight: 700 }}>{s.seq}</span>
                <span style={{ fontSize: 12, fontWeight: 550, color: t.c }}>{g.ch}</span>
              </div>
            );
          })}
        </div>
        {isTen && <div className="t3 xs" style={{ marginTop: 12 }}>③ 公众号内容 当前 v2，闸 1/2（主编审✔ · Van审待）</div>}
      </SectionCard>

      <div style={{ display: "grid", gridTemplateColumns: "1.5fr 1fr", gap: 14, marginTop: 14, alignItems: "start" }}>
        {/* stage detail */}
        <SectionCard title="阶段明细" subtitle="点开看产出 / 派工单 / 审核记录" noPad>
          <div className="col">
            {[...stages].filter((s) => s.status !== "blocked" || true).map((s, i) => {
              const open = expanded === s.seq;
              const g = STAGE_GLYPH[s.status] || STAGE_GLYPH.blocked;
              return (
                <div key={s.seq} style={{ borderTop: i ? "1px solid var(--border)" : "none" }}>
                  <div className="row between" onClick={() => setExpanded(open ? null : s.seq)} style={{ padding: "12px 16px", cursor: "pointer" }}
                    onMouseEnter={(e) => e.currentTarget.style.background = "var(--surface-2)"} onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}>
                    <div className="row gap12">
                      <span style={{ width: 22, height: 22, borderRadius: 6, background: TONES[g.tone].bg, color: TONES[g.tone].c, display: "flex", alignItems: "center", justifyContent: "center", fontSize: 11, fontWeight: 700 }}>{s.seq}</span>
                      <div className="col" style={{ gap: 2 }}>
                        <div className="row gap8"><span className="b" style={{ fontSize: 13.5 }}>{s.name}</span><span className="t3 sm">{s.role}</span></div>
                        <span className="t2 xs">{s.cur_v > 0 ? `当前 v${s.cur_v} · ${s.gate_state}` : "未就绪"}</span>
                      </div>
                    </div>
                    <div className="row gap10">
                      <StatusBadge status={s.status} sm />
                      {s.cur_v > 0 && <span className="row gap4 t3"><I.download size={14} /></span>}
                      <span style={{ transform: open ? "rotate(90deg)" : "none", transition: "transform .15s", color: "var(--text-3)", display: "flex" }}><I.chevRight size={15} /></span>
                    </div>
                  </div>
                  {open && (
                    <div className="col gap10" style={{ padding: "4px 16px 16px 50px", animation: "fadeIn .15s" }}>
                      {s.cur_v > 0 ? (
                        <>
                          <div className="row gap8 wrap">
                            <DownloadChip label={`产出 v${s.cur_v}`} sub={`${s.role}_${run.subject}_v${s.cur_v}.zip`} />
                            <DownloadChip label="派工单" sub="中枢意见 + 上游链接" dispatch />
                          </div>
                          {s.returns && s.returns.length > 0 && (
                            <div className="col gap6">
                              {s.returns.map((r, j) => (
                                <div key={j} className="col gap3" style={{ background: "var(--red-bg)", border: "1px solid var(--red-bd)", borderRadius: 7, padding: "8px 11px" }}>
                                  <div className="row gap6" style={{ color: "var(--red)", fontSize: 12, fontWeight: 600 }}><I.refresh size={13} />v{r.v} 在 {r.gate} 被退回</div>
                                  <div className="t2 xs">方向：{r.direction} · 位置：{r.location}</div>
                                </div>
                              ))}
                            </div>
                          )}
                        </>
                      ) : <span className="t3 sm">依赖未就绪，等待上游阶段完成。</span>}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </SectionCard>

        {/* timeline */}
        <SectionCard title="时间线" subtitle="events 流水" noPad
          actions={<Btn variant="ghost" size="sm" onClick={() => setOrder((o) => o === "desc" ? "asc" : "desc")}>{order === "desc" ? "最新在前" : "最早在前"}</Btn>}>
          <div className="col" style={{ padding: "8px 0", maxHeight: 540, overflowY: "auto" }}>
            {tl.map((e, i) => (
              <div key={i} className="row gap10" style={{ padding: "7px 16px", alignItems: "flex-start" }}>
                <span className="t3 mono xs" style={{ width: 38, flexShrink: 0, paddingTop: 2 }}>{e.t}</span>
                <div className="col" style={{ alignItems: "center", flexShrink: 0, paddingTop: 3 }}>
                  <span style={{ width: 8, height: 8, borderRadius: 999, background: EVENT_TONE[e.type] || "var(--gray)" }} />
                  {i < tl.length - 1 && <span style={{ width: 1.5, flex: 1, minHeight: 14, background: "var(--border-2)", marginTop: 3 }} />}
                </div>
                <div className="col" style={{ gap: 1, paddingBottom: 4 }}>
                  <span style={{ fontSize: 12.5, lineHeight: 1.45 }}>{e.text}</span>
                  <span className="t3 xs row gap5"><Avatar name={e.actor} size={14} />{e.actor}</span>
                </div>
              </div>
            ))}
          </div>
        </SectionCard>
      </div>
    </Content>
  );
}

const EVENT_TONE = {
  run_created: "var(--accent)", dispatched: "var(--blue)", file_uploaded: "var(--gray)",
  submitted: "var(--blue)", gate_passed: "var(--green)", stage_passed: "var(--green)",
  gate_returned: "var(--red)", run_done: "var(--green)",
};

function DownloadChip({ label, sub, dispatch }) {
  return (
    <button className="row gap8" onClick={() => toast(`下载 ${label}（演示）`, "success")}
      style={{ border: "1px solid var(--border-strong)", background: "var(--surface)", borderRadius: 8, padding: "7px 11px", boxShadow: "var(--shadow-sm)" }}
      onMouseEnter={(e) => e.currentTarget.style.background = "var(--surface-2)"} onMouseLeave={(e) => e.currentTarget.style.background = "var(--surface)"}>
      <span style={{ color: dispatch ? "var(--amber)" : "var(--accent-text)", display: "flex" }}>{dispatch ? <I.doc size={16} /> : <I.download size={16} />}</span>
      <div className="col" style={{ gap: 0, alignItems: "flex-start", lineHeight: 1.25 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600 }}>{label}</span>
        <span className="t3 mono" style={{ fontSize: 10.5 }}>{sub}</span>
      </div>
    </button>
  );
}

// synthetic data for non-#10 runs
function syntheticStages(run) {
  if (run.status === "done") return [1, 2, 3, 4, 5, 6].map((n) => ({ seq: n, name: `0${n}-阶段`, role: "—", status: "passed", cur_v: 1, gate_state: "已通过", returns: [] }));
  return [{ seq: 1, name: "01-素材汇集", role: "情报收集员", status: "passed", cur_v: 1, gate_state: "已通过", returns: [] },
  { seq: 2, name: "02-复盘提纲", role: "选题研究员", status: "dispatched", cur_v: 0, gate_state: "已派工 · 待提交", returns: [] },
  { seq: 3, name: "03-初稿", role: "文案", status: "blocked", cur_v: 0, gate_state: "—", returns: [] },
  { seq: 4, name: "04-配图", role: "设计师", status: "blocked", cur_v: 0, gate_state: "—", returns: [] },
  { seq: 5, name: "05-成品", role: "主编", status: "blocked", cur_v: 0, gate_state: "—", returns: [] },
  { seq: 6, name: "06-发布", role: "发布员", status: "blocked", cur_v: 0, gate_state: "—", returns: [] }];
}
function syntheticTimeline(run) {
  if (run.status === "done") return [{ t: "06:00", actor: run.trigger, type: "run_created", text: `触发实例 ${run.wf_name} · ${run.subject}` }, { t: "07:30", actor: "情报bot", type: "stage_passed", text: "全部阶段依次通过" }, { t: "18:40", actor: "发布bot", type: "run_done", text: "末阶段通过 → 实例完成" }];
  return [{ t: "09:10", actor: run.trigger, type: "run_created", text: `触发实例 ${run.wf_name} · ${run.subject}` }, { t: "09:20", actor: "主编bot", type: "dispatched", text: "派工 01-素材汇集" }, { t: "10:05", actor: "情报bot", type: "stage_passed", text: "01-素材汇集 双闸通过" }, { t: "10:10", actor: "主编bot", type: "dispatched", text: "派工 02-复盘提纲" }];
}

Object.assign(window, { RunsList, RunDetail });
