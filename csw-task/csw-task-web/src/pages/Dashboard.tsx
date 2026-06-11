import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { I, type IconComponent } from '@/components/icons'
import { Avatar, Btn, Card, EmptyState, PageHeader, SectionCard, StatusBadge, toast } from '@/components/ui'
import { ActionTag, ProgressPill } from '@/components/common'
import { TONES, type Tone } from '@/lib/constants'
import { api } from '@/lib/api'
import { fmtDate, fmtDateTime } from '@/lib/utils'

function StatCard({
  label,
  value,
  icon: IconC,
  tone = 'violet',
  sub,
  onClick,
}: {
  label: string
  value: number | string
  icon: IconComponent
  tone?: Tone
  sub?: string
  onClick?: () => void
}) {
  const t = TONES[tone]
  return (
    <Card hover={!!onClick} onClick={onClick} style={{ padding: '16px 18px' }}>
      <div className="row between" style={{ alignItems: 'flex-start' }}>
        <div className="col" style={{ gap: 6 }}>
          <span className="t2 sm" style={{ fontWeight: 550 }}>
            {label}
          </span>
          <span style={{ fontSize: 27, fontWeight: 680, letterSpacing: '-.02em', lineHeight: 1 }}>{value}</span>
          {sub && <span className="t3 xs">{sub}</span>}
        </div>
        <span style={{ width: 34, height: 34, borderRadius: 9, background: t.bg, color: t.c, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <IconC size={18} />
        </span>
      </div>
    </Card>
  )
}

export function Dashboard() {
  const navigate = useNavigate()
  const { data, refetch } = useQuery({ queryKey: ['overview'], queryFn: api.overview })
  const c = data?.counts
  const runs = data?.recent_runs || []
  const audit = data?.recent_audit || []
  const changes = audit.filter((a) => a.action.startsWith('workflow_')).slice(0, 5)
  const today = fmtDate(new Date().toISOString())

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="仪表盘"
          desc="一眼看清后台与运行健康度。卡片可点击跳转对应列表 / 详情。"
          actions={
            <Btn
              variant="default"
              icon={I.refresh}
              onClick={() => {
                refetch()
                toast('已刷新概览数据', 'success')
              }}
            >
              刷新
            </Btn>
          }
        />

        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 14, marginBottom: 18 }}>
          <StatCard label="已激活工作流" value={c?.active_workflows ?? 0} icon={I.workflow} tone="green" sub="active 状态" onClick={() => navigate('/workflows')} />
          <StatCard label="在跑实例" value={c?.active_runs ?? 0} icon={I.runs} tone="blue" sub="实时运行中" onClick={() => navigate('/runs')} />
          <StatCard label="今日完成" value={c?.done_today ?? 0} icon={I.checkCircle} tone="violet" sub={today} />
          <StatCard
            label="待处理告警"
            value={c?.pending_alerts ?? 0}
            icon={I.warn}
            tone={c?.pending_alerts ? 'amber' : 'gray'}
            sub={c?.pending_alerts ? '需关注' : '一切正常'}
            onClick={() => navigate('/members')}
          />
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1.6fr 1fr', gap: 14, marginBottom: 14 }}>
          {/* running instances */}
          <SectionCard
            title="在跑实例"
            subtitle="点击进入运行详情（只读）"
            noPad
            actions={
              <Btn variant="ghost" size="sm" iconRight={I.chevRight} onClick={() => navigate('/runs')}>
                全部
              </Btn>
            }
          >
            {runs.length === 0 ? (
              <EmptyState icon={I.runs} title="暂无运行实例" desc="工作流被触发后，实例会出现在这里" compact />
            ) : (
              <div className="col">
                {runs.map((r, i) => (
                  <div
                    key={r.id}
                    className="row between"
                    onClick={() => navigate(`/runs/${r.id}`)}
                    style={{ padding: '14px 16px', borderTop: i ? '1px solid var(--border)' : 'none', cursor: 'pointer' }}
                    onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-2)')}
                    onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
                  >
                    <div className="row gap12">
                      <span className="mono t3" style={{ fontSize: 12.5, width: 34 }}>
                        #{r.id}
                      </span>
                      <div className="col" style={{ gap: 3 }}>
                        <div className="row gap8">
                          <span className="b" style={{ fontSize: 13.5 }}>
                            {r.workflow_name}
                          </span>
                          <span className="t3 mono sm">{r.subject}</span>
                        </div>
                        <div className="row gap8 t2 sm">
                          <span>当前：{r.current_stage || '—'}</span>
                        </div>
                      </div>
                    </div>
                    <div className="row gap12">
                      {r.current_stage && <ProgressPill text={r.current_stage} />}
                      <StatusBadge status={r.status} map={(s) => (s === 'active' ? 'run_active' : s)} />
                      <I.chevRight size={16} style={{ color: 'var(--text-3)' }} />
                    </div>
                  </div>
                ))}
              </div>
            )}
          </SectionCard>

          {/* recent workflow changes (派生自审计) */}
          <SectionCard title="最近激活 / 改动" subtitle="工作流定义变更" noPad>
            {changes.length === 0 ? (
              <EmptyState icon={I.workflow} title="暂无改动" compact />
            ) : (
              <div className="col">
                {changes.map((a, i) => (
                  <div key={a.id} className="row between" style={{ padding: '12px 16px', borderTop: i ? '1px solid var(--border)' : 'none' }}>
                    <div className="row gap10">
                      <span style={{ width: 30, height: 30, borderRadius: 7, background: 'var(--surface-2)', border: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--text-2)' }}>
                        <I.workflow size={15} />
                      </span>
                      <div className="col" style={{ gap: 3 }}>
                        <ActionTag action={a.action} />
                        <span className="t2 xs mono">{a.target || '—'}</span>
                      </div>
                    </div>
                    <div className="col" style={{ alignItems: 'flex-end', gap: 4 }}>
                      <span className="t2 xs">{a.username || '系统'}</span>
                      <span className="t3 xs mono">{fmtDateTime(a.created_at).slice(5)}</span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </SectionCard>
        </div>

        {/* recent audit */}
        <SectionCard
          title="最近审计"
          subtitle="后台操作留痕"
          noPad
          actions={
            <Btn variant="ghost" size="sm" iconRight={I.chevRight} onClick={() => navigate('/audit')}>
              全部
            </Btn>
          }
        >
          {audit.length === 0 ? (
            <EmptyState icon={I.audit} title="暂无审计记录" compact />
          ) : (
            <div className="col">
              {audit.map((a, i) => (
                <div key={a.id} className="row gap12" style={{ padding: '11px 16px', borderTop: i ? '1px solid var(--border)' : 'none' }}>
                  <span className="t3 mono sm" style={{ width: 110, flexShrink: 0 }}>
                    {fmtDateTime(a.created_at).slice(5)}
                  </span>
                  <Avatar name={a.username || '?'} size={22} />
                  <span className="b sm" style={{ width: 70 }}>
                    {a.username || '系统'}
                  </span>
                  <ActionTag action={a.action} />
                  <span className="t2 sm grow mono" style={{ textAlign: 'right' }}>
                    {a.target}
                  </span>
                </div>
              ))}
            </div>
          )}
        </SectionCard>
      </div>
    </div>
  )
}
