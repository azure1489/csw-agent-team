import { useState } from 'react'
import { I } from '@/components/icons'
import { Badge, Btn, Checkbox, Drawer, Field, IconBtn, MarkdownView, Radio, Select, Switch, Tabs, Textarea, TextInput } from '@/components/ui'
import { useRoles, useRoleName } from '@/hooks/useRoles'
import { OUTPUT_OPTS } from './lib'
import type { EditorGate, EditorStage } from '@/types'

const TABS = [
  { value: 'inst', label: '作业手册' },
  { value: 'self', label: '自检标准' },
  { value: 'acc', label: '验收标准' },
]
const TAB_HINT: Record<string, string> = {
  inst: 'Markdown：任务内容——做什么 / 工具 / 产出要求。派工时随单下发。',
  self: 'Markdown：交付前自查清单，全过才交。结果随 submit 申报。',
  acc: 'Markdown：主编 / Van 审核依据（与自检不同）。审核时随附。',
}

export function StageDrawer({
  stage,
  allStages,
  defaultGates,
  readonly,
  onClose,
  onSave,
}: {
  stage: EditorStage
  allStages: EditorStage[]
  defaultGates: EditorGate[]
  readonly: boolean
  onClose: () => void
  onSave: (s: EditorStage) => void
}) {
  const { data: roles } = useRoles()
  const roleName = useRoleName()
  const [s, setS] = useState<EditorStage>(() => JSON.parse(JSON.stringify(stage)))
  const [tab, setTab] = useState('inst')
  const [preview, setPreview] = useState(false)
  const set = (patch: Partial<EditorStage>) => setS((x) => ({ ...x, ...patch }))
  const others = allStages.filter((x) => x.id !== s.id)
  const customGates = !!s.gate_override

  const toggleDep = (id: number) => set({ deps: s.deps.includes(id) ? s.deps.filter((x) => x !== id) : [...s.deps, id] })
  const setGateMode = (custom: boolean) => set({ gate_override: custom ? s.gate_override || defaultGates.map((g) => ({ ...g })) : null })
  const updGate = (i: number, patch: Partial<EditorGate>) => set({ gate_override: (s.gate_override || []).map((g, j) => (j === i ? { ...g, ...patch } : g)) })
  const addGate = () => set({ gate_override: [...(s.gate_override || []), { gate_order: (s.gate_override || []).length + 1, reviewer_role: 'editor', relayed: false, name: '新审核' }] })
  const delGate = (i: number) => set({ gate_override: (s.gate_override || []).filter((_, j) => j !== i).map((g, k) => ({ ...g, gate_order: k + 1 })) })

  const fieldKey = tab === 'inst' ? 'instructions' : tab === 'self' ? 'self_check' : 'acceptance'
  const fieldVal = s[fieldKey as 'instructions' | 'self_check' | 'acceptance']
  const gateRoleOpts = (roles || []).filter((r) => r.is_management || !r.is_human).map((r) => ({ value: r.code, label: r.name }))

  return (
    <Drawer
      open
      onClose={onClose}
      width={620}
      title={`阶段：${s.name}`}
      subtitle={`${s.code} · 责任角色 ${roleName(s.role_code)}`}
      badge={s.is_merge ? <Badge tone="violet" sm>合流</Badge> : undefined}
      footer={
        <>
          <Btn variant="ghost" onClick={onClose}>
            取消
          </Btn>
          {!readonly && (
            <Btn variant="primary" onClick={() => onSave(s)}>
              保存阶段
            </Btn>
          )}
        </>
      }
    >
      <div className="col gap24">
        {/* basic toggles */}
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="名称">
            <TextInput value={s.name} onChange={(v) => set({ name: v })} readOnly={readonly} />
          </Field>
          <Field label="产出类型">
            <Select value={s.output_type} onChange={(v) => set({ output_type: v })} options={OUTPUT_OPTS} disabled={readonly} />
          </Field>
        </div>
        <div className="row gap16">
          <div className="row gap8">
            <Switch checked={s.is_merge} onChange={(v) => set({ is_merge: v })} disabled={readonly} />
            <span className="sm">合流阶段（由中枢合成，自动派工给自己）</span>
          </div>
        </div>

        {/* deps */}
        <Field label="依赖上游" hint="勾选构成 DAG 边。不能选自己，保存激活时校验不成环。">
          {others.length === 0 ? (
            <span className="t3 sm">没有其他阶段</span>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, border: '1px solid var(--border)', borderRadius: 8, padding: 12 }}>
              {others.map((o) => (
                <Checkbox
                  key={o.id}
                  checked={s.deps.includes(o.id)}
                  onChange={() => !readonly && toggleDep(o.id)}
                  disabled={readonly}
                  label={
                    <span>
                      <span className="mono t3" style={{ marginRight: 5 }}>
                        {o.seq}
                      </span>
                      {o.name}
                    </span>
                  }
                />
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
              <div className="col gap8" style={{ border: '1px dashed var(--border-2)', borderRadius: 8, padding: 12, background: 'var(--surface-2)' }}>
                {defaultGates.map((g, i) => (
                  <div key={i} className="row gap10">
                    <span className="mono b sm" style={{ color: 'var(--accent-text)' }}>
                      {g.gate_order}.
                    </span>
                    <span className="sm">{g.name}</span>
                    <span className="t3 xs">
                      （{roleName(g.reviewer_role)}
                      {g.relayed ? ' · 人工代录' : ''}）
                    </span>
                  </div>
                ))}
                {defaultGates.length === 0 && <span className="t3 xs">工作流暂无默认闸</span>}
                <span className="t3 xs">沿用默认闸，未覆盖。</span>
              </div>
            ) : (
              <div className="col gap8">
                {(s.gate_override || []).map((g, i) => (
                  <div key={i} className="row gap8" style={{ border: '1px solid var(--border)', borderRadius: 8, padding: '9px 11px', alignItems: 'center' }}>
                    <span className="mono b sm" style={{ width: 16, color: 'var(--accent-text)' }}>
                      {g.gate_order}
                    </span>
                    <div style={{ width: 132 }}>
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
                {!readonly && (
                  <Btn variant="ghost" size="sm" icon={I.plus} onClick={addGate} style={{ width: 'fit-content' }}>
                    加一道闸
                  </Btn>
                )}
              </div>
            )}
          </div>
        </Field>

        {/* three markdown editors */}
        <div className="col gap10">
          <div className="row between">
            <Tabs tabs={TABS} value={tab} onChange={setTab} />
            <Btn variant="ghost" size="sm" icon={preview ? I.edit : I.eye} onClick={() => setPreview((p) => !p)}>
              {preview ? '编辑' : '预览'}
            </Btn>
          </div>
          <div className="t3 xs">{TAB_HINT[tab]}</div>
          {preview || readonly ? (
            <div style={{ border: '1px solid var(--border)', borderRadius: 8, padding: 14, background: 'var(--surface-2)', minHeight: 180 }}>
              <MarkdownView text={fieldVal || '（空）'} />
            </div>
          ) : (
            <Textarea value={fieldVal || ''} onChange={(v) => set({ [fieldKey]: v } as Partial<EditorStage>)} rows={10} mono />
          )}
        </div>
      </div>
    </Drawer>
  )
}
