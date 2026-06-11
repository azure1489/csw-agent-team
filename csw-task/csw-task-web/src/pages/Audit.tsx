import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { I } from '@/components/icons'
import { Avatar, Btn, Card, EmptyState, IconBtn, Modal, PageHeader, Select, Td, TextInput, Th, toast } from '@/components/ui'
import { ActionTag } from '@/components/common'
import { api } from '@/lib/api'
import { ACTION_META } from '@/lib/constants'
import { fmtDateTime } from '@/lib/utils'
import type { AuditEntry } from '@/types'

function prettyDetail(detail: string): string {
  if (!detail) return '（空）'
  try {
    return JSON.stringify(JSON.parse(detail), null, 2)
  } catch {
    return detail
  }
}

export function Audit() {
  const [fUser, setFUser] = useState('')
  const [fAction, setFAction] = useState('')
  const [fDate, setFDate] = useState('')
  const [detail, setDetail] = useState<AuditEntry | null>(null)

  const { data: audit } = useQuery({ queryKey: ['audit'], queryFn: () => api.listAudit() })
  const all = audit || []

  const users = useMemo(() => [...new Set(all.map((a) => a.username).filter(Boolean) as string[])], [all])
  const actions = useMemo(() => [...new Set(all.map((a) => a.action))], [all])

  const rows = all
    .filter((a) => !fUser || a.username === fUser)
    .filter((a) => !fAction || a.action === fAction)
    .filter((a) => !fDate || fmtDateTime(a.created_at).startsWith(fDate))

  const exportCsv = () => {
    const head = '时间,用户,动作,目标\n'
    const body = rows.map((a) => `${fmtDateTime(a.created_at)},${a.username || ''},${ACTION_META[a.action]?.label || a.action},${(a.target || '').replace(/,/g, ' ')}`).join('\n')
    const blob = new Blob([head + body], { type: 'text/csv;charset=utf-8' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = 'audit.csv'
    link.click()
    URL.revokeObjectURL(url)
    toast('已导出 audit.csv', 'success')
  }

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="审计日志"
          desc="后台操作留痕：登录、定义增改、激活、token 签发 / 吊销、用户管理。只读、可筛选、可导出。"
          actions={
            <Btn variant="default" icon={I.download} onClick={exportCsv}>
              导出
            </Btn>
          }
        />
        <Card noPad>
          <div className="row gap12" style={{ padding: '12px 14px', borderBottom: '1px solid var(--border)', flexWrap: 'wrap' }}>
            <div style={{ width: 130 }}>
              <Select value={fUser} onChange={setFUser} placeholder="全部用户" options={[{ value: '', label: '全部用户' }, ...users.map((u) => ({ value: u, label: u }))]} sm />
            </div>
            <div style={{ width: 160 }}>
              <Select
                value={fAction}
                onChange={setFAction}
                placeholder="全部动作"
                options={[{ value: '', label: '全部动作' }, ...actions.map((a) => ({ value: a, label: ACTION_META[a]?.label || a }))]}
                sm
              />
            </div>
            <div style={{ width: 150 }}>
              <TextInput value={fDate} onChange={setFDate} placeholder="2026-06-10" icon={I.clock} style={{ padding: '5px 10px 5px 32px', fontSize: 12.5 }} />
            </div>
            {(fUser || fAction || fDate) && (
              <Btn
                variant="ghost"
                size="sm"
                onClick={() => {
                  setFUser('')
                  setFAction('')
                  setFDate('')
                }}
              >
                清除
              </Btn>
            )}
            <span className="grow" />
            <span className="t3 sm">{rows.length} 条记录</span>
          </div>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
            <thead>
              <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                <Th style={{ paddingLeft: 18 }}>时间</Th>
                <Th>用户</Th>
                <Th>动作</Th>
                <Th>目标</Th>
                <Th right style={{ paddingRight: 18 }}>
                  详情
                </Th>
              </tr>
            </thead>
            <tbody>
              {rows.map((a) => (
                <tr key={a.id} className="hover-row" style={{ borderTop: '1px solid var(--border)' }}>
                  <Td style={{ paddingLeft: 18 }}>
                    <span className="t2 mono sm">{fmtDateTime(a.created_at)}</span>
                  </Td>
                  <Td>
                    <span className="row gap6">
                      <Avatar name={a.username || '?'} size={20} />
                      <span className="b sm">{a.username || '系统'}</span>
                    </span>
                  </Td>
                  <Td>
                    <ActionTag action={a.action} />
                  </Td>
                  <Td>
                    <span className="mono t2 sm">{a.target}</span>
                  </Td>
                  <Td right style={{ paddingRight: 14 }}>
                    <IconBtn icon={I.eye} title="查看详情" onClick={() => setDetail(a)} />
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && <EmptyState icon={I.audit} title="没有匹配的记录" desc="调整筛选条件" compact />}
        </Card>

        <Modal open={!!detail} onClose={() => setDetail(null)} width={460} icon={I.audit} title="审计详情" subtitle={detail ? `${fmtDateTime(detail.created_at)} · ${detail.username || '系统'}` : ''}>
          {detail && (
            <div className="col gap14" style={{ paddingBottom: 14 }}>
              <div className="row gap8">
                <ActionTag action={detail.action} />
                <span className="mono t2 sm">{detail.target}</span>
              </div>
              <div>
                <div className="t3 xs b" style={{ marginBottom: 6, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                  detail_json
                </div>
                <pre className="mono" style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 8, padding: 13, fontSize: 12, lineHeight: 1.6, margin: 0, overflow: 'auto' }}>
                  {prettyDetail(detail.detail)}
                </pre>
              </div>
            </div>
          )}
        </Modal>
      </div>
    </div>
  )
}
