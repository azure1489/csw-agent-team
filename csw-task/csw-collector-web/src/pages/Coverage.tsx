/**
 * 采集与覆盖：**每个采集器取到多少、图识别了多少、哪些没成。**
 *
 * `unreviewed` 那一列是这一页的重点：工作台每条都判，它**应当是 0**。
 * 不是 0 就说明采集那一步没跑完——如实显示，别把它藏起来凑成 0。
 */
import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Stat, Table, bj } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useCoverage, useRoundMedia, useRounds } from '@/lib/queries'

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
                第 {r.id} 轮 · {bj(r.window_end)}
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
    </section>
  )
}

function num(v: unknown): string {
  return typeof v === 'number' ? String(v) : '——'
}

function resultLabel(r: string): string {
  return { ok: '正常', failed: '失败', partial: '部分' }[r] ?? r
}
