/**
 * 采集与覆盖：**每个采集器取到多少、图识别了多少、哪些没成。**
 *
 * `unreviewed` 那一列是这一页的重点：工作台每条都判，它**应当是 0**。
 * 不是 0 就说明采集那一步没跑完——如实显示，别把它藏起来凑成 0。
 */
import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Stat, Table, bj, bjDate, roundLabel } from '@/components/Bits'
import { I } from '@/components/icons'
import { Btn, Td, Th } from '@/components/ui'
import { useBackfill, useCoverage, useRoundMedia, useRounds } from '@/lib/queries'
import type { BackfillRow } from '@/lib/types'

export function Coverage() {
  const [sp, setSp] = useSearchParams()
  const rounds = useRounds({ limit: 20 })
  const roundId = Number(sp.get('round')) || rounds.data?.find((r) => r.kind === 'task')?.id
  const cov = useCoverage(roundId)
  const [onlyFailed, setOnlyFailed] = useState(true)
  const media = useRoundMedia(roundId, onlyFailed ? 'failed' : undefined)

  const h = cov.data?.harvest ?? {}
  const secs = (ms?: number) => (ms ? `${Math.round(ms / 1000)} 秒` : '——')

  return (
    <section>
      <Head
        title="采集与覆盖"
        desc="计数由代码算，不由采集器自报——让实现方自己填等于自己给自己打分。"
        right={
          <select
            value={roundId ?? ''}
            onChange={(e) => setSp({ round: e.target.value })}
            className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
          >
            {(rounds.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                {roundLabel(r)}
              </option>
            ))}
          </select>
        }
      />

      {cov.isLoading && <Loading what="覆盖" />}
      {cov.error && <ErrorBox error={cov.error} />}

      {cov.data && (
        <>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            <Stat label="候选" value={num(h['候选'])} />
            <Stat
              label="读到实图"
              value={num(h['读到实图'])}
              note={h['候选'] ? `共 ${h['候选']} 条` : undefined}
            />
            <Stat label="复用识别" value={num(h['复用识别'])} note="预取轮省下来的" />
            <Stat label="新落库描述" value={num(h['新落库描述'])} />
          </div>
          <div className="mt-3 grid grid-cols-2 gap-3 md:grid-cols-4">
            <Stat label="下载" value={secs(h['下载毫秒'])} />
            <Stat label="识别与向量墙钟" value={secs(h['识别向量墙钟毫秒'])} />
            <Stat label="网关占用" value={secs(h['网关占用毫秒'])} note="是占用不是墙钟" />
            <Stat label="GPU 占用" value={secs(h['GPU占用毫秒'])} note="全局串行" />
          </div>

          <H2>各采集器</H2>
          {cov.data.sweeps.length === 0 ? (
            <Empty>这一轮还没把采集轮账发出去。</Empty>
          ) : (
            <Table
              head={
                <tr>
                  <Th>采集器</Th>
                  <Th right>接口返回</Th>
                  <Th right>去重后</Th>
                  <Th right>窗口内</Th>
                  <Th right>已审阅</Th>
                  <Th right>未审阅</Th>
                  <Th>结果</Th>
                </tr>
              }
            >
              {cov.data.sweeps.map((s) => (
                <tr key={s.sweep_key} className="border-t border-rule">
                  <Td>
                    <div className="text-[13px]">{s.sweep_key}</div>
                    <div className="text-[11.5px] text-muted">
                      {s.platform}
                      {s.source_key && ` · ${s.source_key}`}
                    </div>
                  </Td>
                  <Td right className="tnum">
                    {s.found}
                  </Td>
                  <Td right className="tnum">
                    {s.fetched_unique}
                  </Td>
                  <Td right className="tnum">
                    {s.in_window}
                  </Td>
                  <Td right className="tnum">
                    {s.reviewed}
                  </Td>
                  <Td right className="tnum" style={{ color: s.unreviewed > 0 ? 'var(--bad)' : undefined }}>
                    {s.unreviewed}
                  </Td>
                  <Td>
                    <span
                      className={
                        s.result === 'ok'
                          ? 'text-ok'
                          : s.result === 'failed'
                            ? 'text-bad'
                            : 'text-warn'
                      }
                    >
                      {resultLabel(s.result)}
                    </span>
                    {s.error && <div className="text-[11.5px] text-muted">{s.error}</div>}
                    {!s.paged_to_end && s.result === 'ok' && (
                      <div className="text-[11.5px] text-warn">没翻到底</div>
                    )}
                  </Td>
                </tr>
              ))}
            </Table>
          )}
          {cov.data.sweeps.some((s) => s.unreviewed > 0) && (
            <p className="mt-2 text-[12.5px] text-bad">
              「未审阅」不是 0。工作台每条都判，不是 0 就说明采集那一步没跑完——
              自查会因此报红，这是对的。
            </p>
          )}
        </>
      )}

      <H2>图</H2>
      <label className="mb-2 flex items-center gap-1.5 text-[12.5px] text-muted">
        <input
          type="checkbox"
          checked={onlyFailed}
          onChange={(e) => setOnlyFailed(e.target.checked)}
        />
        只看没下到或没识别成的
      </label>
      {media.isLoading && <Loading what="图" />}
      {media.data?.length === 0 && (
        <Empty>{onlyFailed ? '这一轮的图都处理成功了。' : '这一轮还没有图。'}</Empty>
      )}
      {media.data && media.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>序</Th>
              <Th>地址</Th>
              <Th>类型</Th>
              <Th>画面</Th>
            </tr>
          }
        >
          {media.data.slice(0, 200).map((m) => (
            <tr key={m.blake3 + m.ordinal} className="border-t border-rule">
              <Td className="tnum">{m.ordinal}</Td>
              <Td>
                <a href={m.url} target="_blank" rel="noreferrer" className="text-accent no-underline">
                  {m.url.slice(0, 60)}…
                </a>
              </Td>
              <Td>{m.failed ? <span className="text-bad">没识别成</span> : m.kind || '——'}</Td>
              <Td className="text-[12.5px] text-muted">{m.content || '——'}</Td>
            </tr>
          ))}
        </Table>
      )}

      <BackfillSection />
    </section>
  )
}

const KIND_LABEL: Record<string, string> = {
  decision: '决定',
  published_item: '已发',
  example: '范例',
}

const CONCLUSION_LABEL: Record<string, string> = {
  written: '已写',
  approved_write: '批准写',
  published: '已发',
  dropped: '淘汰',
  rejected: '否决',
  pending_check: '待核',
}

/**
 * 待补录清单：**出现在选题里、却不在 csw 在册名单里的品牌。**
 * 不跟轮次走——它对的是整个知识库。补录由人去 csw 后台做，这里只列、只给依据。
 */
function BackfillSection() {
  const q = useBackfill()
  const rows = q.data?.rows ?? []
  return (
    <>
      <div className="mb-2 mt-5 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-[14px] font-semibold">待补录品牌</h2>
        {rows.length > 0 && (
          <Btn size="sm" icon={I.download} onClick={() => downloadCsv(rows)}>
            导出 CSV
          </Btn>
        )}
      </div>
      <p className="mb-2 text-[12.5px] text-muted">
        进过选题（决定、已发、范例）却对不上 csw 在册账号的品牌。
        「被选中」= 写过、批准写、已发或范例；只被淘汰过的也列出来，排在后面。
        {q.data && ` 对照的在册别名 ${q.data.registered_brands} 条。`}
      </p>
      {q.isLoading && <Loading what="待补录清单" />}
      {q.error && <ErrorBox error={q.error} />}
      {q.data && rows.length === 0 && <Empty>进过选题的品牌都在册。</Empty>}
      {rows.length > 0 && (
        <Table
          head={
            <tr>
              <Th>品牌</Th>
              <Th right>被选中</Th>
              <Th right>出现</Th>
              <Th>最近</Th>
              <Th>依据</Th>
            </tr>
          }
        >
          {rows.map((r) => (
            <tr key={r.brand} className="border-t border-rule align-top">
              <Td className="font-medium">{r.brand}</Td>
              <Td right className="tnum" style={{ color: r.adopted > 0 ? 'var(--accent)' : undefined }}>
                {r.adopted}
              </Td>
              <Td right className="tnum">
                {r.mentions}
              </Td>
              <Td className="tnum whitespace-nowrap">{bjDate(r.latest)}</Td>
              <Td>
                <ul className="space-y-0.5 text-[12.5px]">
                  {r.evidence.map((e, i) => (
                    <li key={i}>
                      <span className="text-muted">
                        {KIND_LABEL[e.kind] ?? e.kind}
                        {e.conclusion && `·${CONCLUSION_LABEL[e.conclusion] ?? e.conclusion}`}
                      </span>{' '}
                      {e.url ? (
                        <a href={e.url} target="_blank" rel="noreferrer" className="text-accent no-underline">
                          {e.title || e.url}
                        </a>
                      ) : (
                        e.title
                      )}
                    </li>
                  ))}
                </ul>
              </Td>
            </tr>
          ))}
        </Table>
      )}
    </>
  )
}

/** 前端拼 CSV：带 BOM，Excel 打开中文不乱码。 */
function downloadCsv(rows: BackfillRow[]) {
  const cell = (v: string | number | null) => {
    const s = v == null ? '' : String(v)
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
  }
  const lines = [
    ['品牌', '被选中', '出现', '最近', '依据标题', '依据链接'].join(','),
    ...rows.map((r) =>
      [
        r.brand,
        r.adopted,
        r.mentions,
        r.latest?.slice(0, 10) ?? '',
        r.evidence.map((e) => e.title).join(' / '),
        r.evidence.map((e) => e.url).join(' '),
      ]
        .map(cell)
        .join(','),
    ),
  ]
  const blob = new Blob(['\ufeff' + lines.join('\n')], { type: 'text/csv;charset=utf-8' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = `待补录品牌_${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  // 立刻回收有的浏览器会把下载掐掉
  setTimeout(() => URL.revokeObjectURL(a.href), 1000)
}

function num(v: unknown): string {
  return typeof v === 'number' ? String(v) : '——'
}

function resultLabel(r: string): string {
  return { ok: '正常', failed: '失败', partial: '部分' }[r] ?? r
}
