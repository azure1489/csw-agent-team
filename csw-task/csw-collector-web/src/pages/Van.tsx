/**
 * Van 模式：**手机优先的当期视图。**
 *
 * 她一天只做两类决定，这一页服务其中一类。所以：
 *
 * - **只给推荐与备选。** 手机上翻三百条不是在帮她；要看全部去判断台账，
 *   那是主编的页面。
 * - **每条只给三句话与一张图。** 六维、依据、对照材料是主编核的东西，
 *   给她看只会让她在一屏里找那三句话。
 * - **勾选只写本地。** 进不进评选由主编在引擎上代录并附她的原话——
 *   这一页替她按下「采用」，流程上那一步就被绕过去了。
 */
import { useState } from 'react'

import { Empty, ErrorBox, Head, Loading, TierBadge } from '@/components/Bits'
import { useVanMark, useVanToday } from '@/lib/queries'
import type { VanItem } from '@/lib/types'

export function Van() {
  const today = useVanToday()
  const mark = useVanMark()

  if (today.isLoading) return <Loading what="今天的" />
  if (today.error) return <ErrorBox error={today.error} />

  const items = today.data?.items ?? []
  return (
    <section className="mx-auto max-w-[680px]">
      <Head
        title="今天的"
        desc={
          today.data?.round_id
            ? `推荐 ${items.filter((i) => i.tier === 'recommend').length} 条、备选 ${items.filter((i) => i.tier === 'alternate').length} 条`
            : undefined
        }
      />
      {!today.data?.round_id && <Empty>今天还没开始。</Empty>}
      {today.data?.round_id && items.length === 0 && (
        <Empty>这一轮没有推荐或备选。要看全部去判断台账。</Empty>
      )}
      <div className="space-y-3">
        {items.map((it) => (
          <Item
            key={it.candidate_key}
            it={it}
            onMark={(m, note, remove) =>
              mark.mutate({ candidate_key: it.candidate_key, mark: m, note, remove })
            }
          />
        ))}
      </div>
      {mark.error && (
        <div className="mt-3">
          <ErrorBox error={mark.error} />
        </div>
      )}
      <p className="mt-6 text-[12px] text-muted">
        勾选只记在这里，不回写引擎。要采用哪几条，请告诉主编——他会带着你的原话录进流程。
      </p>
    </section>
  )
}

function Item({
  it,
  onMark,
}: {
  it: VanItem
  onMark: (mark: string, note?: string, remove?: boolean) => void
}) {
  const [noteOpen, setNoteOpen] = useState(false)
  const [note, setNote] = useState('')
  const has = (m: string) => it.marks.includes(m)

  return (
    <article className="rounded-[var(--r-md)] border border-rule bg-surface p-3.5">
      <div className="mb-1.5 flex items-center gap-2">
        <TierBadge tier={it.tier} sm />
        <span className="text-[12px] text-muted">{it.brand}</span>
        {it.heat_note && <span className="ml-auto text-[11.5px] text-dim">{it.heat_note}</span>}
      </div>
      <h3 className="m-0 text-[15px] font-semibold leading-snug">{it.title}</h3>
      <ol className="mb-0 mt-2 space-y-1 pl-4 text-[13.5px] leading-relaxed">
        <li>{it.three_sentences.what_changed}</li>
        <li>{it.three_sentences.why_it_matters}</li>
        <li>{it.three_sentences.how_different}</li>
      </ol>
      {it.look && <p className="mb-0 mt-2 text-[12.5px] text-muted">图里：{it.look}</p>}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <MarkButton on={has('like')} label="要这条" onClick={() => onMark('like', '', has('like'))} />
        <MarkButton on={has('doubt')} label="存疑" onClick={() => onMark('doubt', '', has('doubt'))} />
        <button
          onClick={() => setNoteOpen(!noteOpen)}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-3 py-1.5 text-[13px]"
        >
          说一句
        </button>
        <a
          href={it.url}
          target="_blank"
          rel="noreferrer"
          className="ml-auto text-[12.5px] text-accent no-underline"
        >
          看原贴
        </a>
      </div>

      {noteOpen && (
        <div className="mt-2 flex gap-2">
          <input
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="你的原话，会一字不改地留着"
            className="grow rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1.5 text-[13px]"
          />
          <button
            disabled={!note.trim()}
            onClick={() => {
              onMark('note', note.trim())
              setNote('')
              setNoteOpen(false)
            }}
            className="rounded-[var(--r-sm)] bg-van px-3 py-1.5 text-[13px] text-white disabled:opacity-50"
          >
            记下
          </button>
        </div>
      )}
      {has('note') && <p className="mb-0 mt-2 text-[12.5px] text-van">已经写过一句</p>}
    </article>
  )
}

function MarkButton({ on, label, onClick }: { on: boolean; label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={[
        'rounded-[var(--r-sm)] border px-3 py-1.5 text-[13px]',
        on ? 'border-van bg-van-soft font-semibold text-van' : 'border-rule bg-surface text-ink',
      ].join(' ')}
    >
      {on ? `✓ ${label}` : label}
    </button>
  )
}
