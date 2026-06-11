import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { I } from '@/components/icons'
import {
  Avatar,
  Badge,
  Btn,
  Card,
  Checkbox,
  ConfirmDialog,
  Drawer,
  Field,
  IconBtn,
  Menu,
  Modal,
  PageHeader,
  Select,
  StatusBadge,
  Tabs,
  Td,
  TextInput,
  Th,
  toast,
} from '@/components/ui'
import { api, getApiError } from '@/lib/api'
import { useRoles, useRoleName } from '@/hooks/useRoles'
import { useAuth } from '@/lib/auth'
import type { Agent, Role } from '@/types'

function expiresToISO(opt: string): string | undefined {
  const days = opt === '30 天' ? 30 : opt === '90 天' ? 90 : 0
  if (!days) return undefined
  const d = new Date()
  d.setDate(d.getDate() + days)
  return d.toISOString()
}

function RoleModal({ role, onClose }: { role: Role | 'new'; onClose: () => void }) {
  const qc = useQueryClient()
  const isNew = role === 'new'
  const [code, setCode] = useState(isNew ? '' : (role as Role).code)
  const [name, setName] = useState(isNew ? '' : (role as Role).name)
  const [mgmt, setMgmt] = useState(isNew ? false : (role as Role).is_management)
  const [human, setHuman] = useState(isNew ? false : (role as Role).is_human)

  const save = useMutation({
    mutationFn: () => (isNew ? api.createRole({ code, name, is_management: mgmt, is_human: human }) : api.updateRole((role as Role).code, { name, is_management: mgmt, is_human: human })),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['roles'] })
      onClose()
      toast(isNew ? '角色已创建' : '角色已更新', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <Modal
      open
      onClose={onClose}
      width={440}
      icon={I.plus}
      title={isNew ? '新建角色' : `编辑角色：${name}`}
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={!code || !name || save.isPending} onClick={() => save.mutate()}>
            {isNew ? '创建' : '保存'}
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="code" hint="创建后只读">
            <TextInput value={code} onChange={(v) => setCode(v.replace(/[^a-z0-9_]/g, ''))} mono placeholder="reviewer" readOnly={!isNew} />
          </Field>
          <Field label="名称">
            <TextInput value={name} onChange={setName} placeholder="审核员" />
          </Field>
        </div>
        <div className="col gap10" style={{ border: '1px solid var(--border)', borderRadius: 8, padding: 12 }}>
          <Checkbox checked={mgmt} onChange={setMgmt} label="管理类（可任中枢 / 审核 / 管理定义）" />
          <Checkbox checked={human} onChange={setHuman} label="人工角色（无 token，结论由中枢代录）" />
        </div>
      </div>
    </Modal>
  )
}

function NewAgentModal({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient()
  const { data: roles } = useRoles()
  const [name, setName] = useState('')
  const [role, setRole] = useState('collector')
  const create = useMutation({
    mutationFn: () => api.createAgent({ role_code: role, name }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      onClose()
      toast('成员已创建', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  return (
    <Modal
      open
      onClose={onClose}
      width={420}
      icon={I.plus}
      title="新建成员"
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={!name || create.isPending} onClick={() => create.mutate()}>
            创建
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="名称">
          <TextInput value={name} onChange={setName} placeholder="如 文案bot" />
        </Field>
        <Field label="角色">
          <Select value={role} onChange={setRole} options={(roles || []).map((r) => ({ value: r.code, label: `${r.name}（${r.code}）` }))} />
        </Field>
      </div>
    </Modal>
  )
}

function IssueTokenModal({ agent, onClose }: { agent: Agent; onClose: () => void }) {
  const qc = useQueryClient()
  const [label, setLabel] = useState('生产')
  const [exp, setExp] = useState('永不')
  const [issued, setIssued] = useState<string | null>(null)
  const issue = useMutation({
    mutationFn: () => api.issueToken(agent.id, { label, expires_at: expiresToISO(exp) }),
    onSuccess: (res) => {
      setIssued(res.token)
      qc.invalidateQueries({ queryKey: ['agents'] })
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  return (
    <Modal
      open
      onClose={onClose}
      width={460}
      icon={I.key}
      title={`签发 token：${agent.name}`}
      subtitle="明文仅显示一次，关闭后服务器只存 sha256 哈希"
      footer={
        issued ? (
          <>
            <span />
            <Btn variant="primary" onClick={onClose}>
              完成
            </Btn>
          </>
        ) : (
          <>
            <Btn onClick={onClose}>取消</Btn>
            <Btn variant="primary" icon={I.key} disabled={issue.isPending} onClick={() => issue.mutate()}>
              签发
            </Btn>
          </>
        )
      }
    >
      {!issued ? (
        <div className="col gap16" style={{ paddingBottom: 8 }}>
          <Field label="备注 label" hint="便于识别用途，如「生产」「轮换-2026Q2」">
            <TextInput value={label} onChange={setLabel} />
          </Field>
          <Field label="过期">
            <Select value={exp} onChange={setExp} options={['永不', '30 天', '90 天']} />
          </Field>
        </div>
      ) : (
        <div className="col gap14" style={{ paddingBottom: 8 }}>
          <div className="row gap8" style={{ background: 'var(--amber-bg)', border: '1px solid var(--amber-bd)', color: 'var(--amber)', padding: '10px 13px', borderRadius: 8, fontSize: 12.5 }}>
            <I.warn size={16} />
            <span>明文仅显示一次。关闭后无法再查看，请立即复制保存。</span>
          </div>
          <div className="row between" style={{ background: '#1f1f27', borderRadius: 9, padding: '13px 15px' }}>
            <span className="mono" style={{ color: '#9ef0c3', fontSize: 13.5, wordBreak: 'break-all' }}>
              {issued}
            </span>
            <Btn
              variant="default"
              size="sm"
              icon={I.copy}
              onClick={() => {
                navigator.clipboard?.writeText(issued)
                toast('已复制到剪贴板', 'success')
              }}
            >
              复制
            </Btn>
          </div>
          <div className="t3 xs">备注「{label}」 · 过期 {exp} · 服务器已保存哈希。</div>
        </div>
      )}
    </Modal>
  )
}

function AgentDetailDrawer({ agent, canWrite, onClose, onIssue, onRevoke }: { agent: Agent; canWrite: boolean; onClose: () => void; onIssue: (a: Agent) => void; onRevoke: (a: Agent, tid: number, label: string) => void }) {
  const qc = useQueryClient()
  const { data: roles } = useRoles()
  const roleName = useRoleName()
  const human = roles?.find((r) => r.code === agent.role_code)?.is_human
  const [name, setName] = useState(agent.name)
  const [roleCode, setRoleCode] = useState(agent.role_code)
  const dirty = name !== agent.name || roleCode !== agent.role_code

  const update = useMutation({
    mutationFn: () => api.updateAgent(agent.id, { name, role_code: roleCode, active: agent.active }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      toast('成员已更新', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <Drawer
      open
      onClose={onClose}
      width={560}
      title={`成员：${agent.name}`}
      subtitle={`${roleName(agent.role_code)} · ${agent.role_code}`}
      badge={<StatusBadge status={agent.active ? 'enabled' : 'disabled'} sm />}
      footer={canWrite && dirty ? <><Btn variant="ghost" onClick={onClose}>取消</Btn><Btn variant="primary" disabled={update.isPending} onClick={() => update.mutate()}>保存修改</Btn></> : undefined}
    >
      <div className="col gap24">
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="名称">
            <TextInput value={name} onChange={setName} readOnly={!canWrite} />
          </Field>
          <Field label="角色">
            <Select
              value={roleCode}
              disabled={!canWrite}
              onChange={setRoleCode}
              options={(roles || []).filter((r) => r.code !== 'scheduler' || agent.role_code === 'scheduler').map((r) => ({ value: r.code, label: r.name }))}
            />
          </Field>
        </div>
        {human ? (
          <div className="row gap8" style={{ background: 'var(--amber-bg)', border: '1px solid var(--amber-bd)', color: 'var(--amber)', padding: '10px 13px', borderRadius: 8, fontSize: 12.5 }}>
            <I.hand size={15} />
            人工角色不签发 token——其审核结论由中枢（主编）代录。
          </div>
        ) : (
          <div className="col gap10">
            <div className="row between">
              <span className="b" style={{ fontSize: 13.5 }}>
                Token
              </span>
              {canWrite && (
                <Btn variant="default" size="sm" icon={I.key} onClick={() => onIssue(agent)}>
                  签发新 token
                </Btn>
              )}
            </div>
            <div className="col gap8">
              {agent.tokens.length === 0 && <span className="t3 sm">暂无 token</span>}
              {agent.tokens.map((t) => (
                <div key={t.id} className="row between" style={{ border: '1px solid var(--border)', borderRadius: 8, padding: '10px 13px', opacity: t.status === 'revoked' ? 0.62 : 1 }}>
                  <div className="col" style={{ gap: 3 }}>
                    <div className="row gap8">
                      <span className="mono b sm" style={{ textDecoration: t.status === 'revoked' ? 'line-through' : 'none' }}>
                        #{t.id} {t.label}
                      </span>
                      <StatusBadge status={t.status === 'active' ? 'token_active' : t.status} sm />
                    </div>
                    <span className="t3 xs">
                      最近使用 {t.last_used_at || '—'} · 过期 {t.expires_at || '永不'}
                    </span>
                  </div>
                  {canWrite && t.status === 'active' && (
                    <Btn variant="ghost" size="sm" onClick={() => onRevoke(agent, t.id, t.label)} style={{ color: 'var(--red)' }}>
                      吊销
                    </Btn>
                  )}
                </div>
              ))}
            </div>
            <div className="t3 xs">轮换 = 先签发新 token，确认生效后再吊销旧的。吊销即时失效该 token 的运行面调用。</div>
          </div>
        )}
      </div>
    </Drawer>
  )
}

export function Members() {
  const qc = useQueryClient()
  const { user } = useAuth()
  const canWrite = user?.role !== 'viewer'
  const roleName = useRoleName()
  const [tab, setTab] = useState('agents')
  const [detail, setDetail] = useState<number | null>(null)
  const [issueFor, setIssueFor] = useState<Agent | null>(null)
  const [roleModal, setRoleModal] = useState<Role | 'new' | null>(null)
  const [newAgent, setNewAgent] = useState(false)
  const [confirm, setConfirm] = useState<{ type: 'revoke'; a: Agent; tid: number; label: string } | { type: 'disable'; a: Agent } | null>(null)

  const { data: roles } = useRoles()
  const { data: agents } = useQuery({ queryKey: ['agents'], queryFn: api.listAgents })
  const detailAgent = agents?.find((a) => a.id === detail)

  const toggleAgent = useMutation({
    mutationFn: (a: Agent) => api.updateAgent(a.id, { name: a.name, role_code: a.role_code, active: !a.active }),
    onSuccess: (_r, a) => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      toast(a.active ? '成员已禁用' : '已启用成员', 'success')
      setConfirm(null)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  const revoke = useMutation({
    mutationFn: ({ a, tid }: { a: Agent; tid: number }) => api.revokeToken(a.id, tid),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      toast('token 已吊销', 'success')
      setConfirm(null)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="角色与成员"
          desc="角色定义谁能做什么（管理 / 人工）；成员是具体 agent，bearer token 由后台签发 / 轮换 / 吊销。"
          actions={
            canWrite &&
            (tab === 'roles' ? (
              <Btn variant="primary" icon={I.plus} onClick={() => setRoleModal('new')}>
                新建角色
              </Btn>
            ) : (
              <Btn variant="primary" icon={I.plus} onClick={() => setNewAgent(true)}>
                新建成员
              </Btn>
            ))
          }
        />

        <div style={{ marginBottom: 16 }}>
          <Tabs
            tabs={[
              { value: 'agents', label: `成员 / Token (${agents?.length ?? 0})` },
              { value: 'roles', label: `角色 (${roles?.length ?? 0})` },
            ]}
            value={tab}
            onChange={setTab}
          />
        </div>

        {tab === 'roles' ? (
          <Card noPad>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
              <thead>
                <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                  <Th style={{ paddingLeft: 18 }}>code</Th>
                  <Th>名称</Th>
                  <Th center>管理类</Th>
                  <Th center>人工</Th>
                  <Th center>成员数</Th>
                  <Th right style={{ paddingRight: 18 }} />
                </tr>
              </thead>
              <tbody>
                {(roles || []).map((r) => (
                  <tr key={r.code} className="hover-row" style={{ borderTop: '1px solid var(--border)' }}>
                    <Td style={{ paddingLeft: 18 }}>
                      <span className="mono t2" style={{ fontSize: 12.5 }}>
                        {r.code}
                      </span>
                    </Td>
                    <Td>
                      <span className="b">{r.name}</span>
                    </Td>
                    <Td center>{r.is_management ? <I.check size={16} style={{ color: 'var(--green)' }} /> : <span className="t3">—</span>}</Td>
                    <Td center>{r.is_human ? <Badge tone="amber" sm>人工</Badge> : <span className="t3">—</span>}</Td>
                    <Td center>
                      <span className="mono t2">{r.member_count}</span>
                    </Td>
                    <Td right style={{ paddingRight: 14 }}>
                      {canWrite && (
                        <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => setRoleModal(r)}>
                          编辑
                        </Btn>
                      )}
                    </Td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="t3 xs" style={{ padding: '10px 18px', borderTop: '1px solid var(--border)' }}>
              管理类 → 可任中枢 / 审核 / 管理工作流定义；人工 → 无 token、结论由中枢代录。被工作流 / agent 引用的角色不可删。
            </div>
          </Card>
        ) : (
          <Card noPad>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
              <thead>
                <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                  <Th style={{ paddingLeft: 18 }}>名称</Th>
                  <Th>角色</Th>
                  <Th>有效 token</Th>
                  <Th>状态</Th>
                  <Th right style={{ paddingRight: 18 }}>
                    操作
                  </Th>
                </tr>
              </thead>
              <tbody>
                {(agents || []).map((a) => {
                  const valid = a.tokens.filter((t) => t.status === 'active').length
                  const human = roles?.find((r) => r.code === a.role_code)?.is_human
                  return (
                    <tr key={a.id} className="hover-row" style={{ borderTop: '1px solid var(--border)' }}>
                      <Td style={{ paddingLeft: 18 }}>
                        <span className="row gap8">
                          <Avatar name={a.name} size={24} />
                          <span className="b">{a.name}</span>
                        </span>
                      </Td>
                      <Td>
                        <span className="t2">{roleName(a.role_code)}</span> <span className="mono t3 xs">{a.role_code}</span>
                      </Td>
                      <Td>
                        {human ? (
                          <span className="t3 sm">无 token（结论代录）</span>
                        ) : (
                          <span className="row gap6">
                            <span className="mono b">{valid}</span>
                            <span className="t3 sm">枚{a.tokens.length > valid ? `（共 ${a.tokens.length}，轮换）` : ''}</span>
                          </span>
                        )}
                      </Td>
                      <Td>
                        <StatusBadge status={a.active ? 'enabled' : 'disabled'} />
                      </Td>
                      <Td right style={{ paddingRight: 14 }}>
                        <div className="row gap4" style={{ justifyContent: 'flex-end' }}>
                          <Btn variant="ghost" size="sm" onClick={() => setDetail(a.id)}>
                            详情
                          </Btn>
                          {canWrite && !human && (
                            <Menu
                              trigger={<I.more size={18} />}
                              triggerClassName="hover-row"
                              triggerStyle={{ width: 30, height: 30, borderRadius: 7, justifyContent: 'center', color: 'var(--text-2)' }}
                              items={[
                                { icon: I.key, label: '签发新 token', onClick: () => setIssueFor(a) },
                                {
                                  icon: a.active ? I.lock : I.check,
                                  label: a.active ? '禁用成员' : '启用成员',
                                  danger: a.active,
                                  onClick: () => (a.active ? setConfirm({ type: 'disable', a }) : toggleAgent.mutate(a)),
                                },
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
          </Card>
        )}

        {detailAgent && (
          <AgentDetailDrawer
            agent={detailAgent}
            canWrite={canWrite}
            onClose={() => setDetail(null)}
            onIssue={(a) => setIssueFor(a)}
            onRevoke={(a, tid, label) => setConfirm({ type: 'revoke', a, tid, label })}
          />
        )}

        {issueFor && <IssueTokenModal agent={issueFor} onClose={() => setIssueFor(null)} />}
        {roleModal && <RoleModal role={roleModal} onClose={() => setRoleModal(null)} />}
        {newAgent && <NewAgentModal onClose={() => setNewAgent(false)} />}

        <ConfirmDialog
          open={confirm?.type === 'revoke'}
          onClose={() => setConfirm(null)}
          danger
          confirmText="确认吊销"
          title="吊销 token？"
          message={
            confirm?.type === 'revoke' && (
              <>
                吊销{' '}
                <b>
                  #{confirm.tid} {confirm.label}
                </b>{' '}
                后，使用该 token 的运行面调用将<b>即时失效</b>。此操作不可恢复，会写入审计日志。
              </>
            )
          }
          onConfirm={() => confirm?.type === 'revoke' && revoke.mutate({ a: confirm.a, tid: confirm.tid })}
        />
        <ConfirmDialog
          open={confirm?.type === 'disable'}
          onClose={() => setConfirm(null)}
          danger
          confirmText="确认禁用"
          title="禁用成员？"
          message={
            confirm?.type === 'disable' && (
              <>
                禁用 <b>{confirm.a.name}</b> 后，其<b>所有 token 立即失效</b>，运行面将拒绝其调用。可随时重新启用。
              </>
            )
          }
          onConfirm={() => confirm?.type === 'disable' && toggleAgent.mutate(confirm.a)}
        />
      </div>
    </div>
  )
}
