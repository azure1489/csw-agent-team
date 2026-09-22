/**
 * 知识库：**同步到哪天了、库里有什么、搜得到什么。**
 *
 * 水位排在最前面：它回答的是「今天的判断有没有拿到昨天的已发条目」——
 * 「库里有两万条」好看，但回答不了这个问题。
 */
import { useState } from 'react'

import { Empty, ErrorBox, H2, Head, Loading, Stat, Table, bj } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useKbBrand, useKbSearch, useKbStatus } from '@/lib/queries'
import type { KbDocRow } from '@/lib/types'

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
}

export function Kb() {
  const status = useKbStatus()
  const [q, setQ] = useState('')
  const [submitted, setSubmitted] = useState('')
  const [brand, setBrand] = useState('')
  const search = useKbSearch(submitted)
  const byBrand = useKbBrand(brand)

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
              </span>
            ))}
          </div>
        </>
      )}

      <H2>搜一下</H2>
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
      <p className="mb-3 text-[12px] text-muted">
        三路召回：向量 ∪ 品牌命中 ∪ 全文。<b>这条路上不重排</b>——重排占 GPU，
        而 GPU 是全进程串行的，连搜几下会把正式轮的向量化堵住。
      </p>
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

      {brand && (
        <>
          <H2>{brand} 的历史覆盖</H2>
          {byBrand.isFetching && <Loading what="历史" />}
          {byBrand.data && <DocList docs={byBrand.data.docs} onBrand={setBrand} />}
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
