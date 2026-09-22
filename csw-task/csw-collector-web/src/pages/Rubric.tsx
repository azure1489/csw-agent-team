/**
 * 判断框架与锚点：**Van 的那把尺子长什么样。**
 *
 * 从代码里的常量来，不是从库里——它要跟着提示词一起走，
 * 改动经 `RUBRIC_VERSION` 进版本。这一页是给人核对「系统是不是按她的口径判的」，
 * 所以**一个字都不改写**。
 */
import { useState } from 'react'

import { Empty, ErrorBox, H2, Head, Loading, bjDate } from '@/components/Bits'
import { useAuth } from '@/lib/auth'
import { errText } from '@/lib/api'
import { useExclusions, useRubric, useSetExclusionActive } from '@/lib/queries'
import type { Exclusion } from '@/lib/types'

const DIM_CN: Record<string, string> = {
  change: '变化必须具体',
  use: '与真实使用有关',
  gain: '有 CSW 读者能理解的价值',
  compare: '最好有比较参照',
  explain: '能产生有事实支持的编辑判断',
  csw: 'CSW 视角（调性适配）',
}

export function Rubric() {
  const q = useRubric()
  if (q.isLoading) return <Loading what="判断框架" />
  if (q.error) return <ErrorBox error={q.error} />
  const r = q.data
  if (!r) return <Empty>没拿到框架。</Empty>

  return (
    <section>
      <Head title="判断框架与锚点" desc={`版本 ${r.version}。改动跟着提示词一起走。`} />

      <div className="rounded-[var(--r-md)] border border-accent-soft bg-accent-soft px-4 py-3">
        <div className="text-[12px] text-muted">核心问题</div>
        <div className="mt-1 text-[15px] font-semibold text-accent">{r.core_question}</div>
      </div>

      <H2>三问</H2>
      <ol className="m-0 space-y-1 pl-5 text-[13.5px]">
        {r.three_questions.map((x, i) => (
          <li key={i}>{x}</li>
        ))}
      </ol>

      <H2>六维与锚点</H2>
      <div className="grid gap-2 md:grid-cols-2">
        {r.dim_anchors.map((d) => (
          <div key={d.dim} className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
            <div className="text-[13px] font-semibold">{DIM_CN[d.dim] ?? d.dim}</div>
            <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted">{d.anchor}</div>
          </div>
        ))}
      </div>

      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <div>
          <H2>七类优先关注</H2>
          <ul className="m-0 space-y-0.5 pl-5 text-[13px]">
            {r.priority.map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </div>
        <div>
          <H2>七类降低优先级</H2>
          <ul className="m-0 space-y-0.5 pl-5 text-[13px]">
            {r.lower.map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </div>
      </div>
      <p className="mt-2 text-[12.5px] text-muted">
        这两类是<b>倾向，不是黑名单</b>：命中「降低优先级」的条目照样判、照样在台账里。
      </p>

      <H2>不是维度的四样</H2>
      <div className="flex flex-wrap gap-2">
        {r.not_dimensions.map((x) => (
          <span
            key={x}
            className="rounded-[var(--r-sm)] border border-rule bg-surface-2 px-2.5 py-1 text-[12.5px] text-muted line-through"
          >
            {x}
          </span>
        ))}
      </div>
      <p className="mt-2 text-[12.5px] text-muted">
        它们最容易被偷偷用上——点赞数高的看着就像值得写，但那不是 Van 的判据。
      </p>

      <Exclusions />
    </section>
  )
}

/**
 * 硬性排除规则。**这是唯一一类不经模型就让候选落定的东西**，
 * 所以这一页要把每条规则的出处、原话、当前状态都摆清楚。
 *
 * 停用过的、没原话的也都列出来——看不见的规则最危险。
 */
function Exclusions() {
  const q = useExclusions()
  const { me } = useAuth()
  const canEdit = me?.role === 'operator' || me?.role === 'superadmin'
  if (q.isLoading) return <Loading what="排除规则" />
  if (q.error) return <ErrorBox error={q.error} />
  const rows = q.data ?? []
  const on = rows.filter((r) => r.active)
  const off = rows.filter((r) => !r.active)

  return (
    <>
      <H2>硬性排除</H2>
      <p className="mb-2 text-[12.5px] text-muted">
        Van 在 03 否过的东西，<b>同一件事且没有新料</b>的不再送判断。
        这是唯一不经模型就让候选落定的规则——生效 {on.length} 条、未生效 {off.length} 条。
      </p>
      {rows.length === 0 ? (
        <Empty>
          还没有规则。它们来自引擎的 <code>/ledger/decisions</code>，随知识库同步一起进来。
        </Empty>
      ) : (
        <div className="space-y-2">
          {[...on, ...off].map((e) => (
            <Rule key={e.id} e={e} canEdit={canEdit} />
          ))}
        </div>
      )}
    </>
  )
}

function Rule({ e, canEdit }: { e: Exclusion; canEdit: boolean }) {
  const m = useSetExclusionActive()
  const [reason, setReason] = useState('')
  const [open, setOpen] = useState(false)
  const noQuote = !e.quote.trim()

  return (
    <div
      className={`rounded-[var(--r)] border px-3 py-2 ${
        e.active ? 'border-rule bg-surface' : 'border-rule bg-surface-2'
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={`rounded-[var(--r-sm)] px-1.5 py-0.5 text-[11.5px] ${
            e.active ? 'bg-accent-soft text-accent' : 'bg-surface text-muted'
          }`}
        >
          {e.active ? '生效中' : '未生效'}
        </span>
        <span className="text-[13.5px] font-semibold">{e.title || e.item_key}</span>
        {e.brand && <span className="text-[12.5px] text-muted">{e.brand}</span>}
        <span className="text-[12px] text-muted">
          {e.actor_role || '编辑部'} · {bjDate(e.decided_at)}
        </span>
      </div>

      {noQuote ? (
        <p className="mt-1 text-[12.5px] text-muted">
          <b>这次否决没有留下原话。</b>
          拿不出她的话就没法在台账上解释它为什么挡人，也判断不了否的是这件事还是这个角度——
          所以它进了表但不自动生效，要人看过再开。
        </p>
      ) : (
        <blockquote className="mt-1 border-l-2 border-accent-soft pl-2 text-[13px] leading-relaxed">
          {e.quote}
        </blockquote>
      )}
      {e.reason && <div className="mt-0.5 text-[12.5px] text-muted">理由：{e.reason}</div>}
      {!e.active && e.inactive_reason && !noQuote && (
        <div className="mt-1 text-[12.5px] text-muted">
          停用：{e.inactive_reason}
          {e.changed_by && `（${e.changed_by}）`}
        </div>
      )}

      {canEdit && (
        <div className="mt-1.5">
          {!open ? (
            <button
              onClick={() => setOpen(true)}
              className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-0.5 text-[12px]"
            >
              {e.active ? '停用' : '启用'}
            </button>
          ) : (
            <div className="flex flex-wrap items-center gap-1">
              {e.active && (
                <input
                  value={reason}
                  onChange={(ev) => setReason(ev.target.value)}
                  placeholder="停用理由（必填）"
                  className="w-[220px] rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
                />
              )}
              <button
                disabled={(e.active && !reason.trim()) || m.isPending}
                onClick={() =>
                  m.mutate(
                    { id: e.id, active: !e.active, reason: reason.trim() },
                    { onSuccess: () => setOpen(false) },
                  )
                }
                className="rounded-[var(--r-sm)] bg-accent px-2 py-0.5 text-[12px] text-white disabled:opacity-50"
              >
                存
              </button>
              <button
                onClick={() => setOpen(false)}
                className="rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
              >
                取消
              </button>
            </div>
          )}
          {m.error && <span className="ml-2 text-[12px] text-danger">{errText(m.error)}</span>}
        </div>
      )}
    </div>
  )
}
