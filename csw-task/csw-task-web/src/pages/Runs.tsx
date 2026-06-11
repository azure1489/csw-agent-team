import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { I } from '@/components/icons'
import { Avatar, Btn, Card, EmptyState, PageHeader, Select, StatusBadge, Td, TextInput, Th, toast } from '@/components/ui'
import { api } from '@/lib/api'
import { fmtDateTime } from '@/lib/utils'

export function Runs() {
  const navigate = useNavigate()
  const [wf, setWf] = useState('')
  const [status, setStatus] = useState('')
  const [date, setDate] = useState('')

  const { data: workflows } = useQuery({ queryKey: ['workflows'], queryFn: () => api.listWorkflows() })
  const { data: runs, refetch } = useQuery({
    queryKey: ['runs', wf, status, date],
    queryFn: () => api.listRuns({ workflow: wf || undefined, status: status || undefined, date: date || undefined }),
  })

  const wfOptions = useMemo(() => {
    const seen = new Map<string, string>()
    ;(workflows || []).forEach((w) => seen.set(w.wf_key, w.name))
    return [{ value: '', label: '全部工作流' }, ...[...seen].map(([k, n]) => ({ value: k, label: n }))]
  }, [workflows])

  const rows = runs || []

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="运行监控"
          desc="查看所有运行实例及状态。后台不操作流程——派工 / 审核 / 提交均由 agent 经 csw-task skill 完成，此处全只读。"
          actions={
            <Btn
              variant="default"
              icon={I.refresh}
              onClick={() => {
                refetch()
                toast('已刷新运行列表', 'success')
              }}
            >
              刷新
            </Btn>
          }
        />
        <Card noPad>
          <div className="row gap12" style={{ padding: '12px 14px', borderBottom: '1px solid var(--border)', flexWrap: 'wrap' }}>
            <div style={{ width: 160 }}>
              <Select value={wf} onChange={setWf} placeholder="全部工作流" options={wfOptions} sm />
            </div>
            <div style={{ width: 130 }}>
              <Select
                value={status}
                onChange={setStatus}
                placeholder="全部状态"
                options={[
                  { value: '', label: '全部状态' },
                  { value: 'active', label: '进行中' },
                  { value: 'done', label: '已完成' },
                  { value: 'paused', label: '已暂停' },
                  { value: 'aborted', label: '已中止' },
                ]}
                sm
              />
            </div>
            <div style={{ width: 150 }}>
              <TextInput value={date} onChange={setDate} placeholder="2026-06-10" icon={I.clock} style={{ padding: '5px 10px 5px 32px', fontSize: 12.5 }} />
            </div>
            {(wf || status || date) && (
              <Btn
                variant="ghost"
                size="sm"
                onClick={() => {
                  setWf('')
                  setStatus('')
                  setDate('')
                }}
              >
                清除筛选
              </Btn>
            )}
          </div>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13.5 }}>
            <thead>
              <tr style={{ color: 'var(--text-3)', fontSize: 11.5, textTransform: 'uppercase', letterSpacing: '.03em' }}>
                <Th style={{ paddingLeft: 18 }}>run</Th>
                <Th>工作流</Th>
                <Th>subject</Th>
                <Th>状态</Th>
                <Th>当前阶段</Th>
                <Th>触发者</Th>
                <Th>创建时间</Th>
                <Th right style={{ paddingRight: 18 }} />
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.id} className="hover-row" style={{ borderTop: '1px solid var(--border)', cursor: 'pointer' }} onClick={() => navigate(`/runs/${r.id}`)}>
                  <Td style={{ paddingLeft: 18 }}>
                    <span className="mono b">#{r.id}</span>
                  </Td>
                  <Td>
                    <span className="b">{r.workflow_name}</span>
                  </Td>
                  <Td>
                    <span className="mono t2 sm">{r.subject}</span>
                  </Td>
                  <Td>
                    <StatusBadge status={r.status} map={(s) => (s === 'active' ? 'run_active' : s)} />
                  </Td>
                  <Td>
                    <span className="t2">{r.current_stage || '—'}</span>
                  </Td>
                  <Td>
                    <span className="row gap6">
                      <Avatar name={r.trigger_name || '?'} size={20} />
                      <span className="sm">{r.trigger_name || '—'}</span>
                    </span>
                  </Td>
                  <Td>
                    <span className="t3 mono sm">{fmtDateTime(r.created_at)}</span>
                  </Td>
                  <Td right style={{ paddingRight: 14 }}>
                    <Btn variant="ghost" size="sm" iconRight={I.chevRight}>
                      查看
                    </Btn>
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && <EmptyState icon={I.runs} title="没有匹配的运行实例" desc="调整筛选条件后再试" compact />}
        </Card>
      </div>
    </div>
  )
}
