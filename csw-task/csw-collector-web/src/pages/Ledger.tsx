/**
 * 判断台账：**窗口内每条候选判了什么、凭什么判。**
 *
 * 三条刻意的设计：
 *
 * 1. **不推荐的也全列出来。** 只给推荐的，等于把「审阅覆盖 100%」变成一句
 *    无法验证的话——主编要能看见这三百多条都过了一遍。
 * 2. **没有分数栏，也不会有。** 排序在服务端定：按生效档、再按条目键。
 *    加一列分数，人就会开始按分数看，而那个分数根本不存在。
 * 3. **原判与生效档同时显示。** 「模型判不推荐、主编捞回成备选」与
 *    「模型判备选」是两件事，混成一个字段就再也分不出来。
 */
import { useMemo, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { Empty, ErrorBox, Head, Loading, Table, TierBadge, bj } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useAuth } from '@/lib/auth'
import { useJudgements, useOverride, useRounds, useSetFirstBatch } from '@/lib/queries'
import type { JudgementRow, Tier } from '@/lib/types'

const TIERS: { key: Tier | 'all'; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'recommend', label: '推荐' },
  { key: 'alternate', label: '备选' },
  { key: 'pending_check', label: '待核' },
  { key: 'not_recommend', label: '不推荐' },
]

const DIM_LABEL: Record<string, string> = {
  change: '变化',
  use: '使用',
  gain: '价值',
  compare: '比较',
  explain: '判断',
  csw: '调性',
}

export function Ledger() {
  const [sp, setSp] = useSearchParams()
  const rounds = useRounds({ limit: 20 })
  const roundId = Number(sp.get('round')) || rounds.data?.find((r) => r.kind === 'task')?.id
  const [tier, setTier] = useState<Tier | 'all'>('all')
  const [onlyGap, setOnlyGap] = useState(false)
  const [open, setOpen] = useState<string | null>(null)

  const js = useJudgements(roundId, {
    tier: tier === 'all' ? undefined : tier,
    has_gap: onlyGap || undefined,
  })
  const { me } = useAuth()
  const canEdit = me?.role === 'operator' || me?.role === 'superadmin'
  const override = useOverride(roundId)
  const firstBatch = useSetFirstBatch(roundId)

  const pinned = useMemo(
    () => (js.data ?? []).filter((j) => j.first_batch).map((j) => j.candidate_key),
    [js.data],
  )

  return (
    <section>
      <Head
        title="判断台账"
        desc="窗口内每条都在这里，包括不推荐的——只给推荐的，等于把「审阅覆盖 100%」变成一句无法验证的话。没有分数栏。"
        right={
          <select
            value={roundId ?? ''}
            onChange={(e) => setSp({ round: e.target.value })}
            className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
          >
            {(rounds.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                第 {r.id} 轮 · {bj(r.window_end)}
              </option>
            ))}
          </select>
        }
      />

      <div className="mb-3 flex flex-wrap items-center gap-2">
        {TIERS.map((t) => (
          <button
            key={t.key}
            onClick={() => setTier(t.key)}
            className={[
              'rounded-[var(--r-sm)] border px-2.5 py-1 text-[12.5px]',
              tier === t.key
                ? 'border-accent bg-accent-soft font-semibold text-accent'
                : 'border-rule bg-surface text-ink',
            ].join(' ')}
          >
            {t.label}
          </button>
        ))}
        <label className="ml-2 flex items-center gap-1.5 text-[12.5px] text-muted">
          <input type="checkbox" checked={onlyGap} onChange={(e) => setOnlyGap(e.target.checked)} />
          只看有缺口的
        </label>
        <span className="ml-auto text-[12.5px] text-muted">
          {js.data ? `${js.data.length} 条` : ''}
          {pinned.length > 0 && ` · 首批指定了 ${pinned.length} 条`}
        </span>
      </div>

      {js.isLoading && <Loading what="台账" />}
      {js.error && <ErrorBox error={js.error} />}
      {override.error && <ErrorBox error={override.error} />}
      {js.data?.length === 0 && <Empty>这一档下没有条目。</Empty>}

      {js.data && js.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>条目</Th>
              <Th>档</Th>
              <Th>六维</Th>
              <Th>热度</Th>
              <Th>缺口</Th>
              <Th right>操作</Th>
            </tr>
          }
        >
          {js.data.map((j) => (
            <Row
              key={j.candidate_key}
              j={j}
              roundId={roundId}
              open={open === j.candidate_key}
              onToggle={() => setOpen(open === j.candidate_key ? null : j.candidate_key)}
              canEdit={canEdit}
              onOverride={(to, reason) =>
                override.mutate({ key: j.candidate_key, to_tier: to, reason })
              }
              onPin={() =>
                firstBatch.mutate(
                  j.first_batch
                    ? pinned.filter((k) => k !== j.candidate_key)
                    : [...pinned, j.candidate_key],
                )
              }
            />
          ))}
        </Table>
      )}
    </section>
  )
}

function Row({
  j,
  roundId,
  open,
  onToggle,
  canEdit,
  onOverride,
  onPin,
}: {
  j: JudgementRow
  roundId?: number
  open: boolean
  onToggle: () => void
  canEdit: boolean
  onOverride: (to: Tier, reason: string) => void
  onPin: () => void
}) {
  const changed = j.effective_tier !== j.tier
  return (
    <>
      <tr className="border-t border-rule align-top">
        <Td>
          <button onClick={onToggle} className="border-none bg-transparent p-0 text-left">
            <div className="text-[13px] font-medium text-ink">{j.three_sentences.what_changed}</div>
            <div className="mt-0.5 text-[11.5px] text-muted">
              {j.account} · {j.candidate_key}
            </div>
          </button>
        </Td>
        <Td>
          <TierBadge tier={j.effective_tier} sm />
          {changed && (
            // 「模型判不推荐、主编捞回成备选」与「模型判备选」是两件事
            <div className="mt-1 text-[11px] text-muted">
              原判 {tierLabel(j.tier)} · {j.override_actor} 改的
            </div>
          )}
          {j.first_batch && <div className="mt-1 text-[11px] text-accent">已进首批</div>}
        </Td>
        <Td>
          <div className="flex gap-1">
            {j.dims.map(([dim, d]) => (
              <span
                key={dim}
                title={`${DIM_LABEL[dim] ?? dim}：${d.basis}`}
                className={[
                  'inline-block h-[18px] w-[18px] rounded-[3px] text-center text-[10px] leading-[18px]',
                  d.verdict === 'yes'
                    ? 'bg-ok-soft text-ok'
                    : d.verdict === 'no'
                      ? 'bg-surface-2 text-dim'
                      : 'border border-rule text-dim',
                ].join(' ')}
              >
                {(DIM_LABEL[dim] ?? dim).slice(0, 1)}
              </span>
            ))}
          </div>
        </Td>
        <Td className="text-[12px] text-muted">{j.heat_note || '——'}</Td>
        <Td>
          {j.gaps.length === 0 && j.check_flags.length === 0 ? (
            <span className="text-[12px] text-dim">——</span>
          ) : (
            <span className="text-[12px] text-warn">
              {j.gaps.length > 0 && `${j.gaps.length} 条缺口`}
              {j.check_flags.length > 0 && ` · ${j.check_flags.length} 条依据被标出`}
            </span>
          )}
          {!j.image_seen && <div className="text-[11.5px] text-warn">没读到实图</div>}
        </Td>
        <Td right>
          {canEdit ? (
            <div className="flex justify-end gap-1">
              <button
                onClick={onPin}
                className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-0.5 text-[12px]"
              >
                {j.first_batch ? '移出首批' : '加入首批'}
              </button>
              <ChangeTier current={j.effective_tier} onPick={onOverride} />
            </div>
          ) : (
            <span className="text-[12px] text-dim">只读</span>
          )}
        </Td>
      </tr>
      {open && <Detail j={j} roundId={roundId} />}
    </>
  )
}

/** 展开行：三句话、六维依据、对照结论、缺口。**依据是这一页的重点** */
function Detail({ j, roundId }: { j: JudgementRow; roundId?: number }) {
  return (
    <tr className="border-t border-rule bg-surface-2">
      <td colSpan={6} className="px-3 py-3">
        <div className="grid gap-4 md:grid-cols-2">
          <div>
            <div className="mb-1 text-[12px] font-semibold text-muted">三句话</div>
            <ol className="m-0 space-y-1 pl-4 text-[13px]">
              <li>{j.three_sentences.what_changed}</li>
              <li>{j.three_sentences.why_it_matters}</li>
              <li>{j.three_sentences.how_different}</li>
            </ol>
            <div className="mb-1 mt-3 text-[12px] font-semibold text-muted">对照结论</div>
            <div className="text-[13px]">
              {comparisonLabel(j.comparison.verdict)}
              {j.comparison.against && ` · 对照 ${j.comparison.against}`}
              {j.comparison.note && <div className="text-[12.5px] text-muted">{j.comparison.note}</div>}
            </div>
          </div>
          <div>
            <div className="mb-1 text-[12px] font-semibold text-muted">六维与依据</div>
            <div className="space-y-1">
              {j.dims.map(([dim, d]) => (
                <div key={dim} className="text-[12.5px]">
                  <span className="mr-1.5 text-muted">{DIM_LABEL[dim] ?? dim}</span>
                  <span
                    className={
                      d.verdict === 'yes' ? 'text-ok' : d.verdict === 'no' ? 'text-dim' : 'text-muted'
                    }
                  >
                    {verdictLabel(d.verdict)}
                  </span>
                  <span className="ml-1.5 text-ink">{d.basis}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
        {(j.gaps.length > 0 || j.check_flags.length > 0) && (
          <div className="mt-3">
            <div className="mb-1 text-[12px] font-semibold text-muted">缺口与被标出的依据</div>
            <ul className="m-0 space-y-0.5 pl-4 text-[12.5px] text-warn">
              {j.gaps.map((g, i) => (
                <li key={`g${i}`}>{g}</li>
              ))}
              {j.check_flags.map((f, i) => (
                <li key={`f${i}`}>{f}</li>
              ))}
            </ul>
          </div>
        )}
        {j.override_reason && (
          <div className="mt-3 text-[12.5px]">
            <span className="text-muted">改档理由（{j.override_actor}）：</span>
            {j.override_reason}
          </div>
        )}
        <div className="mt-3 flex gap-3 text-[12px]">
          <Link
            to={`/ledger/${roundId}/${encodeURIComponent(j.candidate_key)}`}
            className="text-accent no-underline"
          >
            看全貌（图、深核、改档历史）
          </Link>
          <a href={j.url} target="_blank" rel="noreferrer" className="text-accent no-underline">
            看原贴
          </a>
        </div>
      </td>
    </tr>
  )
}

/** 改档。**理由必填**——没有理由的改档在台账上与「模型本来就这么判的」分不出来 */
function ChangeTier({ current, onPick }: { current: Tier; onPick: (t: Tier, reason: string) => void }) {
  const [open, setOpen] = useState(false)
  const [to, setTo] = useState<Tier>('alternate')
  const [reason, setReason] = useState('')
  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-0.5 text-[12px]"
      >
        改档
      </button>
    )
  }
  return (
    <div className="flex items-center gap-1">
      <select
        value={to}
        onChange={(e) => setTo(e.target.value as Tier)}
        className="rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
      >
        {TIERS.filter((t) => t.key !== 'all' && t.key !== current).map((t) => (
          <option key={t.key} value={t.key}>
            {t.label}
          </option>
        ))}
      </select>
      <input
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        placeholder="理由（必填）"
        className="w-[140px] rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
      />
      <button
        disabled={!reason.trim()}
        onClick={() => {
          onPick(to, reason.trim())
          setOpen(false)
          setReason('')
        }}
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
  )
}

function tierLabel(t: string): string {
  return TIERS.find((x) => x.key === t)?.label ?? t
}

function verdictLabel(v: string): string {
  return { yes: '成立', no: '不成立', unclear: '不明' }[v] ?? v
}

function comparisonLabel(v: string): string {
  return (
    {
      same_fact_no_gain: '同一事实、没有新增',
      same_brand_with_gain: '同品牌但有新增',
      unrelated: '与已发的无关',
    }[v] ?? v
  )
}
