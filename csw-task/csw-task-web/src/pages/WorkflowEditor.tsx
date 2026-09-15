import { useEffect, useRef, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { I } from '@/components/icons'
import {
  Badge,
  Btn,
  Checkbox,
  ConfirmDialog,
  EmptyState,
  Field,
  IconBtn,
  SectionCard,
  Segmented,
  Select,
  StatusBadge,
  Tabs,
  Td,
  Textarea,
  TextInput,
  Th,
  MarkdownView,
  toast,
} from '@/components/ui'
import { DagView } from '@/components/dag/DagView'
import { StageDrawer } from './editor/StageDrawer'
import { ValidateModal } from './editor/ValidateModal'
import { api, getApiError } from '@/lib/api'
import { apiToEditor, editorToPutBody } from '@/lib/adapters'
import { useRoles, useRoleName } from '@/hooks/useRoles'
import { useAuth } from '@/lib/auth'
import type { EditorGate, EditorStage, EditorWorkflow } from '@/types'

// ---------- Basic ----------
function BasicSection({ draft, update, readonly }: { draft: EditorWorkflow; update: (p: Partial<EditorWorkflow>) => void; readonly: boolean }) {
  const { data: roles } = useRoles()
  const mgmtRoles = (roles || []).filter((r) => r.is_management)
  const triggerable = (roles || []).filter((r) => r.is_management || r.code === 'scheduler')
  const tr = draft.trigger_roles || []
  const toggleTrigger = (code: string) => update({ trigger_roles: tr.includes(code) ? tr.filter((x) => x !== code) : [...tr, code] })
  return (
    <SectionCard title="基本信息">
      <div className="col gap20">
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <Field label="key（机器键）" hint="创建后只读">
            <TextInput value={draft.wf_key} readOnly mono />
          </Field>
          <Field label="名称" required>
            <TextInput value={draft.name} onChange={(v) => update({ name: v })} readOnly={readonly} />
          </Field>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
          <Field label="中枢角色" hint="仅管理类角色可选；负责派工 / 代录人工闸 / 合流">
            <Select value={draft.hub_role} onChange={(v) => update({ hub_role: v })} options={mgmtRoles.map((r) => ({ value: r.code, label: `${r.name}（${r.code}）` }))} disabled={readonly} />
          </Field>
          <Field label="派工模式" hint="manual=中枢手动派工；auto=依赖就绪自动建任务">
            <Segmented
              value={draft.dispatch_mode}
              onChange={(v) => !readonly && update({ dispatch_mode: v })}
              options={[
                { value: 'manual', label: '手动 manual' },
                { value: 'auto', label: '自动 auto' },
              ]}
            />
          </Field>
        </div>
        <Field label="可触发角色" hint="至少一项；定义谁能触发本工作流的实例">
          <div className="row wrap gap8">
            {triggerable.map((r) => {
              const on = tr.includes(r.code)
              return (
                <button
                  key={r.code}
                  disabled={readonly}
                  onClick={() => toggleTrigger(r.code)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 7,
                    padding: '6px 12px',
                    borderRadius: 999,
                    fontSize: 12.5,
                    fontWeight: 540,
                    border: `1px solid ${on ? 'var(--accent)' : 'var(--border-strong)'}`,
                    background: on ? 'var(--accent-weak)' : 'var(--surface)',
                    color: on ? 'var(--accent-text)' : 'var(--text-2)',
                    cursor: readonly ? 'default' : 'pointer',
                  }}
                >
                  {on ? <I.check size={13} /> : <I.plus size={13} />}
                  {r.name}
                </button>
              )
            })}
          </div>
        </Field>
      </div>
    </SectionCard>
  )
}

// ---------- Stages ----------
function StagesSection({
  draft,
  readonly,
  updateStage,
  reorder,
  addStage,
  delStage,
  setEditStage,
}: {
  draft: EditorWorkflow
  readonly: boolean
  updateStage: (id: number, p: Partial<EditorStage>) => void
  reorder: (from: number, to: number) => void
  addStage: () => void
  delStage: (id: number) => void
  setEditStage: (id: number) => void
}) {
  const { data: roles } = useRoles()
  const roleName = useRoleName()
  const dragIdx = useRef<number | null>(null)
  const [dragOver, setDragOver] = useState<number | null>(null)
  const stageById = (id: number) => draft.stages.find((s) => s.id === id)
  const depNames = (deps: number[]) => (deps.length ? deps.map((d) => stageById(d)?.seq ?? '?').join(', ') : '—')
  const roleOpts = (roles || []).filter((r) => r.code !== 'scheduler').map((r) => ({ value: r.code, label: r.name }))

  return (
    <SectionCard
      title="阶段"
      subtitle={readonly ? '只读' : '拖动手柄排序 · 点行末 ✎ 编辑阶段详情（依赖 / 闸 / 三栏文档）'}
      actions={!readonly && <Btn variant="default" size="sm" icon={I.plus} onClick={addStage}>加阶段</Btn>}
      bodyStyle={{ padding: 0 }}
    >
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ color: 'var(--text-3)', fontSize: 11, textTransform: 'uppercase', letterSpacing: '.03em' }}>
            <Th style={{ paddingLeft: 14, width: 28 }} />
            <Th style={{ width: 40 }}>#</Th>
            <Th>code</Th>
            <Th>名称</Th>
            <Th>责任角色</Th>
            <Th>产出</Th>
            <Th center>合流</Th>
            <Th>依赖</Th>
            <Th right style={{ paddingRight: 14 }} />
          </tr>
        </thead>
        <tbody>
          {draft.stages.map((s, idx) => (
            <tr
              key={s.id}
              draggable={!readonly}
              onDragStart={() => (dragIdx.current = idx)}
              onDragOver={(e) => {
                e.preventDefault()
                setDragOver(idx)
              }}
              onDragEnd={() => {
                if (dragIdx.current != null && dragOver != null) reorder(dragIdx.current, dragOver)
                dragIdx.current = null
                setDragOver(null)
              }}
              style={{ borderTop: '1px solid var(--border)', background: dragOver === idx && dragIdx.current != null ? 'var(--accent-weak)' : 'transparent' }}
              className="hover-row"
            >
              <Td style={{ paddingLeft: 14 }}>{!readonly && <span style={{ cursor: 'grab', color: 'var(--text-3)', display: 'flex' }}><I.drag size={16} /></span>}</Td>
              <Td>
                <span className="mono" style={{ fontWeight: 600 }}>
                  {s.seq}
                </span>
              </Td>
              <Td>
                <span className="mono t2" style={{ fontSize: 12 }}>
                  {s.code}
                </span>
              </Td>
              <Td>
                <span className="b">{s.name}</span>
              </Td>
              <Td>
                {readonly ? (
                  <span className="t2">{roleName(s.role_code)}</span>
                ) : (
                  <div style={{ width: 120 }}>
                    <Select value={s.role_code} onChange={(v) => updateStage(s.id, { role_code: v })} options={roleOpts} sm />
                  </div>
                )}
              </Td>
              <Td>
                {readonly ? (
                  <span className="t2 sm">{s.output_type}</span>
                ) : (
                  <div style={{ width: 104 }}>
                    <Select value={s.output_type} onChange={(v) => updateStage(s.id, { output_type: v })} options={['资讯包', '选题成品', '文章', '封面', '成品', '发布物', '小红书文本', '卡片', '素材包', '提纲', '初稿', '配图']} sm />
                  </div>
                )}
              </Td>
              <Td center>{s.is_merge ? <Badge tone="violet" sm>合流</Badge> : <span className="t3">—</span>}</Td>
              <Td>
                <span className="mono t2 sm">{depNames(s.deps)}</span>
              </Td>
              <Td right style={{ paddingRight: 10 }}>
                <div className="row gap2" style={{ justifyContent: 'flex-end' }}>
                  <IconBtn icon={I.edit} title="编辑阶段" onClick={() => setEditStage(s.id)} />
                  {!readonly && draft.stages.length > 1 && <IconBtn icon={I.x} title="删除阶段" danger onClick={() => delStage(s.id)} />}
                </div>
              </Td>
            </tr>
          ))}
        </tbody>
      </table>
      {draft.stages.length === 0 && (
        <EmptyState icon={I.workflow} title="还没有阶段" desc="至少需要 1 个阶段" compact action={!readonly && <Btn size="sm" variant="primary" icon={I.plus} onClick={addStage}>加阶段</Btn>} />
      )}
    </SectionCard>
  )
}

// ---------- Gates（默认闸可编辑 + 阶段覆盖只读展示） ----------
function GatesSection({ draft, update, readonly, setEditStage }: { draft: EditorWorkflow; update: (p: Partial<EditorWorkflow>) => void; readonly: boolean; setEditStage: (id: number) => void }) {
  const { data: roles } = useRoles()
  const roleName = useRoleName()
  const overrides = draft.stages.filter((s) => s.gate_override)
  const gateRoleOpts = (roles || []).filter((r) => r.is_management || !r.is_human).map((r) => ({ value: r.code, label: r.name }))
  const gates = draft.default_gates
  const updGate = (i: number, patch: Partial<EditorGate>) => update({ default_gates: gates.map((g, j) => (j === i ? { ...g, ...patch } : g)) })
  const addGate = () => update({ default_gates: [...gates, { gate_order: gates.length + 1, reviewer_role: 'editor', relayed: false, name: '新审核' }] })
  const delGate = (i: number) => update({ default_gates: gates.filter((_, j) => j !== i).map((g, k) => ({ ...g, gate_order: k + 1 })) })

  return (
    <div className="col gap20">
      <SectionCard title="默认闸" subtitle="套用到所有未单独配闸的阶段" actions={!readonly && <Btn variant="default" size="sm" icon={I.plus} onClick={addGate}>加一道闸</Btn>}>
        {gates.length === 0 ? (
          <EmptyState icon={I.checkCircle} title="暂无默认闸" desc="点右上「加一道闸」配置；至少一道（如 主编审 / Van 审）" compact />
        ) : (
          <div className="col gap10">
            {gates.map((g, i) => (
              <div key={i} className="row gap8" style={{ padding: '9px 11px', border: '1px solid var(--border)', borderRadius: 8, alignItems: 'center' }}>
                <span className="mono b sm" style={{ width: 18, color: 'var(--accent-text)' }}>
                  {g.gate_order}
                </span>
                <div style={{ width: 150 }}>
                  <Select value={g.reviewer_role} onChange={(v) => updGate(i, { reviewer_role: v })} options={gateRoleOpts} disabled={readonly} sm />
                </div>
                <div className="grow">
                  <TextInput value={g.name} onChange={(v) => updGate(i, { name: v })} readOnly={readonly} style={{ padding: '5px 10px', fontSize: 12.5 }} />
                </div>
                <label className="row gap5 sm" style={{ whiteSpace: 'nowrap' }}>
                  <Checkbox checked={g.relayed} onChange={(v) => !readonly && updGate(i, { relayed: v })} disabled={readonly} />
                  <span className="xs t2">人工代录</span>
                </label>
                {!readonly && <IconBtn icon={I.x} danger title="删除闸" onClick={() => delGate(i)} />}
              </div>
            ))}
          </div>
        )}
      </SectionCard>
      <SectionCard title="阶段覆盖" subtitle="个别阶段可单独配闸，覆盖默认；其余沿用默认闸">
        {overrides.length === 0 ? (
          <EmptyState icon={I.checkCircle} title="无阶段覆盖" desc="所有阶段均使用默认闸。在「阶段」中点 ✎ 可为某阶段单独配闸。" compact />
        ) : (
          <div className="col gap10">
            {overrides.map((s) => (
              <div key={s.id} className="row between" style={{ padding: '11px 14px', border: '1px solid var(--border)', borderRadius: 8 }}>
                <div className="col" style={{ gap: 3 }}>
                  <span className="b" style={{ fontSize: 13 }}>
                    {s.name}
                  </span>
                  <div className="row gap6 wrap">
                    {(s.gate_override || []).map((g, i) => (
                      <Badge key={i} tone="violet" sm>
                        {g.gate_order}. {g.name}
                        {g.relayed ? ' ·代录' : ''}
                      </Badge>
                    ))}
                  </div>
                </div>
                <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => setEditStage(s.id)}>
                  编辑
                </Btn>
              </div>
            ))}
          </div>
        )}
      </SectionCard>
      <div className="t3 xs">默认闸的 reviewer 角色须存在；人工代录表示结论由中枢（主编）代录。{roleName(draft.hub_role)} 为本工作流中枢。</div>
    </div>
  )
}

// ---------- Common ----------
function CommonSection({ draft, update, readonly }: { draft: EditorWorkflow; update: (p: Partial<EditorWorkflow>) => void; readonly: boolean }) {
  const [tab, setTab] = useState('inst')
  const val = tab === 'inst' ? draft.common_instructions : draft.common_acceptance
  const key = tab === 'inst' ? 'common_instructions' : 'common_acceptance'
  return (
    <SectionCard title="通用约定" subtitle="作为各阶段作业手册 / 验收标准的公共部分，随派工 / 审核一并下发">
      <Tabs
        tabs={[
          { value: 'inst', label: '通用约定 common_instructions' },
          { value: 'acc', label: '通用合规项 common_acceptance' },
        ]}
        value={tab}
        onChange={setTab}
      />
      <div style={{ marginTop: 14 }}>
        {readonly ? (
          <div style={{ border: '1px solid var(--border)', borderRadius: 8, padding: 14, background: 'var(--surface-2)' }}>
            <MarkdownView text={val || '（空）'} />
          </div>
        ) : (
          <Textarea value={val} onChange={(v) => update({ [key]: v } as Partial<EditorWorkflow>)} rows={7} mono placeholder="Markdown…" />
        )}
      </div>
    </SectionCard>
  )
}

const SECTIONS = [
  { key: 'basic', label: '基本信息', icon: I.doc },
  { key: 'stages', label: '阶段', icon: I.workflow },
  { key: 'dag', label: '依赖关系 (DAG)', icon: I.flow },
  { key: 'gates', label: '审核闸', icon: I.checkCircle },
  { key: 'common', label: '通用约定', icon: I.audit },
]

export function WorkflowEditor() {
  const { id } = useParams()
  const wfId = Number(id)
  const navigate = useNavigate()
  const qc = useQueryClient()
  const { user } = useAuth()
  const [searchParams] = useSearchParams()

  const { data, isLoading } = useQuery({ queryKey: ['workflow', wfId], queryFn: () => api.getWorkflow(wfId), enabled: !!wfId })

  const [draft, setDraft] = useState<EditorWorkflow | null>(null)
  const [section, setSection] = useState('basic')
  const [editStage, setEditStage] = useState<number | null>(null)
  const [dirty, setDirty] = useState(false)
  const [validateOpen, setValidateOpen] = useState(false)
  const [discardOpen, setDiscardOpen] = useState(false)

  useEffect(() => {
    if (data) setDraft(apiToEditor(data))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data?.workflow.id])

  useEffect(() => {
    if (searchParams.get('activate') === '1') setValidateOpen(true)
  }, [searchParams])

  // 归档版本、访客只读；draft 与 active 均可编辑（active 为就地热改）。
  const readonly = !draft || draft.status === 'archived' || user?.role === 'viewer'

  const save = useMutation({
    mutationFn: () => api.putWorkflow(wfId, editorToPutBody(draft!)),
    onSuccess: (res: { validation?: { all_ok: boolean; checks: { name: string; ok: boolean }[] } } | undefined) => {
      setDirty(false)
      qc.invalidateQueries({ queryKey: ['workflows'] })
      const v = res?.validation
      if (v && !v.all_ok) {
        const failed = v.checks.filter((c) => !c.ok).map((c) => c.name).join('、')
        toast('已保存，但校验未通过', 'warn', `触发前请修复：${failed}`)
      } else if (draft?.status === 'active') {
        toast('已保存', 'success', '改动即时生效于此后新触发的实例；已在跑的不受影响')
      } else {
        toast('草稿已保存', 'success', '整份阶段 / 依赖 / 闸 / 文本已提交')
      }
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  const clone = useMutation({
    mutationFn: () => api.cloneWorkflow(wfId),
    onSuccess: (newId) => {
      qc.invalidateQueries({ queryKey: ['workflows'] })
      toast('已复制为新草稿版本', 'success')
      navigate(`/workflows/${newId}/edit`)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  const activate = useMutation({
    mutationFn: async () => {
      await api.putWorkflow(wfId, editorToPutBody(draft!))
      return api.activateWorkflow(wfId)
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['workflows'] })
      setValidateOpen(false)
      toast('工作流已激活', 'success', `${draft?.name} v${draft?.version} · 旧版自动归档`)
      navigate('/workflows')
    },
    onError: (e) => {
      setValidateOpen(false)
      toast(getApiError(e).message, 'error')
    },
  })

  if (isLoading || !draft) {
    return (
      <div className="row center" style={{ flex: 1, color: 'var(--text-3)', fontSize: 13.5 }}>
        加载中…
      </div>
    )
  }

  const update = (patch: Partial<EditorWorkflow>) => {
    setDraft((d) => (d ? { ...d, ...patch } : d))
    setDirty(true)
  }
  const updateStage = (sid: number, patch: Partial<EditorStage>) => {
    setDraft((d) => (d ? { ...d, stages: d.stages.map((s) => (s.id === sid ? { ...s, ...patch } : s)) } : d))
    setDirty(true)
  }
  const reorder = (from: number, to: number) => {
    if (from === to) return
    setDraft((d) => {
      if (!d) return d
      const arr = [...d.stages]
      const [m] = arr.splice(from, 1)
      arr.splice(to, 0, m)
      arr.forEach((s, i) => (s.seq = i + 1))
      return { ...d, stages: arr }
    })
    setDirty(true)
  }
  const addStage = () => {
    setDraft((d) => {
      if (!d) return d
      const maxId = Math.max(1000, ...d.stages.map((s) => s.id))
      const n = d.stages.length + 1
      const ns: EditorStage = {
        id: maxId + 1,
        seq: n,
        code: `stage_${n}`,
        name: `${String(n).padStart(2, '0')}-新阶段`,
        role_code: 'writer',
        output_type: '成品',
        is_merge: false,
        dispatch_mode: '',
        action_class: 'read',
        sla_minutes: 0,
        per_item: false,
        deps: d.stages.length ? [d.stages[d.stages.length - 1].id] : [],
        instructions: '',
        self_check: '',
        acceptance: '',
        gate_override: null,
      }
      setEditStage(ns.id)
      return { ...d, stages: [...d.stages, ns] }
    })
    setDirty(true)
  }
  const delStage = (sid: number) => {
    setDraft((d) => (d ? { ...d, stages: d.stages.filter((s) => s.id !== sid).map((s) => ({ ...s, deps: s.deps.filter((x) => x !== sid) })) } : d))
    setDirty(true)
  }
  const stageById = (sid: number) => draft.stages.find((s) => s.id === sid)

  const leave = () => {
    if (dirty) setDiscardOpen(true)
    else navigate('/workflows')
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div style={{ flex: 1, overflowY: 'auto' }}>
        <div style={{ maxWidth: 1180, margin: '0 auto', padding: '24px 32px 120px' }}>
          <div className="row between wrap" style={{ gap: 14, marginBottom: 8 }}>
            <div className="col" style={{ gap: 6 }}>
              <button className="row gap6 t3 sm" style={{ border: 'none', background: 'transparent', padding: 0, width: 'fit-content', cursor: 'pointer' }} onClick={leave}>
                <I.chevLeft size={14} />
                工作流
              </button>
              <div className="row gap12" style={{ alignItems: 'center' }}>
                <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650 }}>{draft.name}</h1>
                <StatusBadge status={draft.status} />
                <span className="t3 mono sm">v{draft.version}</span>
              </div>
            </div>
          </div>

          {readonly && (
            <div className="row gap8" style={{ background: 'var(--amber-bg)', border: '1px solid var(--amber-bd)', color: 'var(--amber)', padding: '9px 13px', borderRadius: 8, fontSize: 12.5, marginBottom: 18 }}>
              <I.lock size={15} />
              {user?.role === 'viewer' ? (
                '访客为只读权限，无法编辑工作流。'
              ) : (
                <>
                  该版本已 <b>归档</b>，为只读查看。如需复用，请「复制为新版本」生成草稿。
                </>
              )}
            </div>
          )}

          {!readonly && draft.status === 'active' && (
            <div className="row gap8" style={{ background: 'var(--accent-weak)', border: '1px solid var(--border-strong)', color: 'var(--accent-text)', padding: '9px 13px', borderRadius: 8, fontSize: 12.5, marginBottom: 18 }}>
              <I.bolt size={15} />
              该工作流<b>已激活</b>，可直接修改并保存（就地生效）。改动只影响此后新触发的实例，已在跑的实例在触发时已快照定义、不受影响。保存时会校验，未过仅提示不阻断。
            </div>
          )}

          <div style={{ display: 'grid', gridTemplateColumns: '190px 1fr', gap: 26, alignItems: 'start', marginTop: 14 }}>
            <div style={{ position: 'sticky', top: 14 }}>
              <div className="col gap2">
                {SECTIONS.map((sec) => (
                  <button
                    key={sec.key}
                    onClick={() => setSection(sec.key)}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 9,
                      padding: '8px 10px',
                      borderRadius: 7,
                      border: 'none',
                      textAlign: 'left',
                      background: section === sec.key ? 'var(--accent-weak)' : 'transparent',
                      color: section === sec.key ? 'var(--accent-text)' : 'var(--text-2)',
                      fontSize: 13,
                      fontWeight: section === sec.key ? 600 : 500,
                    }}
                    onMouseEnter={(e) => {
                      if (section !== sec.key) e.currentTarget.style.background = 'var(--surface-2)'
                    }}
                    onMouseLeave={(e) => {
                      if (section !== sec.key) e.currentTarget.style.background = 'transparent'
                    }}
                  >
                    <sec.icon size={16} />
                    <span className="grow">{sec.label}</span>
                    {sec.key === 'stages' && <span className="mono xs t3">{draft.stages.length}</span>}
                  </button>
                ))}
              </div>
            </div>

            <div className="col gap20" style={{ minWidth: 0 }}>
              {section === 'basic' && <BasicSection draft={draft} update={update} readonly={readonly} />}
              {section === 'stages' && (
                <StagesSection draft={draft} readonly={readonly} updateStage={updateStage} reorder={reorder} addStage={addStage} delStage={delStage} setEditStage={setEditStage} />
              )}
              {section === 'dag' && (
                <SectionCard title="依赖关系（DAG）" subtitle="由阶段 + 依赖实时渲染。高亮入口 / 终点，有环时红色标出。">
                  <DagView stages={draft.stages} />
                </SectionCard>
              )}
              {section === 'gates' && <GatesSection draft={draft} update={update} readonly={readonly} setEditStage={setEditStage} />}
              {section === 'common' && <CommonSection draft={draft} update={update} readonly={readonly} />}
            </div>
          </div>
        </div>
      </div>

      {/* sticky action bar */}
      <div className="row between" style={{ flexShrink: 0, padding: '12px 32px', borderTop: '1px solid var(--border)', background: 'rgba(255,255,255,.86)', backdropFilter: 'blur(8px)' }}>
        <div className="row gap8">
          {!readonly && (
            <Btn variant="ghost" onClick={leave}>
              放弃修改
            </Btn>
          )}
          {readonly && (
            <Btn variant="default" icon={I.chevLeft} onClick={() => navigate('/workflows')}>
              返回列表
            </Btn>
          )}
          {dirty && (
            <span className="row gap6 t3 sm">
              <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--amber)' }} />
              有未保存的修改
            </span>
          )}
        </div>
        {!readonly && (
          <div className="row gap8">
            {draft.status === 'active' && (
              <Btn variant="ghost" icon={I.copy} onClick={() => clone.mutate()} disabled={clone.isPending}>
                另存为新版本
              </Btn>
            )}
            <Btn variant="default" icon={I.checkCircle} onClick={() => setValidateOpen(true)}>
              校验
            </Btn>
            {draft.status === 'draft' ? (
              <>
                <Btn variant="default" onClick={() => save.mutate()} disabled={save.isPending}>
                  保存草稿
                </Btn>
                <Btn variant="primary" icon={I.bolt} onClick={() => setValidateOpen(true)}>
                  校验并激活
                </Btn>
              </>
            ) : (
              <Btn variant="primary" onClick={() => save.mutate()} disabled={save.isPending}>
                保存
              </Btn>
            )}
          </div>
        )}
        {readonly && user?.role !== 'viewer' && (
          <Btn variant="default" icon={I.copy} onClick={() => clone.mutate()} disabled={clone.isPending}>
            复制为新版本
          </Btn>
        )}
      </div>

      {editStage != null && stageById(editStage) && (
        <StageDrawer
          stage={stageById(editStage)!}
          allStages={draft.stages}
          defaultGates={draft.default_gates}
          readonly={readonly}
          onClose={() => setEditStage(null)}
          onSave={(patch) => {
            updateStage(editStage, patch)
            setEditStage(null)
            toast('阶段已更新', 'success')
          }}
        />
      )}

      <ValidateModal
        open={validateOpen}
        onClose={() => setValidateOpen(false)}
        draft={draft}
        activating={activate.isPending}
        onActivate={() => activate.mutate()}
        gotoStage={(sid) => {
          setValidateOpen(false)
          setSection('stages')
          setEditStage(sid)
        }}
      />

      <ConfirmDialog
        open={discardOpen}
        onClose={() => setDiscardOpen(false)}
        danger
        title="放弃未保存的修改？"
        confirmText="放弃并离开"
        message="当前草稿有未保存的改动，离开后将丢失。确定要离开吗？"
        onConfirm={() => {
          setDiscardOpen(false)
          navigate('/workflows')
        }}
      />
    </div>
  )
}
