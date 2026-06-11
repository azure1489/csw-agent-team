import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { I } from '@/components/icons'
import {
  Badge,
  Btn,
  Card,
  ConfirmDialog,
  EmptyState,
  Field,
  IconBtn,
  Menu,
  Modal,
  PageHeader,
  Segmented,
  Select,
  StatusBadge,
  Td,
  TextInput,
  Th,
  toast,
} from '@/components/ui'
import { api, getApiError } from '@/lib/api'
import { useRoleName, useRoles } from '@/hooks/useRoles'
import { useAuth } from '@/lib/auth'
import type { WorkflowListItem } from '@/types'

function WorkflowJsonModal({ id, name, version, onClose }: { id: number; name: string; version: number; onClose: () => void }) {
  const { data } = useQuery({ queryKey: ['workflow', id], queryFn: () => api.getWorkflow(id) })
  const json = data
    ? JSON.stringify(
        {
          ...data.workflow,
          default_gates: data.default_gates,
          stages: data.stages.map((s) => ({ seq: s.seq, code: s.code, name: s.name, role_code: s.role_code, output_type: s.output_type, is_merge: s.is_merge, deps: s.deps })),
        },
        null,
        2
      )
    : '加载中…'
  return (
    <Modal open onClose={onClose} title={`工作流 JSON · ${name} v${version}`} width={620} icon={I.doc}>
      <pre className="mono" style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 8, padding: 14, fontSize: 11.5, lineHeight: 1.6, overflow: 'auto', maxHeight: 460, margin: '4px 0 14px' }}>
        {json}
      </pre>
    </Modal>
  )
}

function NewWorkflowModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const navigate = useNavigate()
  const qc = useQueryClient()
  const { data: roles } = useRoles()
  const [key, setKey] = useState('')
  const [name, setName] = useState('')
  const [hub, setHub] = useState('editor')
  const [mode, setMode] = useState('manual')
  const mgmtRoles = (roles || []).filter((r) => r.is_management)
  const valid = !!key && !!name

  const create = useMutation({
    mutationFn: () => api.createWorkflow({ wf_key: key, name, hub_role: hub, dispatch_mode: mode, trigger_roles: '' }),
    onSuccess: (id) => {
      qc.invalidateQueries({ queryKey: ['workflows'] })
      onClose()
      toast('草稿已创建', 'success', '进入编辑器配置阶段')
      navigate(`/workflows/${id}/edit`)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="新建工作流"
      subtitle="创建一个草稿版本，随后进入编辑器配置阶段与依赖"
      width={460}
      icon={I.plus}
      footer={
        <>
          <span />
          <div className="row gap8">
            <Btn onClick={onClose}>取消</Btn>
            <Btn variant="primary" disabled={!valid || create.isPending} onClick={() => create.mutate()}>
              创建草稿
            </Btn>
          </div>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="key（机器键）" hint="创建后只读，建议小写下划线，如 daily_news" required>
          <TextInput value={key} onChange={(v) => setKey(v.replace(/[^a-z0-9_]/g, ''))} placeholder="daily_news" mono />
        </Field>
        <Field label="名称" required>
          <TextInput value={name} onChange={setName} placeholder="资讯日更" />
        </Field>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="中枢角色" hint="须为管理类角色">
            <Select value={hub} onChange={setHub} options={mgmtRoles.map((r) => ({ value: r.code, label: r.name }))} />
          </Field>
          <Field label="派工模式">
            <Segmented value={mode} onChange={setMode} options={[{ value: 'manual', label: '手动' }, { value: 'auto', label: '自动' }]} />
          </Field>
        </div>
      </div>
    </Modal>
  )
}

export function Workflows() {
  const navigate = useNavigate()
  const qc = useQueryClient()
  const { user } = useAuth()
  const canWrite = user?.role !== 'viewer'
  const roleName = useRoleName()

  const [search, setSearch] = useState('')
  const [status, setStatus] = useState('')
  const [confirm, setConfirm] = useState<WorkflowListItem | null>(null)
  const [jsonView, setJsonView] = useState<WorkflowListItem | null>(null)
  const [newOpen, setNewOpen] = useState(false)

  const { data: workflows } = useQuery({ queryKey: ['workflows'], queryFn: () => api.listWorkflows() })

  const rows = useMemo(() => {
    return (workflows || [])
      .filter((w) => !search || w.name.includes(search) || w.wf_key.includes(search))
      .filter((w) => !status || w.status === status)
      .sort((a, b) => a.wf_key.localeCompare(b.wf_key) || b.version - a.version)
  }, [workflows, search, status])

  const clone = useMutation({
    mutationFn: (id: number) => api.cloneWorkflow(id),
    onSuccess: (newId, _id) => {
      qc.invalidateQueries({ queryKey: ['workflows'] })
      toast('已复制为新草稿版本', 'success')
      navigate(`/workflows/${newId}/edit`)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  const archive = useMutation({
    mutationFn: (id: number) => api.archiveWorkflow(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['workflows'] })
      toast('已归档', 'success')
      setConfirm(null)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="工作流"
          desc="浏览所有工作流类型及其版本。草稿 / 激活版本可直接编辑（激活为就地热改）；归档版本只读查看。"
          actions={
            canWrite && (
              <Btn variant="primary" icon={I.plus} onClick={() => setNewOpen(true)}>
                新建工作流
              </Btn>
            )
          }
        />

        <Card noPad>
          <div className="row between" style={{ padding: '12px 14px', borderBottom: '1px solid var(--border)', gap: 12 }}>
            <div style={{ width: 280 }}>
              <TextInput value={search} onChange={setSearch} placeholder="搜索名称或 key…" icon={I.search} />
            </div>
            <div className="row gap8">
              <span className="t3 sm row gap4">
                <I.filter size={14} />
                筛选
              </span>
              <div style={{ width: 130 }}>
                <Select
                  value={status}
                  onChange={setStatus}
                  placeholder="全部状态"
                  options={[
                    { value: '', label: '全部状态' },
                    { value: 'active', label: '已激活' },
                    { value: 'draft', label: '草稿' },
                    { value: 'archived', label: '已归档' },
                  ]}
                  sm
                />
              </div>
            </div>
          </div>

          {rows.length === 0 ? (
            <EmptyState
              icon={I.workflow}
              title="还没有工作流"
              desc="点击右上角「新建工作流」创建第一个工作流定义"
              action={canWrite && <Btn variant="primary" icon={I.plus} onClick={() => setNewOpen(true)}>新建工作流</Btn>}
            />
          ) : (
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
              <thead>
                <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                  <Th style={{ paddingLeft: 18 }}>名称</Th>
                  <Th>key</Th>
                  <Th>版本</Th>
                  <Th>状态</Th>
                  <Th>中枢</Th>
                  <Th>派工</Th>
                  <Th center>阶段数</Th>
                  <Th right style={{ paddingRight: 18 }}>
                    操作
                  </Th>
                </tr>
              </thead>
              <tbody>
                {rows.map((w) => {
                  const editable = w.status === 'draft' || w.status === 'active'
                  return (
                    <tr key={w.id} style={{ borderTop: '1px solid var(--border)' }} className="hover-row">
                      <Td style={{ paddingLeft: 18 }}>
                        <span className="b">{w.name}</span>
                      </Td>
                      <Td>
                        <span className="mono t2" style={{ fontSize: 12.5 }}>
                          {w.wf_key}
                        </span>
                      </Td>
                      <Td>
                        <span className="mono" style={{ fontSize: 12.5, fontWeight: 600 }}>
                          v{w.version}
                        </span>
                      </Td>
                      <Td>
                        <StatusBadge status={w.status} />
                      </Td>
                      <Td>
                        <span className="t2">{roleName(w.hub_role)}</span>
                      </Td>
                      <Td>
                        <Badge tone={w.dispatch_mode === 'auto' ? 'blue' : 'gray'} sm>
                          {w.dispatch_mode === 'auto' ? (
                            <>
                              <I.bolt size={11} /> auto
                            </>
                          ) : (
                            <>
                              <I.hand size={11} /> manual
                            </>
                          )}
                        </Badge>
                      </Td>
                      <Td center>
                        <span className="mono t2">{w.stage_count}</span>
                      </Td>
                      <Td right style={{ paddingRight: 14 }}>
                        <div className="row gap4" style={{ justifyContent: 'flex-end' }}>
                          {canWrite && editable ? (
                            <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => navigate(`/workflows/${w.id}/edit`)}>
                              编辑
                            </Btn>
                          ) : (
                            <Btn variant="ghost" size="sm" icon={I.eye} onClick={() => navigate(`/workflows/${w.id}/edit`)}>
                              查看
                            </Btn>
                          )}
                          {canWrite && (
                            <Menu
                              trigger={<I.more size={18} />}
                              triggerClassName="hover-row"
                              triggerStyle={{ width: 30, height: 30, borderRadius: 7, justifyContent: 'center', color: 'var(--text-2)' }}
                              items={[
                                { icon: I.copy, label: '复制为新版本', onClick: () => clone.mutate(w.id) },
                                ...(w.status === 'draft' ? [{ icon: I.checkCircle, label: '校验并激活', onClick: () => navigate(`/workflows/${w.id}/edit?activate=1`) }] : []),
                                ...(w.status === 'active' ? [{ icon: I.doc, label: '归档', danger: true, onClick: () => setConfirm(w) }] : []),
                                { icon: I.doc, label: '查看 JSON', onClick: () => setJsonView(w) },
                              ]}
                            />
                          )}
                        </div>
                      </Td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          )}
        </Card>

        <ConfirmDialog
          open={!!confirm}
          onClose={() => setConfirm(null)}
          danger
          confirmText="确认归档"
          title="归档工作流？"
          message={
            confirm && (
              <>
                归档后{' '}
                <b>
                  {confirm.name} v{confirm.version}
                </b>{' '}
                将不可被触发，但在跑实例不受影响（已快照）。此操作会写入审计日志。
              </>
            )
          }
          onConfirm={() => confirm && archive.mutate(confirm.id)}
        />

        {jsonView && <WorkflowJsonModal id={jsonView.id} name={jsonView.name} version={jsonView.version} onClose={() => setJsonView(null)} />}

        <NewWorkflowModal open={newOpen} onClose={() => setNewOpen(false)} />
      </div>
    </div>
  )
}
