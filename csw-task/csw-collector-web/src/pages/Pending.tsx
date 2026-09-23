/**
 * 待核结转：**跨轮还没结清的 `pending_check`。**
 *
 * 待核不是淘汰，是「影响判断的关键资料还没取得，补齐后重评」。
 * 所以每条都要说清缺什么、由谁处理、已经试过什么、下一步怎么补（09-22 反馈第八项）——
 * 只挂一句「补齐后重跑」，最后还是要 Van 来安排和催促。
 */
import { Link } from 'react-router-dom'

import { Empty, ErrorBox, Head, Loading, Table, bj } from '@/components/Bits'
import { GapList, Thumb } from '@/components/Judged'
import { Td, Th } from '@/components/ui'
import { OWNER } from '@/lib/constants'
import { usePendingCheck } from '@/lib/queries'

const REFETCH_CN: Record<string, string> = {
  ok: '取到',
  blocked: '地址不安全，未访问',
  http_error: '对方返回错误',
  too_large: '页面过大',
  bad_type: '不是网页',
  failed: '访问失败',
}

export function Pending() {
  const q = usePendingCheck()
  const rows = q.data ?? []
  const byOwner = (o: string) => rows.filter((r) => r.gaps.some((g) => g.owner === o)).length
  return (
    <section>
      <Head
        title="待核结转"
        desc="待核不是淘汰——是影响判断的关键资料还没取得。每条写清缺什么、谁处理、试过什么、下一步。"
      />
      {q.isLoading && <Loading what="待核" />}
      {q.error && <ErrorBox error={q.error} />}
      {q.data?.length === 0 && <Empty>没有未结的待核。</Empty>}
      {rows.length > 0 && (
        <>
          <p className="mb-2 text-[12.5px] text-muted">
            {rows.length} 条 · 等收集员 {byOwner('collector')} 条 · 等{OWNER.editor} {byOwner('editor')} 条 ·
            等 Van {byOwner('van')} 条
          </p>
          <div className="overflow-x-auto">
            <Table
              head={
                <tr>
                  <Th>图</Th>
                  <Th>对象</Th>
                  <Th>缺什么 · 谁处理 · 试过什么 · 下一步</Th>
                  <Th>挂了几轮</Th>
                </tr>
              }
            >
              {rows.map((r) => (
                <tr key={r.candidate_key} className="border-t border-rule align-top">
                  <Td>
                    <Thumb hash={r.cover} size={56} />
                  </Td>
                  <Td>
                    <Link
                      to={`/ledger/${r.round_id}/${encodeURIComponent(r.candidate_key)}`}
                      className="text-[13px] font-medium text-accent no-underline"
                    >
                      {r.headline || r.candidate_key}
                    </Link>
                    <div className="text-[11.5px] text-muted">
                      {r.account} ·{' '}
                      <a href={r.url} target="_blank" rel="noreferrer" className="text-accent no-underline">
                        原贴
                      </a>
                    </div>
                  </Td>
                  <Td>
                    <GapList gaps={r.gaps} empty="没写影响判断的缺口——请在台账里补上或改档。" />
                    {r.refetches.length > 0 && (
                      <div className="mt-1 text-[12px] text-muted">
                        补读：{r.refetches.map(([u, s]) => `${u}（${REFETCH_CN[s] ?? s}）`).join('；')}
                      </div>
                    )}
                  </Td>
                  <Td className="tnum whitespace-nowrap text-[12.5px]">
                    {r.rounds_pending} 轮
                    <div className="text-[11.5px] text-muted">自 {bj(r.first_pending_at)}</div>
                  </Td>
                </tr>
              ))}
            </Table>
          </div>
        </>
      )}
    </section>
  )
}
