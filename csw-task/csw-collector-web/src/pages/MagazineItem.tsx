/**
 * 杂志条目详情：裁图大图、整页图上框出位置、描述译文与商品、出处，复制 OSS 地址。
 *
 * 框按清单的 bbox（PDF 点）÷ page_size 换成百分比叠在整页图上——
 * 整页图在刊译台上传前缩过，像素对不上，比例对得上。
 */
import { Link, useParams } from 'react-router-dom'

import { ErrorBox, Head, Loading } from '@/components/Bits'
import { splitTitle } from '@/components/Magazine'
import { useKbMagazineItem } from '@/lib/queries'

function Copy({ text, label }: { text: string; label: string }) {
  return (
    <button
      onClick={() => void navigator.clipboard?.writeText(text)}
      disabled={!text}
      className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px] disabled:opacity-50"
    >
      {label}
    </button>
  )
}

function box(bbox: number[] | null, size: number[] | null) {
  if (!bbox || !size || bbox.length < 4 || size.length < 2 || !size[0] || !size[1]) return null
  const [x0, y0, x1, y1] = bbox
  const [w, h] = size
  return {
    left: `${(x0 / w) * 100}%`,
    top: `${(y0 / h) * 100}%`,
    width: `${((x1 - x0) / w) * 100}%`,
    height: `${((y1 - y0) / h) * 100}%`,
  }
}

export function MagazineItem() {
  const { id } = useParams()
  const q = useKbMagazineItem(Number(id))
  const m = q.data
  const { head, source } = splitTitle(m?.title ?? '')
  const frame = m ? box(m.bbox, m.page_size) : null

  return (
    <section>
      <Head title={head || '杂志条目'} desc="杂志背景：日文杂志的译文与裁图，只作背景参照，不进判断。" />
      <Link to="/kb" className="mb-3 inline-block text-[12.5px] text-accent no-underline">
        ← 回知识库
      </Link>
      {q.isLoading && <Loading what="条目" />}
      {q.error && <ErrorBox error={q.error} />}
      {m && (
        <>
          <div className="mb-3 flex flex-wrap items-center gap-2 text-[12px]">
            <span className="rounded-[var(--r-sm)] border border-rule px-2 py-0.5">{source}</span>
            {m.category && (
              <span className="rounded-[var(--r-sm)] border border-rule px-2 py-0.5 text-muted">{m.category}</span>
            )}
            <span className={m.fused_ready ? 'text-ok' : 'text-muted'}>
              融合向量{m.fused_ready ? '已算' : '未算'}
            </span>
            <span className={m.pure_ready ? 'text-ok' : 'text-muted'}>
              纯图向量{m.pure_ready ? '已算' : '未算'}
            </span>
            <span className="ml-auto flex gap-2">
              <Copy text={m.image_url} label="复制裁图地址" />
              <Copy text={m.page_url} label="复制整页地址" />
            </span>
          </div>
          <div className="grid gap-3 lg:grid-cols-3">
            <div className="rounded-[var(--r)] border border-rule bg-surface p-3">
              <img src={m.image_url} alt={head} className="w-full rounded-[var(--r-sm)]" />
            </div>
            <div className="rounded-[var(--r)] border border-rule bg-surface p-3">
              <div className="mb-2 text-[13px] font-medium">整页 · 框出这张图的位置</div>
              {m.page_url ? (
                <div className="relative">
                  <img src={m.page_url} alt="整页" className="block w-full rounded-[var(--r-sm)]" />
                  {frame && (
                    <i
                      className="absolute rounded-[2px] border-2 border-accent bg-accent/10"
                      style={frame}
                    />
                  )}
                </div>
              ) : (
                <p className="text-[12.5px] text-muted">清单里没有整页图。</p>
              )}
            </div>
            <div className="rounded-[var(--r)] border border-rule bg-surface p-3 text-[12.5px]">
              <div className="mb-1 text-[13px] font-medium">描述、译文与商品</div>
              <p className="whitespace-pre-line">{m.body.replace(/\*\*/g, '').replace(/^#+\s*/gm, '')}</p>
              <dl className="mt-3 grid grid-cols-[5em_minmax(0,1fr)] gap-x-2 gap-y-1">
                <dt className="text-muted">出处</dt>
                <dd className="m-0">{source}{m.issue_date ? `（${m.issue_date.slice(0, 10)}）` : ''}</dd>
                <dt className="text-muted">PDF 页</dt>
                <dd className="m-0">第 {m.pdf_index + 1} 页</dd>
                <dt className="text-muted">书</dt>
                <dd className="mono m-0 break-all">{m.book_key}</dd>
                <dt className="text-muted">品牌</dt>
                <dd className="m-0">
                  {m.brand ? (
                    <Link to={`/kb?brand=${encodeURIComponent(m.brand)}`} className="text-accent no-underline">
                      {m.brand}
                    </Link>
                  ) : (
                    '——'
                  )}
                </dd>
              </dl>
            </div>
          </div>
        </>
      )}
    </section>
  )
}
