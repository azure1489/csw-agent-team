import { useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { I } from '@/components/icons'
import { Avatar, Btn, SectionCard, StatusBadge, toast } from '@/components/ui'
import { api } from '@/lib/api'
import { EVENT_LABEL, EVENT_TONE, SCOPE_LABEL, SCOPE_SATISFIED_BY, STATUS, TONES, type Tone } from '@/lib/constants'
import { fmtTime } from '@/lib/utils'
import { useRoleName } from '@/hooks/useRoles'
import type { Deliverable, TaskBrief } from '@/types'

const STAGE_GLYPH: Record<string, { ch: string; tone: Tone }> = {
  passed: { ch: '✓', tone: 'green' },
  review: { ch: '▣', tone: 'amber' },
  dispatched: { ch: '▸', tone: 'blue' },
  ready: { ch: '▸', tone: 'blue' },
  in_progress: { ch: '▸', tone: 'blue' },
  returned: { ch: '↩', tone: 'red' },
  failed: { ch: '✕', tone: 'red' },
  cancelled: { ch: '–', tone: 'gray' },
  blocked: { ch: '○', tone: 'gray' },
}

function DownloadChip({ label, sub, dispatch, url }: { label: string; sub: string; dispatch?: boolean; url?: string }) {
  return (
    <button
      className="row gap8"
      onClick={() => (url ? window.open(url, '_blank') : toast('该交付物暂无下载链接', 'warn'))}
      style={{ border: '1px solid var(--border-strong)', background: 'var(--surface)', borderRadius: 8, padding: '7px 11px', boxShadow: 'var(--shadow-sm)' }}
      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-2)')}
      onMouseLeave={(e) => (e.currentTarget.style.background = 'var(--surface)')}
    >
      <span style={{ color: dispatch ? 'var(--amber)' : 'var(--accent-text)', display: 'flex' }}>{dispatch ? <I.doc size={16} /> : <I.download size={16} />}</span>
      <div className="col" style={{ gap: 0, alignItems: 'flex-start', lineHeight: 1.25 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600 }}>{label}</span>
        <span className="t3 mono" style={{ fontSize: 10.5 }}>
          {sub}
        </span>
      </div>
    </button>
  )
}

function StageBody({ taskId }: { taskId: number }) {
  const { data } = useQuery({ queryKey: ['task', taskId], queryFn: () => api.getTask(taskId) })
  if (!data) return <span className="t3 sm">加载中…</span>
  const dels = data.deliverables || []
  const rejects = dels.flatMap((d) => (d.reviews || []).filter((r) => r.verdict === 'reject').map((r) => ({ d, r })))
  if (dels.length === 0) return <span className="t3 sm">依赖未就绪或暂无产出，等待上游 / 派工。</span>
  return (
    <>
      <div className="row gap8 wrap">
        {dels.map((d: Deliverable) => (
          <DownloadChip
            key={d.id}
            label={d.is_dispatch ? '派工单' : `产出 v${d.version}`}
            sub={d.filename || d.doc_type || '—'}
            dispatch={d.is_dispatch}
            url={d.download_url}
          />
        ))}
      </div>
      {rejects.length > 0 && (
        <div className="col gap6">
          {rejects.map(({ d, r }, j) => (
            <div key={j} className="col gap3" style={{ background: 'var(--red-bg)', border: '1px solid var(--red-bd)', borderRadius: 7, padding: '8px 11px' }}>
              <div className="row gap6" style={{ color: 'var(--red)', fontSize: 12, fontWeight: 600 }}>
                <I.refresh size={13} />v{d.version} 被退回
              </div>
              {(r.return_direction || r.return_location) && <div className="t2 xs">方向：{r.return_direction || '—'} · 位置：{r.return_location || '—'}</div>}
              {r.comment && <div className="t2 xs">意见：{r.comment}</div>}
            </div>
          ))}
        </div>
      )}
    </>
  )
}

export function RunDetailPage() {
  const { id } = useParams()
  const runId = Number(id)
  const navigate = useNavigate()
  const roleName = useRoleName()
  const [order, setOrder] = useState<'desc' | 'asc'>('desc')
  const [expanded, setExpanded] = useState<number | null>(null)

  const { data: detail } = useQuery({ queryKey: ['run', runId], queryFn: () => api.getRun(runId), enabled: !!runId })
  const { data: events } = useQuery({ queryKey: ['timeline', runId], queryFn: () => api.getTimeline(runId), enabled: !!runId })
  const { data: agents } = useQuery({ queryKey: ['agents'], queryFn: api.listAgents })

  if (!detail) {
    return (
      <div className="row center" style={{ flex: 1, color: 'var(--text-3)', fontSize: 13.5 }}>
        加载中…
      </div>
    )
  }

  const run = detail.run
  const tasks = [...detail.tasks].sort((a, b) => a.seq - b.seq)
  const passedCount = tasks.filter((s) => s.status === 'passed').length
  const tl = order === 'desc' ? [...(events || [])].reverse() : events || []
  const agentName = (aid?: number | null) => (aid ? agents?.find((a) => a.id === aid)?.name || `#${aid}` : '系统')
  const auths = detail.authorizations || []
  const activeScopes = auths.filter((a) => a.status === 'active').map((a) => a.scope)
  const writeScope = (s: TaskBrief) => (s.action_class && s.action_class.startsWith('platform_write:') ? s.action_class.slice('platform_write:'.length) : '')
  const awaitingAuth = (s: TaskBrief) => {
    const sc = writeScope(s)
    return !!sc && s.status === 'ready' && !(SCOPE_SATISFIED_BY[sc] || [sc]).some((x) => activeScopes.includes(x))
  }

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1440, margin: '0 auto', padding: '26px 32px 60px' }}>
        <div className="col gap4" style={{ marginBottom: 18 }}>
          <button className="row gap6 t3 sm" style={{ border: 'none', background: 'transparent', padding: 0, width: 'fit-content', cursor: 'pointer' }} onClick={() => navigate('/runs')}>
            <I.chevLeft size={14} />
            运行监控
          </button>
          <div className="row between wrap" style={{ gap: 12 }}>
            <div className="row gap12" style={{ alignItems: 'center' }}>
              <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650 }}>
                <span className="mono">run #{run.id}</span>
              </h1>
              <span className="t3 mono sm">v{run.workflow_ver}</span>
              <span className="t3 mono sm">{run.subject}</span>
              {run.title && <span className="t2 sm">{run.title}</span>}
              <StatusBadge status={run.status} map={(s) => (s === 'active' ? 'run_active' : s)} />
            </div>
            <span className="row gap6 sm" style={{ color: 'var(--text-2)', background: 'var(--surface-2)', border: '1px solid var(--border)', padding: '4px 11px', borderRadius: 999 }}>
              <I.eye size={14} />
              全只读 · 无派工 / 审核 / 提交
            </span>
          </div>
        </div>

        <SectionCard title="本期授权" subtitle="平台写操作以此为准；只有本地演练时任何解锁都不进平台后台">
          {auths.length === 0 ? (
            <span className="t3 sm">未录入授权：平台写阶段会停在「等待授权」。</span>
          ) : (
            <div className="col gap6">
              {auths.map((a) => (
                <div key={a.id} className="row gap10 sm" style={{ opacity: a.status === 'active' ? 1 : 0.55 }}>
                  <span className="b" style={{ minWidth: 84 }}>
                    {SCOPE_LABEL[a.scope] || a.scope}
                  </span>
                  <span className="t2">{a.source_quote}</span>
                  <span className="t3 mono xs">{fmtTime(a.granted_at)}</span>
                  {a.status !== 'active' && <span className="t3 xs">{a.status === 'revoked' ? '已撤销' : '已过期'}</span>}
                </div>
              ))}
            </div>
          )}
        </SectionCard>

        <div style={{ height: 14 }} />

        <SectionCard title="进度" subtitle={`${passedCount} / ${tasks.length} 阶段已完成`}>
          <div className="row gap8 wrap">
            {tasks.map((s) => {
              const g = STAGE_GLYPH[s.status] || STAGE_GLYPH.blocked
              const t = TONES[g.tone]
              return (
                <div
                  key={s.id}
                  className="row gap6"
                  title={`${s.stage_name} · ${STATUS[s.status]?.label || s.status}`}
                  style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '5px 10px 5px 6px', borderRadius: 999, border: `1px solid ${t.bd}`, background: t.bg }}
                >
                  <span style={{ width: 19, height: 19, borderRadius: 999, background: t.c, color: '#fff', display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, fontWeight: 700 }}>
                    {s.seq}
                  </span>
                  <span style={{ fontSize: 12, fontWeight: 550, color: t.c }}>{g.ch}</span>
                </div>
              )
            })}
          </div>
        </SectionCard>

        <div style={{ display: 'grid', gridTemplateColumns: '1.5fr 1fr', gap: 14, marginTop: 14, alignItems: 'start' }}>
          {/* stage detail */}
          <SectionCard title="阶段明细" subtitle="点开看产出 / 派工单 / 审核记录" noPad>
            <div className="col">
              {tasks.map((s: TaskBrief, i) => {
                const open = expanded === s.id
                const g = STAGE_GLYPH[s.status] || STAGE_GLYPH.blocked
                return (
                  <div key={s.id} style={{ borderTop: i ? '1px solid var(--border)' : 'none' }}>
                    <div
                      className="row between"
                      onClick={() => setExpanded(open ? null : s.id)}
                      style={{ padding: '12px 16px', cursor: 'pointer' }}
                      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-2)')}
                      onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
                    >
                      <div className="row gap12">
                        <span style={{ width: 22, height: 22, borderRadius: 6, background: TONES[g.tone].bg, color: TONES[g.tone].c, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, fontWeight: 700 }}>
                          {s.seq}
                        </span>
                        <div className="col" style={{ gap: 2 }}>
                          <div className="row gap8">
                            <span className="b" style={{ fontSize: 13.5 }}>
                              {s.stage_name}
                            </span>
                            <span className="t3 sm">{roleName(s.role_code)}</span>
                            {s.is_merge && <span className="t3 xs">· 合流</span>}
                            {writeScope(s) && <span className="t3 xs">· 平台写 · {SCOPE_LABEL[writeScope(s)] || writeScope(s)}</span>}
                            {awaitingAuth(s) && <span style={{ color: 'var(--amber)', fontSize: 11.5, fontWeight: 600 }}>等待授权</span>}
                          </div>
                          <span className="t2 xs">{s.cur_version > 0 ? `当前 v${s.cur_version} · ${STATUS[s.status]?.label || s.status}` : STATUS[s.status]?.label || '未就绪'}</span>
                          {s.status === 'failed' && s.fail_reason && <span className="xs" style={{ color: 'var(--red)' }}>原因：{s.fail_reason}</span>}
                        </div>
                      </div>
                      <div className="row gap10">
                        <StatusBadge status={s.status} sm />
                        <span style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform .15s', color: 'var(--text-3)', display: 'flex' }}>
                          <I.chevRight size={15} />
                        </span>
                      </div>
                    </div>
                    {open && (
                      <div className="col gap10" style={{ padding: '4px 16px 16px 50px', animation: 'fadeIn .15s' }}>
                        <StageBody taskId={s.id} />
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          </SectionCard>

          {/* timeline */}
          <SectionCard
            title="时间线"
            subtitle="events 流水"
            noPad
            actions={
              <Btn variant="ghost" size="sm" onClick={() => setOrder((o) => (o === 'desc' ? 'asc' : 'desc'))}>
                {order === 'desc' ? '最新在前' : '最早在前'}
              </Btn>
            }
          >
            <div className="col" style={{ padding: '8px 0', maxHeight: 540, overflowY: 'auto' }}>
              {tl.length === 0 && <div className="t3 sm" style={{ padding: '12px 16px' }}>暂无事件</div>}
              {tl.map((e, i) => (
                <div key={e.id} className="row gap10" style={{ padding: '7px 16px', alignItems: 'flex-start' }}>
                  <span className="t3 mono xs" style={{ width: 38, flexShrink: 0, paddingTop: 2 }}>
                    {fmtTime(e.created_at)}
                  </span>
                  <div className="col" style={{ alignItems: 'center', flexShrink: 0, paddingTop: 3 }}>
                    <span style={{ width: 8, height: 8, borderRadius: 999, background: EVENT_TONE[e.type] || 'var(--gray)' }} />
                    {i < tl.length - 1 && <span style={{ width: 1.5, flex: 1, minHeight: 14, background: 'var(--border-2)', marginTop: 3 }} />}
                  </div>
                  <div className="col" style={{ gap: 1, paddingBottom: 4 }}>
                    <span style={{ fontSize: 12.5, lineHeight: 1.45 }}>{EVENT_LABEL[e.type] || e.type}</span>
                    <span className="t3 xs row gap5">
                      <Avatar name={agentName(e.actor_id)} size={14} />
                      {agentName(e.actor_id)}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </SectionCard>
        </div>
      </div>
    </div>
  )
}
