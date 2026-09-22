/**
 * 待核结转：**跨轮还没结清的 `pending_check`。**
 *
 * 这一页只说一件事：**待核不是淘汰。** 它是「证据或图片不足，补齐后重评」，
 * 所以它会一直挂在这里，直到某一轮真的判出了别的档。
 */
import { Empty, ErrorBox, Head, Loading, Table } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { usePendingCheck } from '@/lib/queries'

export function Pending() {
  const q = usePendingCheck()
  return (
    <section>
      <Head
        title="待核结转"
        desc="待核不是淘汰——它是「证据或图片不足，补齐后重评」。挂在这里直到某一轮判出了别的档。"
      />
      {q.isLoading && <Loading what="待核" />}
      {q.error && <ErrorBox error={q.error} />}
      {q.data?.length === 0 && <Empty>没有未结的待核。</Empty>}
      {q.data && q.data.length > 0 && (
        <>
          <p className="mb-2 text-[12.5px] text-muted">{q.data.length} 条</p>
          <Table
            head={
              <tr>
                <Th>条目键</Th>
                <Th>怎么结</Th>
              </tr>
            }
          >
            {q.data.map((k) => (
              <tr key={k} className="border-t border-rule">
                <Td className="mono text-[12.5px]">{k}</Td>
                <Td className="text-[12.5px] text-muted">
                  补齐材料后重跑判断那一步，或在台账里改档并写明理由
                </Td>
              </tr>
            ))}
          </Table>
        </>
      )}
    </section>
  )
}
