/**
 * 杂志背景的结果卡与图格。杂志是**背景参照**，不进判断——卡片上不出现结论档、范例这类判断用语。
 */
import { Link } from 'react-router-dom'

import { Empty } from '@/components/Bits'
import type { MagazineHit, MagazineItem } from '@/lib/types'

const ROUTE_CN: Record<string, string> = { fts: '全文', vector: '向量', image: '以图' }

/** 标题是「品牌 商品 · 刊名 期号 P.xx」，拆成两段显示 */
export function splitTitle(t: string): { head: string; source: string } {
  const i = t.lastIndexOf(' · ')
  return i < 0 ? { head: t, source: '' } : { head: t.slice(0, i), source: t.slice(i + 3) }
}

/** 正文第一段是画面描述，译文与商品表在后面；卡片上只给一小段 */
function snippet(body: string, n = 110) {
  const s = body.replace(/[#*]/g, '').replace(/\s+/g, ' ').trim()
  return s.length > n ? `${s.slice(0, n)}…` : s
}

export function MagCard({ m }: { m: MagazineHit | MagazineItem }) {
  const { head, source } = splitTitle(m.title)
  const hit = 'routes' in m ? m : null
  return (
    <div className="grid grid-cols-[88px_minmax(0,1fr)] gap-3 rounded-[var(--r)] border border-rule bg-surface p-2.5">
      <Link to={`/kb/magazine/${m.id}`}>
        <img
          src={m.image_url}
          alt={head}
          loading="lazy"
          className="h-[88px] w-[88px] rounded-[var(--r-sm)] bg-surface-2 object-cover"
        />
      </Link>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5 text-[11.5px] text-muted">
          <span>{source}</span>
          {hit?.routes.map((r) => (
            <span key={r} className="rounded-[var(--r-sm)] border border-rule px-1.5">
              {ROUTE_CN[r] ?? r}
            </span>
          ))}
          {hit?.similarity != null && (
            <span className="rounded-[var(--r-sm)] bg-accent-soft px-1.5 text-accent">
              相似 {hit.similarity.toFixed(2)}
            </span>
          )}
        </div>
        <Link
          to={`/kb/magazine/${m.id}`}
          className="mt-0.5 block text-[13px] font-medium text-ink no-underline"
        >
          {head}
        </Link>
        <details className="mt-0.5 text-[12px] text-muted">
          <summary className="cursor-pointer text-accent">描述与译文</summary>
          {snippet(m.body, 400)}
        </details>
      </div>
    </div>
  )
}

export function MagList({ items }: { items: (MagazineHit | MagazineItem)[] }) {
  if (items.length === 0)
    return <Empty>杂志背景里没找到。这只说明已入库的杂志里没有，不说明没出现过。</Empty>
  return (
    <div className="grid gap-2 lg:grid-cols-2">
      {items.map((m) => (
        <MagCard key={m.id} m={m} />
      ))}
    </div>
  )
}

/** 以图搜图的结果：图为主，文字只给出处与相似度 */
export function MagGrid({ items }: { items: MagazineHit[] }) {
  if (items.length === 0)
    return <Empty>没有相近的图。纯图向量可能还在回填，统计卡上有进度。</Empty>
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(130px,1fr))] gap-2">
      {items.map((m) => {
        const { head, source } = splitTitle(m.title)
        return (
          <Link
            key={m.id}
            to={`/kb/magazine/${m.id}`}
            className="overflow-hidden rounded-[var(--r-sm)] border border-rule bg-surface text-ink no-underline"
          >
            <img
              src={m.image_url}
              alt={head}
              loading="lazy"
              className="aspect-square w-full bg-surface-2 object-cover"
            />
            <div className="px-1.5 py-1 text-[11px] leading-snug text-muted">
              <b className="block truncate text-[11.5px] text-ink">{head}</b>
              {source}
              {m.similarity != null && ` · ${m.similarity.toFixed(2)}`}
            </div>
          </Link>
        )
      })}
    </div>
  )
}
