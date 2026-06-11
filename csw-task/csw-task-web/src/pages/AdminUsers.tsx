import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { I } from '@/components/icons'
import { Avatar, Badge, Btn, Card, ConfirmDialog, Field, IconBtn, Menu, Modal, PageHeader, Select, StatusBadge, Td, TextInput, Th, toast } from '@/components/ui'
import { api, getApiError } from '@/lib/api'
import { ROLE_LABEL } from '@/lib/constants'
import { useAuth } from '@/lib/auth'
import { fmtDateTime } from '@/lib/utils'
import type { AdminUser } from '@/types'

const ROLE_OPTS = [
  { value: 'superadmin', label: '超级管理员' },
  { value: 'operator', label: '运营 operator' },
  { value: 'viewer', label: '访客 viewer（只读）' },
]

function NewUserModal({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient()
  const [username, setUsername] = useState('')
  const [name, setName] = useState('')
  const [password, setPassword] = useState('')
  const [role, setRole] = useState('operator')
  const create = useMutation({
    mutationFn: () => api.createUser({ username, password, display_name: name, role }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['users'] })
      onClose()
      toast('用户已创建', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  return (
    <Modal
      open
      onClose={onClose}
      width={440}
      icon={I.plus}
      title="新建后台用户"
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={!username || !name || password.length < 6 || create.isPending} onClick={() => create.mutate()}>
            创建
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="用户名" hint="唯一，创建后只读">
            <TextInput value={username} onChange={(v) => setUsername(v.replace(/[^a-z0-9_]/g, ''))} mono />
          </Field>
          <Field label="名称">
            <TextInput value={name} onChange={setName} placeholder="张管理" />
          </Field>
        </div>
        <Field label="初始密码" hint="至少 6 位；可登录后修改">
          <TextInput value={password} onChange={setPassword} type="password" />
        </Field>
        <Field label="角色">
          <Select value={role} onChange={setRole} options={ROLE_OPTS} />
        </Field>
      </div>
    </Modal>
  )
}

function EditUserModal({ u, self, onClose }: { u: AdminUser; self: boolean; onClose: () => void }) {
  const qc = useQueryClient()
  const [name, setName] = useState(u.display_name)
  const [role, setRole] = useState(u.role)
  const [status, setStatus] = useState(u.status)
  const save = useMutation({
    mutationFn: () => api.updateUser(u.id, { display_name: name, role, status }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['users'] })
      onClose()
      toast('用户已更新', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  return (
    <Modal
      open
      onClose={onClose}
      width={440}
      icon={I.edit}
      title={`编辑用户：${u.username}`}
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={save.isPending} onClick={() => save.mutate()}>
            保存
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="名称">
          <TextInput value={name} onChange={setName} />
        </Field>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="角色" hint={self ? '不能降级自己' : undefined}>
            <Select value={role} onChange={(v) => setRole(v as AdminUser['role'])} options={ROLE_OPTS} disabled={self} />
          </Field>
          <Field label="状态" hint={self ? '不能禁用自己' : undefined}>
            <Select value={status} onChange={setStatus} options={[{ value: 'active', label: '启用' }, { value: 'disabled', label: '禁用' }]} disabled={self} />
          </Field>
        </div>
      </div>
    </Modal>
  )
}

function ResetPasswordModal({ u, onClose }: { u: AdminUser; onClose: () => void }) {
  const [temp, setTemp] = useState<string | null>(null)
  const reset = useMutation({
    mutationFn: () => api.resetPassword(u.id),
    onSuccess: (r) => setTemp(r.temp_password),
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  return (
    <Modal
      open
      onClose={onClose}
      width={440}
      icon={I.key}
      title={`重置密码：${u.display_name}`}
      subtitle={`@${u.username}`}
      footer={
        temp ? (
          <>
            <span />
            <Btn variant="primary" onClick={onClose}>
              完成
            </Btn>
          </>
        ) : (
          <>
            <Btn onClick={onClose}>取消</Btn>
            <Btn variant="primary" disabled={reset.isPending} onClick={() => reset.mutate()}>
              生成临时密码
            </Btn>
          </>
        )
      }
    >
      {!temp ? (
        <div className="t2 sm" style={{ paddingBottom: 10, lineHeight: 1.6 }}>
          将为该用户生成一个临时密码（同时吊销其全部 refresh token，强制重新登录）。生成后请通过安全渠道转交。
        </div>
      ) : (
        <div className="col gap12" style={{ paddingBottom: 8 }}>
          <div className="row between" style={{ background: '#1f1f27', borderRadius: 9, padding: '13px 15px' }}>
            <span className="mono" style={{ color: '#9ef0c3', fontSize: 14, wordBreak: 'break-all' }}>
              {temp}
            </span>
            <Btn
              variant="default"
              size="sm"
              icon={I.copy}
              onClick={() => {
                navigator.clipboard?.writeText(temp)
                toast('已复制', 'success')
              }}
            >
              复制
            </Btn>
          </div>
          <div className="t3 xs">请通过安全渠道转交给用户；用户用此临时密码登录后应立即修改。</div>
        </div>
      )}
    </Modal>
  )
}

export function AdminUsers() {
  const qc = useQueryClient()
  const { user } = useAuth()
  const [confirm, setConfirm] = useState<AdminUser | null>(null)
  const [reset, setReset] = useState<AdminUser | null>(null)
  const [edit, setEdit] = useState<AdminUser | null>(null)
  const [newUser, setNewUser] = useState(false)

  const { data: users } = useQuery({ queryKey: ['users'], queryFn: api.listUsers })

  const toggle = useMutation({
    mutationFn: (u: AdminUser) => api.updateUser(u.id, { display_name: u.display_name, role: u.role, status: u.status === 'active' ? 'disabled' : 'active' }),
    onSuccess: (_r, u) => {
      qc.invalidateQueries({ queryKey: ['users'] })
      toast(u.status === 'active' ? '用户已禁用' : '已启用', 'success')
      setConfirm(null)
    },
    onError: (e) => {
      toast(getApiError(e).message, 'error')
      setConfirm(null)
    },
  })

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="后台用户"
          desc="管理后台登录账号（JWT 登录，与运行面 agent token 完全分离）。仅超级管理员可访问。"
          badge={
            <Badge tone="violet" sm>
              <I.lock size={11} />
              superadmin
            </Badge>
          }
          actions={
            <Btn variant="primary" icon={I.plus} onClick={() => setNewUser(true)}>
              新建用户
            </Btn>
          }
        />
        <Card noPad>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
            <thead>
              <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                <Th style={{ paddingLeft: 18 }}>用户名</Th>
                <Th>名称</Th>
                <Th>角色</Th>
                <Th>状态</Th>
                <Th>最近登录</Th>
                <Th right style={{ paddingRight: 18 }}>
                  操作
                </Th>
              </tr>
            </thead>
            <tbody>
              {(users || []).map((u) => {
                const self = u.id === user?.id
                return (
                  <tr key={u.id} className="hover-row" style={{ borderTop: '1px solid var(--border)' }}>
                    <Td style={{ paddingLeft: 18 }}>
                      <span className="row gap8">
                        <Avatar name={u.display_name || u.username} size={24} />
                        <span className="mono b">{u.username}</span>
                        {self && (
                          <Badge tone="gray" sm>
                            我
                          </Badge>
                        )}
                      </span>
                    </Td>
                    <Td>{u.display_name}</Td>
                    <Td>
                      <Badge tone={u.role === 'superadmin' ? 'violet' : u.role === 'operator' ? 'blue' : 'gray'} sm>
                        {ROLE_LABEL[u.role]}
                      </Badge>
                    </Td>
                    <Td>
                      <StatusBadge status={u.status === 'active' ? 'enabled' : 'disabled'} />
                    </Td>
                    <Td>
                      <span className="t3 mono sm">{fmtDateTime(u.last_login_at)}</span>
                    </Td>
                    <Td right style={{ paddingRight: 14 }}>
                      <Menu
                        trigger={<I.more size={18} />}
                        triggerClassName="hover-row"
                        triggerStyle={{ width: 30, height: 30, borderRadius: 7, justifyContent: 'center', color: 'var(--text-2)' }}
                        items={[
                          { icon: I.edit, label: '编辑', onClick: () => setEdit(u) },
                          { icon: I.key, label: '重置密码', onClick: () => setReset(u) },
                          { divider: true },
                          {
                            icon: u.status === 'active' ? I.lock : I.check,
                            label: u.status === 'active' ? '禁用' : '启用',
                            danger: u.status === 'active',
                            onClick: () => (self ? toast('不能禁用 / 降级自己（防自锁）', 'error') : u.status === 'active' ? setConfirm(u) : toggle.mutate(u)),
                          },
                        ]}
                      />
                    </Td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          <div className="t3 xs" style={{ padding: '10px 18px', borderTop: '1px solid var(--border)' }}>
            安全约束：不能禁用 / 降级自己（防自锁）；系统至少保留一个超级管理员；禁用即时失效其 refresh token。
          </div>
        </Card>

        <ConfirmDialog
          open={!!confirm}
          onClose={() => setConfirm(null)}
          danger
          confirmText="确认禁用"
          title="禁用后台用户？"
          message={
            confirm && (
              <>
                禁用{' '}
                <b>
                  {confirm.display_name}（{confirm.username}）
                </b>{' '}
                后，其 refresh token 立即失效，将无法登录后台。
              </>
            )
          }
          onConfirm={() => confirm && toggle.mutate(confirm)}
        />
        {reset && <ResetPasswordModal u={reset} onClose={() => setReset(null)} />}
        {edit && <EditUserModal u={edit} self={edit.id === user?.id} onClose={() => setEdit(null)} />}
        {newUser && <NewUserModal onClose={() => setNewUser(false)} />}
      </div>
    </div>
  )
}
