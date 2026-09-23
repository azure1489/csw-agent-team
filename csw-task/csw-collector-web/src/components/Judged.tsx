/**
 * 判断台账、条目详情、Van 模式、待核结转共用的几件：实图、缺口、查重。
 *
 * 09-22 反馈第六项：Van 要**直接看图**判断外观、设计与审美，图片描述代替不了；
 * 第四、八项：查重要列出命中的文章与状态，缺口要分级、写清谁处理与下一步。
 * 同一件东西在四页长得一样，人才不会以为是四件事。
 */
import { useCallback, useEffect, useRef, useState } from 'react'

import { Badge } from '@/components/ui'
import { DEDUP, GAP_LEVEL, HIT_STATE, OWNER } from '@/lib/constants'
import { mediaUrl } from '@/lib/judgement'
import type { ComparisonHit, Gap } from '@/lib/types'

/** 缩略图。取不到（清理过、下载失败）就显示「无图」，不留一个破图标 */
export function Thumb({ hash, size = 56, alt = '' }: { hash: string | null; size?: number; alt?: string }) {
  const [bad, setBad] = useState(false)
  // 同一个组件换了图（翻页、刷新）要重新试，不能一直停在「无图」
  useEffect(() => setBad(false), [hash])
  if (!hash || bad) {
    return (
      <div
        style={{ width: size, height: size }}
        className="flex shrink-0 items-center justify-center rounded-[var(--r-sm)] border border-rule bg-surface-2 text-[11px] text-dim"
      >
        无图
      </div>
    )
  }
  return (
    <img
      src={mediaUrl(hash)}
      alt={alt}
      loading="lazy"
      onError={() => setBad(true)}
      style={{ width: size, height: size }}
      className="shrink-0 rounded-[var(--r-sm)] border border-rule bg-surface-2 object-cover"
    />
  )
}

/**
 * 图集：网格缩略，点开看大图。Esc 关、左右键翻，**手机上有关闭与翻页按钮**；
 * 打开时焦点进对话框、页面不滚，关上后焦点回到点开的那张缩略图。
 */
export function Gallery({ hashes, size = 132 }: { hashes: string[]; size?: number }) {
  const [open, setOpen] = useState<number | null>(null)
  const [bigBad, setBigBad] = useState(false)
  const opener = useRef<HTMLElement | null>(null)
  const closeBtn = useRef<HTMLButtonElement | null>(null)
  const close = useCallback(() => setOpen(null), [])
  const go = useCallback(
    (d: number) =>
      setOpen((i) => (i === null ? i : Math.min(Math.max(i + d, 0), hashes.length - 1))),
    [hashes.length],
  )

  // 列表刷新后变短：别去取不存在的那一张
  useEffect(() => {
    if (open !== null && open >= hashes.length) setOpen(hashes.length > 0 ? hashes.length - 1 : null)
  }, [open, hashes.length])
  useEffect(() => setBigBad(false), [open])

  useEffect(() => {
    if (open === null) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
      if (e.key === 'ArrowRight') {
        e.preventDefault()
        go(1)
      }
      if (e.key === 'ArrowLeft') {
        e.preventDefault()
        go(-1)
      }
    }
    const prevOverflow = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    window.addEventListener('keydown', onKey)
    closeBtn.current?.focus()
    return () => {
      window.removeEventListener('keydown', onKey)
      document.body.style.overflow = prevOverflow
      opener.current?.focus()
    }
  }, [open === null, close, go]) // eslint-disable-line react-hooks/exhaustive-deps

  if (hashes.length === 0) {
    return <div className="text-[12.5px] text-muted">这一条没有取到实图。</div>
  }
  const btn =
    'rounded-[var(--r-sm)] border border-white/40 bg-black/40 px-3 py-1.5 text-[14px] text-white disabled:opacity-30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-white'
  return (
    <>
      <div className="flex flex-wrap gap-2">
        {hashes.map((h, i) => (
          <button
            key={h}
            onClick={(e) => {
              opener.current = e.currentTarget
              setOpen(i)
            }}
            aria-label={`看第 ${i + 1} 张大图`}
            className="rounded-[var(--r-sm)] border-none bg-transparent p-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
          >
            <Thumb hash={h} size={size} alt={`第 ${i + 1} 张`} />
          </button>
        ))}
      </div>
      {open !== null && open < hashes.length && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={`大图，第 ${open + 1} / ${hashes.length} 张`}
          onClick={close}
          className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-3 bg-black/85 p-4"
        >
          {bigBad ? (
            <div className="text-[13px] text-white/80">这张图取不到（可能已经清理）。</div>
          ) : (
            <img
              src={mediaUrl(hashes[open])}
              alt={`第 ${open + 1} 张`}
              onError={() => setBigBad(true)}
              onClick={(e) => e.stopPropagation()}
              className="max-h-[78vh] max-w-full rounded-[var(--r)] object-contain"
            />
          )}
          <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
            <button className={btn} disabled={open === 0} onClick={() => go(-1)} aria-label="上一张">
              ‹ 上一张
            </button>
            <span className="tnum text-[12.5px] text-white/80">
              {open + 1} / {hashes.length}
            </span>
            <button
              className={btn}
              disabled={open === hashes.length - 1}
              onClick={() => go(1)}
              aria-label="下一张"
            >
              下一张 ›
            </button>
            <button ref={closeBtn} className={btn} onClick={close}>
              关闭
            </button>
          </div>
        </div>
      )}
    </>
  )
}

/** 只渲染 http(s) 链接：链接是模型写的，不能让一个 `javascript:` 地址变成可点的东西 */
export function safeHref(u: string | undefined | null): string | undefined {
  if (!u) return undefined
  try {
    const p = new URL(u)
    return p.protocol === 'http:' || p.protocol === 'https:' ? p.href : undefined
  } catch {
    return undefined
  }
}

export function DedupBadge({ verdict, sm }: { verdict: string; sm?: boolean }) {
  const d = DEDUP[verdict] ?? { label: verdict || '——', tone: 'gray' as const }
  return (
    <Badge tone={d.tone} sm={sm} outline={verdict === 'unrelated'}>
      {d.label}
    </Badge>
  )
}

/** 查重命中：哪篇、什么状态、重复了哪条事实。生成稿不是「已发」 */
export function Hits({ hits }: { hits: ComparisonHit[] }) {
  if (hits.length === 0) return <div className="text-[12.5px] text-muted">没有命中历史材料。</div>
  return (
    <ul className="m-0 space-y-1 pl-4 text-[12.5px]">
      {hits.map((h, i) => (
        <li key={`${h.ref_no}-${i}`}>
          {safeHref(h.url) ? (
            <a href={safeHref(h.url)} target="_blank" rel="noreferrer" className="text-accent no-underline">
              《{h.title || h.ref_no}》
            </a>
          ) : (
            <span>《{h.title || h.ref_no}》</span>
          )}
          <span className="ml-1 text-muted">
            {HIT_STATE[h.state] ?? h.state}
            {h.published_at && ` · ${h.published_at.slice(0, 10)}`}
            {!h.body_available && ' · 正文不可得'}
          </span>
          {h.dup_fact && <div className="text-ink">重复的事实：{h.dup_fact}</div>}
        </li>
      ))}
    </ul>
  )
}

/** 缺口：级别、缺什么、谁处理、已尝试、下一步 */
export function GapList({ gaps, empty = '没有缺口。' }: { gaps: Gap[]; empty?: string }) {
  if (gaps.length === 0) return <div className="text-[12.5px] text-muted">{empty}</div>
  return (
    <ul className="m-0 list-none space-y-1.5 p-0">
      {gaps.map((g, i) => {
        const lv = GAP_LEVEL[g.level] ?? GAP_LEVEL.decision
        return (
          <li key={i} className="text-[12.5px] leading-relaxed">
            <Badge tone={lv.tone} sm>
              {lv.label}
            </Badge>
            <span className="ml-1.5 text-ink">{g.what}</span>
            <div className="text-muted">
              由{OWNER[g.owner] ?? g.owner}处理
              {g.tried && `；已尝试：${g.tried}`}
              {g.next && `；下一步：${g.next}`}
            </div>
          </li>
        )
      })}
    </ul>
  )
}

/** 折叠块：技术记录默认收起，要看再点开 */
export function Fold({ title, children, open }: { title: string; children: React.ReactNode; open?: boolean }) {
  return (
    <details open={open} className="mt-3 rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
      <summary className="cursor-pointer select-none text-[13px] font-semibold text-muted">{title}</summary>
      <div className="mt-2">{children}</div>
    </details>
  )
}
