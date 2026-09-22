/**
 * 选题记忆：**Van 说过的话，原样存着。**
 *
 * 准则卡是从她历次决定里归纳出来的，**归纳出来的东西不算数**——
 * 要她本人点头才标「已校准」。案例带原话，一字不改：改写过的原话
 * 不能拿去跟她对质。
 */
import { Empty, ErrorBox, H2, Head, Loading, Table, bjDate } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useMemoryCases, useMemoryRules } from '@/lib/queries'

export function Memory() {
  const rules = useMemoryRules()
  const cases = useMemoryCases()

  return (
    <section>
      <Head
        title="选题记忆"
        desc="准则卡是归纳出来的，要她点头才算数；案例带原话，一字不改。"
      />

      <H2>准则卡</H2>
      {rules.isLoading && <Loading what="准则" />}
      {rules.error && <ErrorBox error={rules.error} />}
      {rules.data?.length === 0 && (
        <Empty>还没有准则卡。它们从历次决定里归纳，等 P5 把历史反馈回收进来。</Empty>
      )}
      {rules.data && rules.data.length > 0 && (
        <div className="space-y-2">
          {rules.data.map((r) => (
            <div key={r.rule_key} className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
              <div className="flex items-center gap-2">
                <span className="mono text-[11.5px] text-dim">{r.rule_key}</span>
                {r.confirmed_by_van ? (
                  <span className="rounded-[var(--r-sm)] bg-van-soft px-1.5 py-0.5 text-[11.5px] text-van">
                    Van 已校准
                  </span>
                ) : (
                  <span className="rounded-[var(--r-sm)] border border-rule px-1.5 py-0.5 text-[11.5px] text-muted">
                    归纳出来的，还没点头
                  </span>
                )}
                <span className="ml-auto text-[11.5px] text-dim">{bjDate(r.updated_at)}</span>
              </div>
              <div className="mt-1 text-[13.5px]">{r.text}</div>
            </div>
          ))}
        </div>
      )}

      <H2>案例与原话</H2>
      {cases.isLoading && <Loading what="案例" />}
      {cases.error && <ErrorBox error={cases.error} />}
      {cases.data?.length === 0 && <Empty>还没有案例。</Empty>}
      {cases.data && cases.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>决定</Th>
              <Th>原话</Th>
              <Th>时间</Th>
            </tr>
          }
        >
          {cases.data.map((c) => (
            <tr key={c.case_key} className="border-t border-rule align-top">
              <Td>
                <span
                  className={
                    c.decision === '采用'
                      ? 'text-ok'
                      : c.decision === '否决'
                        ? 'text-bad'
                        : 'text-warn'
                  }
                >
                  {c.decision}
                </span>
              </Td>
              <Td>
                <div className="text-[13px]">「{c.quote}」</div>
                {c.source_url && (
                  <a
                    href={c.source_url}
                    target="_blank"
                    rel="noreferrer"
                    className="text-[12px] text-accent no-underline"
                  >
                    看那一条
                  </a>
                )}
              </Td>
              <Td className="tnum text-[12.5px]">{bjDate(c.decided_at)}</Td>
            </tr>
          ))}
        </Table>
      )}
    </section>
  )
}
