/**
 * 知识库：**同步到哪天了、库里有什么、搜得到什么。**
 *
 * 水位排在最前面：它回答的是「今天的判断有没有拿到昨天的已发条目」——
 * 「库里有两万条」好看，但回答不了这个问题。
 */
import { useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Stat, Table, bj } from '@/components/Bits'
import { MagGrid, MagList } from '@/components/Magazine'
import { Td, Th } from '@/components/ui'
import { errText } from '@/lib/api'
import { atLeast, useAuth } from '@/lib/auth'
import {
  useKbBrand,
  useKbImageSearch,
  useKbMagazineSearch,
  useKbSearch,
  useKbStatus,
} from '@/lib/queries'
import type { KbDocRow, MagazineStatus } from '@/lib/types'

const KIND_CN: Record<string, string> = {
  published_item: '已发条目',
  example: '范例',
  generated_post: '已生成贴文',
  decision: '历次决定',
}

const SOURCE_CN: Record<string, string> = {
  ledger_posts: '引擎已发条目',
  ledger_decisions: '引擎历次决定',
  csw_generated: 'csw 已生成贴文',
  examples: '范例目录',
  magazine: '刊译台杂志清单',
}

type Scope = 'ref' | 'mag'
type Mode = 'text' | 'image'

/** 两个并排的切换钮 */
function Seg<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T
  options: [T, string][]
  onChange: (v: T) => void
}) {
  return (
    <span className="inline-flex overflow-hidden rounded-[var(--r-sm)] border border-rule">
      {options.map(([v, label]) => (
        <button
          key={v}
          type="button"
          onClick={() => onChange(v)}
          className={
            'border-0 border-r border-rule px-2.5 py-1 text-[12.5px] last:border-r-0 ' +
            (v === value ? 'bg-accent-soft font-medium text-accent' : 'bg-surface text-muted')
          }
        >
          {label}
        </button>
      ))}
    </span>
  )
}

function magNote(m: MagazineStatus) {
  const left = m.eligible - m.fused_done + (m.pure_target - m.pure_done)
  return `融合 ${m.fused_done} / ${m.eligible} · 纯图 ${m.pure_done} / ${m.pure_target}${left > 0 ? ' · 回填中，正式轮期间暂停' : ''}`
}

export function Kb() {
  const status = useKbStatus()
  const { me } = useAuth()
  const [params] = useSearchParams()
  const [q, setQ] = useState('')
  const [submitted, setSubmitted] = useState('')
  // 范围默认判断参考库：杂志是背景，不该是搜索的默认答案
  const [scope, setScope] = useState<Scope>('ref')
  const [mode, setMode] = useState<Mode>('text')
  const [brand, setBrand] = useState(params.get('brand') ?? '')
  const search = useKbSearch(scope === 'ref' ? submitted : '')
  const magSearch = useKbMagazineSearch(scope === 'mag' ? submitted : '')
  const byBrand = useKbBrand(brand)
  const canImage = atLeast(me?.role, 'operator')

  const s = status.data
  return (
    <section>
      <Head
        title="知识库"
        desc="判断时的对照材料都从这里来。水位比条数重要：它回答「今天的判断有没有拿到昨天的已发条目」。"
      />

      {status.isLoading && <Loading what="水位" />}
      {status.error && <ErrorBox error={status.error} />}

      {s && (
        <>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Stat label="参考库条目" value={s.docs} />
            <Stat
              label="待算向量"
              value={s.待算向量}
              note={s.待算向量 > 0 ? '换过模型的也算' : undefined}
              tone={s.待算向量 > 0 ? 'warn' : undefined}
            />
            <Stat label="品牌" value={s.品牌} note={`别名 ${s.别名}`} />
            <Stat label="向量模型" value={<span className="text-[13px]">{s.embed_model}</span>} />
          </div>
          {s.magazine && s.magazine.docs > 0 && (
            <div className="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2">
              <Stat
                label="杂志背景（不进判断）"
                value={`${s.magazine.books} 本 · ${s.magazine.docs} 条`}
                note={magNote(s.magazine)}
                tone={
                  s.magazine.fused_done < s.magazine.eligible ||
                  s.magazine.pure_done < s.magazine.pure_target
                    ? 'warn'
                    : undefined
                }
              />
              <Stat
                label="最近入库的书"
                value={<span className="mono text-[13px]">{s.magazine.last_book ?? '——'}</span>}
                note="同步时间看下面「刊译台杂志清单」那一行"
              />
            </div>
          )}

          <H2>各来源同步到哪天</H2>
          {s.cursors.length === 0 ? (
            <Empty>还没同步过。去「运行与设置」看服务状态，或等夜里那两次定时。</Empty>
          ) : (
            <Table
              head={
                <tr>
                  <Th>来源</Th>
                  <Th>水位</Th>
                  <Th>最后一次同步</Th>
                </tr>
              }
            >
              {s.cursors.map((c) => (
                <tr key={c.source} className="border-t border-rule">
                  <Td>{SOURCE_CN[c.source] ?? c.source}</Td>
                  <Td className="mono text-[12.5px]">{c.cursor || '——'}</Td>
                  <Td className="tnum">{bj(c.synced_at)}</Td>
                </tr>
              ))}
            </Table>
          )}

          <H2>各类各多少</H2>
          <div className="flex flex-wrap gap-2">
            {Object.entries(s.docs_by_kind).map(([k, n]) => (
              <span
                key={k}
                className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px]"
              >
                {KIND_CN[k] ?? k} <b className="tnum">{n}</b>
                {k === 'example' && s.examples_expected > 0 && (
                  <span className={n < s.examples_expected ? 'ml-1 text-warn' : 'ml-1 text-muted'}>
                    / 应有 {s.examples_expected}
                  </span>
                )}
              </span>
            ))}
          </div>
          {(s.examples_missing?.length ?? 0) > 0 && (
            <p className="mt-2 text-[12.5px] text-warn">
              这几篇标成了范例、却没有整篇入库（正文为空、读取失败或与已发记录合并了）：
              <span className="mono">{s.examples_missing.join('、')}</span>
            </p>
          )}
        </>
      )}

      <H2>搜一下</H2>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <Seg<Mode>
          value={mode}
          options={[
            ['text', '文字搜'],
            ['image', '以图搜'],
          ]}
          onChange={(m) => {
            setMode(m)
            if (m === 'image') setScope('mag')
          }}
        />
        {mode === 'text' && (
          <Seg<Scope>
            value={scope}
            options={[
              ['ref', '判断参考库（四类）'],
              ['mag', '杂志背景'],
            ]}
            onChange={setScope}
          />
        )}
      </div>
      {mode === 'image' ? (
        canImage ? (
          <ImageSearch />
        ) : (
          <Empty>以图搜图一次占一块 GPU，只开给主编那一档。</Empty>
        )
      ) : (
        <>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              setSubmitted(q)
            }}
            className="mb-3 flex gap-2"
          >
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="型号、材质、联名对象……"
              className="grow rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1.5 text-[13px]"
            />
            <button
              type="submit"
              className="rounded-[var(--r-sm)] bg-accent px-3 py-1.5 text-[13px] text-white"
            >
              搜
            </button>
          </form>
          {scope === 'ref' ? (
            <p className="mb-3 text-[12px] text-muted">
              三路召回：向量 ∪ 品牌命中 ∪ 全文。<b>这条路上不重排</b>——重排占 GPU，
              而 GPU 是全进程串行的，连搜几下会把正式轮的向量化堵住。
            </p>
          ) : (
            <p className="mb-3 text-[12px] text-muted">
              只搜杂志背景：全文 ∪ 向量，不重排。杂志是背景参照，不进判断；同一本杂志可以出多条。
            </p>
          )}
          {magSearch.isFetching && <Loading what="杂志" />}
          {magSearch.error && <ErrorBox error={magSearch.error} />}
          {scope === 'mag' && magSearch.data && <MagList items={magSearch.data.items} />}
          {search.isFetching && <Loading what="结果" />}
          {search.error && <ErrorBox error={search.error} />}
          {search.data && (
            <>
              <p className="mb-2 text-[12.5px] text-muted">
                向量 {search.data.counts.vector} · 品牌 {search.data.counts.brand} · 全文{' '}
                {search.data.counts.fts} · 合并后 {search.data.counts.merged}
                {search.data.counts.truncated > 0 && ` · 截掉 ${search.data.counts.truncated}`}
                {search.data.brands_hit.length > 0 && ` · 认出品牌：${search.data.brands_hit.join('、')}`}
              </p>
              <DocList docs={search.data.docs} onBrand={setBrand} />
            </>
          )}
        </>
      )}

      {brand && (
        <>
          <H2>{brand} 的历史覆盖</H2>
          {byBrand.isFetching && <Loading what="历史" />}
          {byBrand.data && <DocList docs={byBrand.data.docs} onBrand={setBrand} />}
          {byBrand.data?.magazine && byBrand.data.magazine.length > 0 && (
            <>
              <H2>{brand} 在杂志里出现过（刊期新的在前）</H2>
              <p className="mb-2 text-[12px] text-muted">
                和上面的 CSW 历史覆盖分开：杂志不是编辑部的口味证据。
              </p>
              <MagList items={byBrand.data.magazine} />
            </>
          )}
        </>
      )}
    </section>
  )
}

function DocList({ docs, onBrand }: { docs: KbDocRow[]; onBrand: (b: string) => void }) {
  if (docs.length === 0) return <Empty>查过，无相关。</Empty>
  return (
    <div className="space-y-2">
      {docs.map((d) => (
        <div key={d.id} className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded-[var(--r-sm)] bg-surface-2 px-1.5 py-0.5 text-[11.5px] text-muted">
              {KIND_CN[d.kind] ?? d.kind}
            </span>
            {d.brand && (
              <button
                onClick={() => onBrand(d.brand)}
                className="border-none bg-transparent p-0 text-[12px] text-accent"
              >
                {d.brand}
              </button>
            )}
            {d.published_at && <span className="text-[11.5px] text-dim">{d.published_at}</span>}
            {d.backfilled && (
              <span className="text-[11.5px] text-warn" title="按类补进来的，说服力弱一档">
                补齐
              </span>
            )}
            {d.routes && d.routes.length > 0 && (
              <span className="ml-auto text-[11.5px] text-dim">{d.routes.join(' · ')}</span>
            )}
          </div>
          <div className="mt-1 text-[13px] font-medium">{d.title || d.ref_id}</div>
          <div className="mt-0.5 text-[12.5px] text-muted">{d.snippet}</div>
          {d.url && (
            <a
              href={d.url}
              target="_blank"
              rel="noreferrer"
              className="mt-1 inline-block text-[12px] text-accent no-underline"
            >
              打开
            </a>
          )}
        </div>
      ))}
    </div>
  )
}

/** 以图搜图：拖入、粘贴或选文件。≤20MB；服务端缩到 768 宽再算。 */
function ImageSearch() {
  const run = useKbImageSearch()
  const [preview, setPreview] = useState<string>('')
  const [name, setName] = useState('')
  const input = useRef<HTMLInputElement>(null)

  const pick = (f: File | null | undefined) => {
    if (!f || !f.type.startsWith('image/')) return
    if (f.size > 20 << 20) {
      setName(`${f.name} 太大（上限 20MB）`)
      return
    }
    setPreview((old) => {
      if (old) URL.revokeObjectURL(old)
      return URL.createObjectURL(f)
    })
    setName(`${f.name} · ${(f.size / 1024 / 1024).toFixed(1)} MB`)
    run.mutate(f)
  }

  // 粘贴图片：整页都接，不必先点进框里
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const item = [...(e.clipboardData?.items ?? [])].find((i) => i.type.startsWith('image/'))
      if (item) pick(item.getAsFile())
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return (
    <div className="space-y-3">
      <div
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault()
          pick(e.dataTransfer.files[0])
        }}
        className="flex items-center gap-3 rounded-[var(--r)] border-[1.5px] border-dashed border-rule bg-surface px-4 py-5 text-[12.5px] text-muted"
      >
        {preview && <img src={preview} alt="" className="h-16 w-16 rounded-[var(--r-sm)] object-cover" />}
        <div className="grow">
          <b className="block text-[13px] text-ink">{name || '拖一张图进来，或粘贴'}</b>
          jpg / png / webp，≤20MB；只在杂志裁图里找，一次占 GPU 约一秒，正式轮在跑时要排队
          {run.isPending && <span className="ml-2 text-accent">算向量中…</span>}
        </div>
        <button
          type="button"
          onClick={() => input.current?.click()}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px]"
        >
          选文件
        </button>
        <input
          ref={input}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => pick(e.target.files?.[0])}
        />
      </div>
      {run.error && <p className="text-[12.5px] text-warn">{errText(run.error)}</p>}
      {run.data && (
        <>
          <p className="text-[12px] text-muted">
            算向量 {((run.data.embed_ms ?? 0) / 1000).toFixed(1)} 秒 · 共{' '}
            {((run.data.total_ms ?? 0) / 1000).toFixed(1)} 秒 · 回 {run.data.count} 条
          </p>
          <MagGrid items={run.data.items} />
        </>
      )}
    </div>
  )
}
