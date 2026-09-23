/**
 * 今日总览：**本期跑到哪一步了、有没有卡住、哪些要人看。**
 *
 * 设计上只回答这三个问题。想看每条判了什么去台账，想看每个采集器取了多少去覆盖页——
 * 总览再塞一张大表，人就会在这一页里找东西，而这一页的价值恰恰是「一眼」。
 */
import { Link } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Stat, StepBadge, Table, bj } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useHealth, usePendingCheck, useRound, useRounds, useSteps, useWork } from '@/lib/queries'

/** 十步。名字与 `core::types::StepCode` 一一对应，顺序就是流程顺序。 */
const STEPS: [string, string][] = [
  ['intake', '接单'],
  ['harvest', '采集'],
  ['merge', '合并'],
  ['materials', '对照'],
  ['judge', '判断'],
  ['deepcheck', '深核'],
  ['build', '交付物'],
  ['register', '登记'],
  ['self_check', '自查'],
  ['submit', '提交'],
]

export function Overview() {
  const rounds = useRounds({ limit: 10 })
  const latest = rounds.data?.find((r) => r.kind === 'task') ?? rounds.data?.[0]
  const detail = useRound(latest?.id)
  const steps = useSteps(latest?.id)
  const health = useHealth()
  const work = useWork()
  const pending = usePendingCheck()

  if (rounds.isLoading) return <Loading what="轮次" />
  if (rounds.error) return <ErrorBox error={rounds.error} />

  // 按有效档（叠加人工改档）算：主编改过的档，总览上也该是改过的
  const tiers = Object.fromEntries(detail.data?.effective_tiers ?? detail.data?.tiers ?? [])
  // 改版前的轮次没有选题记录：那时退回按贴文数
  const tc = (detail.data?.topics?.topics ?? 0) > 0 ? detail.data?.topics : undefined
  // 等主编或 Van 处理的待核亮出来，不用等 Van 来催（09-22 反馈第八项）
  const waiting = (pending.data ?? []).filter((r) =>
    r.gaps.some((g) => g.owner === 'editor' || g.owner === 'van'),
  ).length
  const bad = health.data?.deps.filter((d) => !d.ok) ?? []

  return (
    <section>
      <Head
        title="今日总览"
        desc={
          latest
            ? `第 ${latest.id} 轮 · ${latest.kind === 'task' ? `r${latest.run_id} 任务#${latest.task_id}` : latest.kind} · 窗口 ${bj(latest.window_start)} → ${bj(latest.window_end)}`
            : '还没有开过轮次'
        }
        right={latest ? <StepBadge status={latest.status} /> : null}
      />

      {waiting > 0 && (
        <div className="mb-3 rounded-[var(--r-md)] border border-warn-soft bg-warn-soft px-3.5 py-2.5 text-[13px] text-warn">
          有 {waiting} 条待核在等主编或 Van 处理（缺口里写了下一步）。
          <Link to="/pending" className="ml-1 text-warn underline">
            去待核结转
          </Link>
        </div>
      )}

      {(bad.length > 0 || (health.data?.outbox_conflicts ?? 0) > 0) && (
        <div className="mb-4 rounded-[var(--r-md)] border border-warn-soft bg-warn-soft px-3.5 py-2.5 text-[13px] text-warn">
          {bad.map((d) => (
            <div key={d.name}>
              {d.name}：{d.note}
            </div>
          ))}
          {(health.data?.outbox_conflicts ?? 0) > 0 && (
            <div>
              有 {health.data?.outbox_conflicts} 条写引擎的记录卡在冲突上。
              <b>先查任务状态对账，不要换幂等键重试。</b>
            </div>
          )}
        </div>
      )}

      {!latest ? (
        <Empty>还没有开过轮次。05:30 接单后这里会有东西，也可以去「运行与设置」手动开一轮。</Empty>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Stat
              label="窗口内候选"
              value={detail.data?.candidates ?? '—'}
              note={detail.data ? `其中结转 ${detail.data.carried}` : undefined}
            />
            <Stat
              label="没判的"
              value={detail.data?.unjudged ?? '—'}
              note="每条都判，这里应当是 0"
              tone={(detail.data?.unjudged ?? 0) > 0 ? 'bad' : undefined}
            />
            <Stat
              label="推荐选题"
              value={tc?.recommend_topics ?? tiers.recommend ?? 0}
              note={
                tc
                  ? `${tc.recommend_posts} 帖 · 备选 ${tc.alternate_topics} 题 / ${tc.alternate_posts} 帖`
                  : `备选 ${tiers.alternate ?? 0}`
              }
            />
            <Stat
              label="待核"
              value={tiers.pending_check ?? 0}
              note="待核不是淘汰，补齐后重评"
              tone={(tiers.pending_check ?? 0) > 0 ? 'warn' : undefined}
            />
          </div>

          <H2>十步</H2>
          <div className="flex flex-wrap gap-2">
            {STEPS.map(([code, label]) => {
              // 只看最后一次 attempt：前几次是历史，要看去轮次详情。
              // 合并与判断在同一个流水线里做，不单独成步——跟着判断那一格走
              const src = code === 'merge' ? 'judge' : code
              const rows = (steps.data ?? []).filter((s) => s.step === src)
              const last = rows[rows.length - 1]
              const done = Number(last?.counts?.['进度'])
              const total = Number(last?.counts?.['共'])
              return (
                <div
                  key={code}
                  className="min-w-[104px] flex-1 rounded-[var(--r)] border border-rule bg-surface px-2.5 py-2"
                >
                  <div className="text-[12px] text-muted">{label}</div>
                  <div className="mt-1">
                    {last ? <StepBadge status={last.status} sm /> : <span className="text-[12px] text-dim">未开始</span>}
                  </div>
                  {last?.status === 'running' && total > 0 && (
                    <div className="mt-1.5">
                      <div className="h-1 overflow-hidden rounded-full bg-surface-2">
                        <div
                          className="h-full rounded-full bg-accent transition-[width] duration-500"
                          style={{ width: `${Math.min(100, (done / total) * 100)}%` }}
                        />
                      </div>
                      <div className="mt-1 text-[11px] text-dim tnum">
                        {done}/{total}
                      </div>
                    </div>
                  )}
                  {code === 'merge' && last && (
                    <div className="mt-1 text-[11px] text-dim">随判断</div>
                  )}
                  {last && last.attempt > 1 && (
                    <div className="mt-1 text-[11px] text-dim">第 {last.attempt} 次</div>
                  )}
                </div>
              )
            })}
          </div>

          {(steps.data ?? []).some((s) => s.error) && (
            <>
              <H2>没跑成的</H2>
              <Table
                head={
                  <tr>
                    <Th>步</Th>
                    <Th>第几次</Th>
                    <Th>原因</Th>
                  </tr>
                }
              >
                {(steps.data ?? [])
                  .filter((s) => s.error)
                  .map((s, i) => (
                    <tr key={i} className="border-t border-rule">
                      <Td>{STEPS.find((x) => x[0] === s.step)?.[1] ?? s.step}</Td>
                      <Td>{s.attempt}</Td>
                      <Td style={{ color: 'var(--bad)' }}>{s.error}</Td>
                    </tr>
                  ))}
              </Table>
            </>
          )}
        </>
      )}

      <H2>最近的轮次</H2>
      <Table
        head={
          <tr>
            <Th>轮次</Th>
            <Th>类型</Th>
            <Th>窗口</Th>
            <Th>状态</Th>
            <Th>开的时间</Th>
          </tr>
        }
      >
        {(rounds.data ?? []).map((r) => (
          <tr key={r.id} className="border-t border-rule">
            <Td>
              <Link to={`/ledger?round=${r.id}`} className="text-accent no-underline">
                第 {r.id} 轮
              </Link>
            </Td>
            <Td>{kindLabel(r.kind)}</Td>
            <Td className="tnum">
              {bj(r.window_start)} → {bj(r.window_end)}
            </Td>
            <Td>
              <StepBadge status={r.status} sm />
              {r.note && <span className="ml-2 text-[12px] text-muted">{r.note}</span>}
            </Td>
            <Td className="tnum">{bj(r.created_at)}</Td>
          </tr>
        ))}
      </Table>

      {(work.data ?? []).length > 0 && (
        <>
          <H2>排队的活</H2>
          <Table
            head={
              <tr>
                <Th>活</Th>
                <Th>谁排的</Th>
                <Th>状态</Th>
                <Th>结果</Th>
              </tr>
            }
          >
            {(work.data ?? []).map((w) => (
              <tr key={w.id} className="border-t border-rule">
                <Td>{w.kind === 'manual_round' ? '手动开轮' : `重跑第 ${w.round_id} 轮`}</Td>
                <Td>{w.actor}</Td>
                <Td>
                  <StepBadge status={w.status === 'done' ? 'succeeded' : w.status} sm />
                </Td>
                <Td style={{ color: w.status === 'failed' ? 'var(--bad)' : undefined }}>
                  {w.note || '——'}
                </Td>
              </tr>
            ))}
          </Table>
        </>
      )}
    </section>
  )
}

function kindLabel(k: string): string {
  return { task: '派单轮', manual: '手动轮', prefetch: '预取轮', replay: '回放' }[k] ?? k
}
