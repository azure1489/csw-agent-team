/**
 * 判断台账：**窗口内每条候选判了什么、凭什么判。**
 *
 * 几条刻意的设计：
 *
 * 1. **不推荐的也全列出来。** 只给推荐的，等于把「审阅覆盖 100%」变成一句
 *    无法验证的话——主编要能看见这三百多条都过了一遍。
 * 2. **没有分数栏，也不会有。** 排序在服务端定：按生效档、再按条目键。
 * 3. **原判与生效档同时显示。** 「模型判不推荐、主编捞回成备选」与
 *    「模型判备选」是两件事，混成一个字段就再也分不出来。
 * 4. **默认按选题看**（09-22 反馈第五项）：同产品、同事件的多帖合成一行，
 *    推荐位按选题算；逐帖台账切到「按贴文」照样全在。
 * 5. **主信息是「图 + 具体对象 + 一句推荐理由」**（第六项）：发布时间、关键缺口、
 *    查重在行上；六维依据、制作条件、热度折叠在展开里。
 */
import { useMemo, useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'

import { Empty, ErrorBox, Head, Loading, Table, TierBadge, bj } from '@/components/Bits'
import { DedupBadge, Fold, GapList, Hits, Thumb } from '@/components/Judged'
import { Td, Th } from '@/components/ui'
import { DIMS, FACT_SOURCE, NOVELTY, TIER_LABEL } from '@/lib/constants'
import { errText } from '@/lib/api'
import { useAuth } from '@/lib/auth'
import { normGaps, three, titleOf } from '@/lib/judgement'
import {
  useJudgements,
  useOverride,
  useRestoreExcluded,
  useRoundExclusions,
  useRounds,
  useSetFirstBatch,
  useTopics,
} from '@/lib/queries'
import type { JudgementRow, Tier, Topic } from '@/lib/types'

const TIERS: { key: Tier | 'all'; label: string }[] = [
  { key: 'all', label: '全部' },
  { key: 'recommend', label: '推荐' },
  { key: 'alternate', label: '备选' },
  { key: 'pending_check', label: '待核' },
  { key: 'not_recommend', label: '不推荐' },
]

type View = 'topic' | 'post'

/** 一行 = 一个选题（按选题看）或一条贴文（按贴文看） */
interface Line {
  /** 占这一行位置的那一帖：选题里**生效档最高**的（人工改过档也算），改档与首批都作用在它上面 */
  head: JudgementRow
  members: JudgementRow[]
  topic?: Topic
}

export function Ledger() {
  const [sp, setSp] = useSearchParams()
  const rounds = useRounds({ limit: 20 })
  const roundId = Number(sp.get('round')) || rounds.data?.find((r) => r.kind === 'task')?.id
  const [tier, setTier] = useState<Tier | 'all'>('all')
  const [onlyGap, setOnlyGap] = useState(false)
  const [view, setView] = useState<View>('topic')
  const [open, setOpen] = useState<string | null>(null)

  const js = useJudgements(roundId, {
    tier: tier === 'all' ? undefined : tier,
    has_gap: onlyGap || undefined,
  })
  const topics = useTopics(roundId)
  const { me } = useAuth()
  const canEdit = me?.role === 'operator' || me?.role === 'superadmin'
  const override = useOverride(roundId)
  const firstBatch = useSetFirstBatch(roundId)

  const pinned = useMemo(
    () => (js.data ?? []).filter((j) => j.first_batch).map((j) => j.candidate_key),
    [js.data],
  )

  const lines: Line[] = useMemo(() => {
    const rows = js.data ?? []
    if (view === 'post') return rows.map((r) => ({ head: r, members: [r] }))
    const byTopic = new Map((topics.data ?? []).map((t) => [t.topic_key, t]))
    // 服务端已按生效档排好：每个选题第一次出现的位置就是它最高那一档的位置
    const out: Line[] = []
    const at = new Map<string, number>()
    for (const r of rows) {
      const k = r.topic_key ?? r.candidate_key
      const i = at.get(k)
      if (i === undefined) {
        at.set(k, out.length)
        out.push({ head: r, members: [r], topic: byTopic.get(k) })
      } else {
        // 不换 head：第一次出现的就是生效档最高的那一帖（服务端按生效档排）
        out[i].members.push(r)
      }
    }
    return out
  }, [js.data, topics.data, view])

  const posts = js.data?.length ?? 0

  return (
    <section>
      <Head
        title="判断台账"
        desc="窗口内每条都在这里，包括不推荐的。默认按选题看：同一产品或事件的多帖合成一行，只占一个推荐位。没有分数栏。"
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
        <div className="ml-2 inline-flex overflow-hidden rounded-[var(--r-sm)] border border-rule">
          {(['topic', 'post'] as View[]).map((v) => (
            <button
              key={v}
              onClick={() => setView(v)}
              className={[
                'border-none px-2.5 py-1 text-[12.5px]',
                view === v ? 'bg-accent-soft font-semibold text-accent' : 'bg-surface text-ink',
              ].join(' ')}
            >
              {v === 'topic' ? '按选题' : '按贴文'}
            </button>
          ))}
        </div>
        <label className="ml-2 flex items-center gap-1.5 text-[12.5px] text-muted">
          <input type="checkbox" checked={onlyGap} onChange={(e) => setOnlyGap(e.target.checked)} />
          只看有缺口的
        </label>
        <span className="ml-auto text-[12.5px] text-muted">
          {js.data ? `${posts} 帖${view === 'topic' ? ` · ${lines.length} 个选题` : ''}` : ''}
          {pinned.length > 0 && ` · 首批指定了 ${pinned.length} 条`}
        </span>
      </div>

      <Excluded roundId={roundId} canEdit={canEdit} />

      {js.isLoading && <Loading what="台账" />}
      {js.error && <ErrorBox error={js.error} />}
      {override.error && <ErrorBox error={override.error} />}
      {js.data?.length === 0 && <Empty>这一档下没有条目。</Empty>}

      {lines.length > 0 && (
        <div className="overflow-x-auto">
          <Table
            head={
              <tr>
                <Th>图</Th>
                <Th>对象与推荐理由</Th>
                <Th>档</Th>
                <Th>发布</Th>
                <Th>关键缺口</Th>
                <Th>查重</Th>
                <Th right>操作</Th>
              </tr>
            }
          >
            {lines.map((l) => (
              <Row
                key={l.head.candidate_key}
                line={l}
                roundId={roundId}
                open={open === l.head.candidate_key}
                onToggle={() =>
                  setOpen(open === l.head.candidate_key ? null : l.head.candidate_key)
                }
                canEdit={canEdit}
                onOverride={(to, reason) =>
                  override.mutate({ key: l.head.candidate_key, to_tier: to, reason })
                }
                onPin={() =>
                  firstBatch.mutate(
                    l.head.first_batch
                      ? pinned.filter((k) => k !== l.head.candidate_key)
                      : [...pinned, l.head.candidate_key],
                  )
                }
              />
            ))}
          </Table>
        </div>
      )}
    </section>
  )
}

function Row({
  line,
  roundId,
  open,
  onToggle,
  canEdit,
  onOverride,
  onPin,
}: {
  line: Line
  roundId?: number
  open: boolean
  onToggle: () => void
  canEdit: boolean
  onOverride: (to: Tier, reason: string) => void
  onPin: () => void
}) {
  const j = line.head
  const changed = j.effective_tier !== j.tier
  const title = line.topic?.headline?.trim() || titleOf(j.headline, j.three_sentences)
  const flags = j.check_flags?.length ?? 0
  return (
    <>
      <tr className="border-t border-rule align-top">
        <Td>
          <Thumb hash={j.cover} size={64} alt={title} />
        </Td>
        <Td>
          <button onClick={onToggle} className="border-none bg-transparent p-0 text-left">
            <div className="text-[13.5px] font-medium text-ink">{title || <Untitled j={j} />}</div>
            <div className="mt-0.5 text-[11.5px] text-muted">
              {j.account} · {j.candidate_key}
              {Math.max(line.members.length, line.topic?.members.length ?? 0) > 1 && (
                <span className="ml-1.5 rounded-[var(--r-sm)] bg-accent-soft px-1.5 text-accent">
                  同一选题 {Math.max(line.members.length, line.topic?.members.length ?? 0)} 帖
                  {line.topic && line.members.length < line.topic.members.length && '（其余在别的档）'}
                </span>
              )}
            </div>
          </button>
        </Td>
        <Td>
          <TierBadge tier={j.effective_tier} sm awaiting={j.decision_gaps > 0} />
          {changed && (
            // 「模型判不推荐、主编捞回成备选」与「模型判备选」是两件事
            <div className="mt-1 text-[11px] text-muted">
              原判 {TIER_LABEL[j.tier] ?? j.tier} · {j.override_actor} 改的
            </div>
          )}
          {j.first_batch && <div className="mt-1 text-[11px] text-accent">已进首批</div>}
        </Td>
        <Td className="tnum whitespace-nowrap text-[12px] text-muted">{bj(j.posted_at)}</Td>
        <Td>
          {j.decision_gaps === 0 && flags === 0 && j.image_seen ? (
            <span className="text-[12px] text-dim">——</span>
          ) : (
            <span className="text-[12px] text-warn">
              {j.decision_gaps > 0 && `${j.decision_gaps} 条影响判断`}
              {flags > 0 && `${j.decision_gaps > 0 ? ' · ' : ''}${flags} 条留痕`}
            </span>
          )}
          {!j.image_seen && <div className="text-[11.5px] text-warn">没读到实图</div>}
        </Td>
        <Td>
          <DedupBadge verdict={j.comparison?.verdict ?? ''} sm />
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
      {open && <Detail line={line} roundId={roundId} />}
    </>
  )
}

/**
 * 展开行。顺序照 09-22 反馈第六项：选题成员 → 推荐理由与三句话 → 关键缺口 → 查重，
 * 六维依据、制作条件与热度折叠在后面。
 */
function Detail({ line, roundId }: { line: Line; roundId?: number }) {
  const j = line.head
  const t = three(j.three_sentences)
  const gaps = normGaps(j.gaps)
  const decision = gaps.filter((g) => g.level === 'decision')
  const rest = gaps.filter((g) => g.level !== 'decision')
  const notes = line.topic?.synthesis.per_member ?? []
  return (
    <tr className="border-t border-rule bg-surface-2">
      <td colSpan={7} className="px-3 py-3">
        {line.members.length > 1 && (
          <div className="mb-3">
            <div className="mb-1 text-[12px] font-semibold text-muted">
              这个选题的 {line.members.length} 帖（推荐位只算一个）
            </div>
            <ul className="m-0 space-y-1 pl-0">
              {line.members.map((m) => {
                const n = notes.find((x) => x.candidate_key === m.candidate_key)
                return (
                  <li key={m.candidate_key} className="flex list-none items-start gap-2 text-[12.5px]">
                    <Thumb hash={m.cover} size={40} />
                    <div>
                      <Link
                        to={`/ledger/${roundId}/${encodeURIComponent(m.candidate_key)}`}
                        className="text-accent no-underline"
                      >
                        {m.account} · {m.candidate_key}
                      </Link>
                      <span className="ml-1.5 text-muted">{TIER_LABEL[m.effective_tier]}</span>
                      {m.candidate_key === line.topic?.primary_key && (
                        <span className="ml-1.5 text-muted">（代表帖）</span>
                      )}
                      {n && (
                        <div className="text-muted">
                          {n.is_duplicate ? '与其余帖子重复' : n.new_info ? `新增：${n.new_info}` : ''}
                        </div>
                      )}
                    </div>
                  </li>
                )
              })}
            </ul>
            {(line.topic?.synthesis.shared_facts.length ?? 0) > 0 && (
              <div className="mt-1 text-[12.5px] text-muted">
                共同事实：{line.topic?.synthesis.shared_facts.join('；')}
              </div>
            )}
            {(line.topic?.synthesis.unsupported.length ?? 0) > 0 && (
              <div className="mt-1 text-[12.5px] text-warn">
                这些「新增」在该帖材料里核不到，请过一眼：{line.topic?.synthesis.unsupported.join('；')}
              </div>
            )}
          </div>
        )}

        <div className="grid gap-4 md:grid-cols-2">
          <div>
            <div className="mb-1 text-[12px] font-semibold text-muted">是什么 · 为什么值得看 · 依据</div>
            <ol className="m-0 space-y-1 pl-4 text-[13px]">
              <li>{t.what || '——'}</li>
              <li>{t.why || '——'}</li>
              <li>{t.grounds || '——'}</li>
            </ol>
            {j.novelty?.kind && (
              <div className="mt-1.5 text-[12.5px] text-muted">
                看点类型：{NOVELTY[j.novelty.kind] ?? j.novelty.kind}
                {j.novelty.prior_evidence && `（旧款依据：${j.novelty.prior_evidence}）`}
              </div>
            )}
            <div className="mb-1 mt-3 text-[12px] font-semibold text-muted">影响判断的缺口</div>
            <GapList gaps={decision} empty="没有影响判断的缺口。" />
          </div>
          <div>
            <div className="mb-1 flex items-center gap-2 text-[12px] font-semibold text-muted">
              查重 <DedupBadge verdict={j.comparison.verdict} sm />
            </div>
            <Hits hits={j.comparison.hits ?? []} />
            {j.comparison.note && (
              <div className="mt-1 text-[12.5px] text-muted">{j.comparison.note}</div>
            )}
            {(j.check_flags?.length ?? 0) > 0 && (
              <>
                <div className="mb-1 mt-3 text-[12px] font-semibold text-muted">留痕（口径兜底、依据核对）</div>
                <ul className="m-0 space-y-0.5 pl-4 text-[12.5px] text-warn">
                  {j.check_flags.map((f, i) => (
                    <li key={i}>{f}</li>
                  ))}
                </ul>
              </>
            )}
          </div>
        </div>

        <Fold title="六维依据（值得推荐的价值是核心，其余为支撑）">
          <div className="space-y-1">
            {j.dims.map(([dim, d]) => (
              <div key={dim} className="text-[12.5px]">
                <span className="mr-1.5 text-muted">{DIMS[dim]?.full ?? dim}</span>
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
        </Fold>
        <Fold title="制作条件、影响成稿的缺口与热度">
          <div className="text-[12.5px] text-muted">
            事实来源 {FACT_SOURCE[j.readiness?.fact_source ?? 'unknown'] ?? '不详'} · 可作配图{' '}
            {j.readiness?.usable_images ?? 0} 张 · 资料{j.readiness?.material_complete ? '齐' : '未齐'}
            {j.readiness?.note && ` · ${j.readiness.note}`}
          </div>
          <div className="mt-2">
            <GapList gaps={rest} empty="没有影响成稿的缺口或表达边界。" />
          </div>
          <div className="mt-2 text-[12.5px] text-muted">热度：{j.heat_note || '——'}（是输入，不是维度）</div>
        </Fold>

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
            看全貌（全部实图、原文、深核、改档历史）
          </Link>
          <a href={j.url} target="_blank" rel="noreferrer" className="text-accent no-underline">
            看原贴
          </a>
        </div>
      </td>
    </tr>
  )
}

/**
 * 这一轮被规则挡下的那几条。
 *
 * **它们没有经过模型**，所以台账主表里那一行的六维全是「不明」。
 * 单独摆在这里，把「它为什么没判」直接回答掉——依据是哪次否决、她当时怎么说的、
 * 两个判据各是多少。
 *
 * 捞回只对这一轮这一条生效，规则本身还开着。捞回之后它要重新走判断——
 * 它当初根本没送模型，没有结论可以拿来改档。
 */
function Excluded({ roundId, canEdit }: { roundId?: number; canEdit: boolean }) {
  const q = useRoundExclusions(roundId)
  const m = useRestoreExcluded(roundId)
  const [openKey, setOpenKey] = useState<string | null>(null)
  const [reason, setReason] = useState('')
  const rows = q.data?.明细 ?? []
  if (rows.length === 0) return null

  return (
    <div className="mb-3 rounded-[var(--r)] border border-rule bg-surface-2 px-3 py-2">
      <div className="flex flex-wrap items-baseline gap-2">
        <span className="text-[13.5px] font-semibold">规则排除</span>
        <span className="text-[12.5px] text-muted">
          挡下 {q.data?.挡下 ?? 0} 条
          {(q.data?.捞回 ?? 0) > 0 && ` · 捞回 ${q.data?.捞回} 条`}
          ——它们没送模型，所以主表里那几行的六维是「不明」
        </span>
      </div>
      <div className="mt-1.5 space-y-1.5">
        {rows.map((r) => (
          <div key={r.候选} className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1.5">
            <div className="flex flex-wrap items-center gap-2 text-[12.5px]">
              <code className="text-[12px]">{r.候选}</code>
              {r.已捞回 ? (
                <span className="rounded-[var(--r-sm)] bg-accent-soft px-1.5 py-0.5 text-[11.5px] text-accent">
                  已捞回
                </span>
              ) : (
                <span className="text-muted">
                  同一事实 {r.同一事实.toFixed(2)} · 新料 {r.新料.toFixed(2)}
                </span>
              )}
              <span className="text-muted">与《{r.否过的条目}》同一件事</span>
            </div>
            {r.原话 && (
              <blockquote className="mt-1 border-l-2 border-rule pl-2 text-[12.5px] leading-relaxed text-muted">
                {r.原话}
              </blockquote>
            )}
            {r.已捞回 && r.捞回理由 && (
              <div className="mt-0.5 text-[12.5px] text-muted">
                捞回（{r.捞回人}）：{r.捞回理由}
              </div>
            )}
            {canEdit && !r.已捞回 && (
              <div className="mt-1">
                {openKey !== r.候选 ? (
                  <button
                    onClick={() => setOpenKey(r.候选)}
                    className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-0.5 text-[12px]"
                  >
                    捞回
                  </button>
                ) : (
                  <div className="flex flex-wrap items-center gap-1">
                    <input
                      value={reason}
                      onChange={(e) => setReason(e.target.value)}
                      placeholder="捞回理由（必填）"
                      className="w-[220px] rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
                    />
                    <button
                      disabled={!reason.trim() || m.isPending}
                      onClick={() =>
                        m.mutate(
                          { key: r.候选, reason: reason.trim() },
                          {
                            onSuccess: () => {
                              setOpenKey(null)
                              setReason('')
                            },
                          },
                        )
                      }
                      className="rounded-[var(--r-sm)] bg-accent px-2 py-0.5 text-[12px] text-white disabled:opacity-50"
                    >
                      存
                    </button>
                    <button
                      onClick={() => setOpenKey(null)}
                      className="rounded-[var(--r-sm)] border border-rule bg-surface px-1.5 py-0.5 text-[12px]"
                    >
                      取消
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        ))}
      </div>
      {m.error && <div className="mt-1 text-[12px] text-bad">{errText(m.error)}</div>}
      <p className="mt-1.5 text-[12px] text-muted">
        捞回只对这一轮这一条生效，规则本身还开着（规则在「判断框架」那一页开关）。
        <b>捞回之后要重跑判断那一步</b>——它当初没送模型，没有结论可以改档。
      </p>
    </div>
  )
}

/**
 * 没有三句话的那一行怎么显示。
 *
 * 被规则挡下的条目**没送过模型**，自然没有「什么变了」可写——照抄空字符串的话，
 * 台账上就是一行没有标题的东西，人看不出那是什么。走查时一眼就发现了。
 *
 * 这里显示的是**它为什么在这儿**，不是编一个标题：与哪条否决撞上了。
 * 依据的全文在上面「规则排除」那一块，那里有原话和两个判据值。
 */
function Untitled({ j }: { j: JudgementRow }) {
  if (j.comparison.verdict === 'same_fact_no_gain' && j.comparison.against) {
    return (
      <span className="text-muted">
        规则排除 · 与《{j.comparison.against}》同一件事
      </span>
    )
  }
  // 其余没有标题的情形（没读到实图的待核也可能是空的）
  return <span className="text-muted">（没有标题）</span>
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

function verdictLabel(v: string): string {
  return { yes: '成立', no: '不成立', unclear: '不明' }[v] ?? v
}
