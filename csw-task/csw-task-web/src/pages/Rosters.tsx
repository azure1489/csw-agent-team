import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { I } from '@/components/icons'
import {
  Badge,
  Btn,
  Card,
  Checkbox,
  ConfirmDialog,
  Field,
  Modal,
  PageHeader,
  Select,
  TextInput,
  Td,
  Th,
  toast,
} from '@/components/ui'
import { api, getApiError } from '@/lib/api'
import { useRoles } from '@/hooks/useRoles'
import { useAuth } from '@/lib/auth'
import type { Chat, ChatMember, MemberBody } from '@/types'

// ── 群 新建 / 编辑 ──
function ChatModal({ chat, onClose }: { chat: Chat | 'new'; onClose: () => void }) {
  const qc = useQueryClient()
  const isNew = chat === 'new'
  const [chatKey, setChatKey] = useState(isNew ? '' : (chat as Chat).chat_key)
  const [name, setName] = useState(isNew ? '' : (chat as Chat).name)
  const [note, setNote] = useState(isNew ? '' : (chat as Chat).note)

  const save = useMutation({
    mutationFn: () =>
      isNew ? api.createChat({ chat_key: chatKey, name, note }) : api.updateChat((chat as Chat).id, { name, note }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chats'] })
      onClose()
      toast(isNew ? '群已创建' : '群已更新', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <Modal
      open
      onClose={onClose}
      width={460}
      icon={I.plus}
      title={isNew ? '新建群' : `编辑群：${name}`}
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={!chatKey || !name || save.isPending} onClick={() => save.mutate()}>
            {isNew ? '创建' : '保存'}
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <Field label="chat_id（飞书群 ID）" hint="创建后只读">
          <TextInput value={chatKey} onChange={setChatKey} mono placeholder="oc_..." readOnly={!isNew} />
        </Field>
        <Field label="群名称">
          <TextInput value={name} onChange={setName} placeholder="CSW编辑部" />
        </Field>
        <Field label="备注">
          <TextInput value={note} onChange={setNote} placeholder="可选" />
        </Field>
      </div>
    </Modal>
  )
}

// ── 成员 新建 / 编辑 ──
function MemberModal({ chatId, member, onClose }: { chatId: number; member: ChatMember | 'new'; onClose: () => void }) {
  const qc = useQueryClient()
  const { data: roles } = useRoles()
  const isNew = member === 'new'
  const m = member === 'new' ? null : (member as ChatMember)
  const [kind, setKind] = useState<'mapped' | 'reserved'>(m?.kind ?? 'mapped')
  const [roleCode, setRoleCode] = useState(m?.role_code ?? '')
  const [openID, setOpenID] = useState(m?.open_id ?? '')
  const [userID, setUserID] = useState(m?.user_id ?? '')
  const [displayName, setDisplayName] = useState(m?.display_name ?? '')
  const [botName, setBotName] = useState(m?.bot_name ?? '')
  const [isHuman, setIsHuman] = useState(m?.is_human ?? false)
  const [sort, setSort] = useState(String(m?.sort ?? 0))

  const body = (): MemberBody => ({
    kind,
    role_code: kind === 'mapped' ? roleCode : '',
    open_id: openID,
    user_id: userID || undefined,
    display_name: displayName,
    bot_name: botName,
    is_human: isHuman,
    sort: Number(sort) || 0,
  })

  const save = useMutation({
    mutationFn: () => (isNew ? api.createChatMember(chatId, body()) : api.updateChatMember(chatId, m!.id, body())),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chat-members', chatId] })
      qc.invalidateQueries({ queryKey: ['chats'] })
      onClose()
      toast(isNew ? '成员已新增' : '成员已更新', 'success')
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  const invalid = !openID || !displayName || !botName || (kind === 'mapped' && !roleCode)

  return (
    <Modal
      open
      onClose={onClose}
      width={480}
      icon={I.plus}
      title={isNew ? '新增成员' : `编辑成员：${botName}`}
      footer={
        <>
          <Btn onClick={onClose}>取消</Btn>
          <Btn variant="primary" disabled={invalid || save.isPending} onClick={() => save.mutate()}>
            {isNew ? '新增' : '保存'}
          </Btn>
        </>
      }
    >
      <div className="col gap16" style={{ paddingBottom: 8 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="类型">
            <Select
              value={kind}
              onChange={(v) => setKind(v as 'mapped' | 'reserved')}
              options={[
                { value: 'mapped', label: 'mapped（映射角色的 agent）' },
                { value: 'reserved', label: 'reserved（预留 bot）' },
              ]}
            />
          </Field>
          <Field label="角色 role_code" hint={kind === 'mapped' ? '必填' : 'reserved 不需要'}>
            <Select
              value={roleCode}
              onChange={setRoleCode}
              disabled={kind !== 'mapped'}
              options={[{ value: '', label: '—' }, ...(roles || []).map((r) => ({ value: r.code, label: `${r.name}（${r.code}）` }))]}
            />
          </Field>
        </div>
        <Field label="open_id" hint="飞书 open_id，群播报 @ 时用">
          <TextInput value={openID} onChange={setOpenID} mono placeholder="ou_..." />
        </Field>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="展示名" hint="引擎角色名，如「文案」">
            <TextInput value={displayName} onChange={setDisplayName} placeholder="文案" />
          </Field>
          <Field label="群内 bot 名" hint="飞书群显示名">
            <TextInput value={botName} onChange={setBotName} placeholder="深度内容创作者" />
          </Field>
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
          <Field label="user_id" hint="仅人类（Van）需要">
            <TextInput value={userID} onChange={setUserID} mono placeholder="可选" />
          </Field>
          <Field label="排序">
            <TextInput value={sort} onChange={(v) => setSort(v.replace(/[^0-9]/g, ''))} mono placeholder="0" />
          </Field>
        </div>
        <div style={{ border: '1px solid var(--border)', borderRadius: 8, padding: 12 }}>
          <Checkbox checked={isHuman} onChange={setIsHuman} label="人类成员（如 Van；无 token、群里 @ 用）" />
        </div>
      </div>
    </Modal>
  )
}

export function Rosters() {
  const qc = useQueryClient()
  const { user } = useAuth()
  const canWrite = user?.role !== 'viewer'
  const isSuper = user?.role === 'superadmin'

  const { data: chats } = useQuery({ queryKey: ['chats'], queryFn: api.listChats })
  const [chatId, setChatId] = useState<number | null>(null)
  useEffect(() => {
    if (chatId == null && chats && chats.length) setChatId(chats[0].id)
  }, [chats, chatId])
  const chat = chats?.find((c) => c.id === chatId) ?? null

  const { data: members } = useQuery({
    queryKey: ['chat-members', chatId],
    queryFn: () => api.listChatMembers(chatId as number),
    enabled: chatId != null,
  })

  const [chatModal, setChatModal] = useState<Chat | 'new' | null>(null)
  const [memberModal, setMemberModal] = useState<ChatMember | 'new' | null>(null)
  const [confirm, setConfirm] = useState<{ type: 'member'; m: ChatMember } | { type: 'chat'; c: Chat } | null>(null)

  const delMember = useMutation({
    mutationFn: (mm: ChatMember) => api.deleteChatMember(mm.chat_id, mm.id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chat-members', chatId] })
      qc.invalidateQueries({ queryKey: ['chats'] })
      toast('成员已删除', 'success')
      setConfirm(null)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })
  const delChat = useMutation({
    mutationFn: (c: Chat) => api.deleteChat(c.id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chats'] })
      setChatId(null)
      toast('群已删除', 'success')
      setConfirm(null)
    },
    onError: (e) => toast(getApiError(e).message, 'error'),
  })

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="通讯录"
          desc="飞书群信息与各角色 agent 的 open_id 映射。agent 群播报靠它把 role_code 解析成 @ 对象；后台改完，运行面 /roster 实时生效。"
          actions={
            canWrite && (
              <>
                <Btn icon={I.plus} onClick={() => setChatModal('new')}>
                  新建群
                </Btn>
                {chatId != null && (
                  <Btn variant="primary" icon={I.plus} onClick={() => setMemberModal('new')}>
                    新增成员
                  </Btn>
                )}
              </>
            )
          }
        />

        <Card style={{ padding: 16 }}>
          <div className="row between" style={{ alignItems: 'flex-end', gap: 16 }}>
            <div className="row gap16" style={{ alignItems: 'flex-end', flexWrap: 'wrap' }}>
              <Field label="群" style={{ minWidth: 240 }}>
                <Select
                  value={String(chatId ?? '')}
                  onChange={(v) => setChatId(Number(v))}
                  options={(chats || []).map((c) => ({ value: String(c.id), label: `${c.name}（${c.member_count} 人）` }))}
                />
              </Field>
              {chat && (
                <div className="col" style={{ gap: 4 }}>
                  <span className="mono t3" style={{ fontSize: 12 }}>
                    {chat.chat_key}
                  </span>
                  {chat.note && <span className="t3 xs">{chat.note}</span>}
                </div>
              )}
            </div>
            {chat && canWrite && (
              <div className="row gap8">
                <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => setChatModal(chat)}>
                  编辑群
                </Btn>
                {isSuper && (
                  <Btn variant="ghost" size="sm" icon={I.x} style={{ color: 'var(--red)' }} onClick={() => setConfirm({ type: 'chat', c: chat })}>
                    删除群
                  </Btn>
                )}
              </div>
            )}
          </div>
        </Card>

        <div style={{ marginTop: 16 }}>
          <Card noPad>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
              <thead>
                <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                  <Th style={{ paddingLeft: 18 }}>群内 bot 名</Th>
                  <Th>展示名</Th>
                  <Th>角色 / 类型</Th>
                  <Th>open_id</Th>
                  <Th center>人工</Th>
                  <Th right style={{ paddingRight: 18 }}>
                    操作
                  </Th>
                </tr>
              </thead>
              <tbody>
                {(members || []).map((mm) => (
                  <tr key={mm.id} className="hover-row" style={{ borderTop: '1px solid var(--border)' }}>
                    <Td style={{ paddingLeft: 18 }}>
                      <span className="b">{mm.bot_name}</span>
                    </Td>
                    <Td>{mm.display_name}</Td>
                    <Td>
                      {mm.kind === 'mapped' ? (
                        <span className="mono t2" style={{ fontSize: 12.5 }}>
                          {mm.role_code}
                        </span>
                      ) : (
                        <Badge tone="gray" sm>
                          预留
                        </Badge>
                      )}
                    </Td>
                    <Td>
                      <span className="mono t3" style={{ fontSize: 12 }}>
                        {mm.open_id}
                      </span>
                    </Td>
                    <Td center>
                      {mm.is_human ? (
                        <Badge tone="amber" sm>
                          人工
                        </Badge>
                      ) : (
                        <span className="t3">—</span>
                      )}
                    </Td>
                    <Td right style={{ paddingRight: 14 }}>
                      {canWrite && (
                        <div className="row gap4" style={{ justifyContent: 'flex-end' }}>
                          <Btn variant="ghost" size="sm" icon={I.edit} onClick={() => setMemberModal(mm)}>
                            编辑
                          </Btn>
                          <Btn variant="ghost" size="sm" icon={I.x} style={{ color: 'var(--red)' }} onClick={() => setConfirm({ type: 'member', m: mm })}>
                            删除
                          </Btn>
                        </div>
                      )}
                    </Td>
                  </tr>
                ))}
                {members && members.length === 0 && (
                  <tr>
                    <td colSpan={6} style={{ padding: '18px', color: 'var(--text-3)' }}>
                      该群暂无成员，点「新增成员」添加。
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
            <div className="t3 xs" style={{ padding: '10px 18px', borderTop: '1px solid var(--border)' }}>
              mapped = 映射到引擎 role_code 的 agent（群播报按 role_code 查这里的 open_id）；reserved = 群里有但未接入工作流的 bot；人工 = Van 等真人（无 token）。
            </div>
          </Card>
        </div>
      </div>

      {chatModal && <ChatModal chat={chatModal} onClose={() => setChatModal(null)} />}
      {memberModal && chatId != null && <MemberModal chatId={chatId} member={memberModal} onClose={() => setMemberModal(null)} />}

      <ConfirmDialog
        open={!!confirm}
        onClose={() => setConfirm(null)}
        danger
        confirmText="删除"
        title={confirm?.type === 'chat' ? '删除该群？' : '删除该成员？'}
        message={
          confirm?.type === 'chat'
            ? `「${confirm.c.name}」及其全部成员将一并删除，不可恢复。`
            : confirm?.type === 'member'
              ? `「${confirm.m.bot_name}」从通讯录移除后，涉及该角色的群播报将 @ 不到人。`
              : ''
        }
        onConfirm={() => {
          if (confirm?.type === 'member') delMember.mutate(confirm.m)
          else if (confirm?.type === 'chat') delChat.mutate(confirm.c)
        }}
      />
    </div>
  )
}
