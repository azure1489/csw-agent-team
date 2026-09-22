/**
 * 指标：**全是计数，没有一个是分数。**
 *
 * 最该看的是「被捞回的」与「被压下的」——它们是「模型判了什么」
 * 与「人改成了什么」之间的差。差得越多，说明判断框架离 Van 的口味越远，
 * 那是要改框架的信号，不是要改人的。
 */
import { ErrorBox, H2, Head, Loading, Stat } from '@/components/Bits'
import { useSelectionMetrics } from '@/lib/queries'

/** 分组显示：混在一起看不出哪些是「系统判的」、哪些是「人改的」 */
const GROUPS: [string, string[]][] = [
  ['这些轮次判了多少', ['轮次', '判过的条目', '没读到实图']],
  ['四档各多少', ['推荐', '备选', '待核', '不推荐']],
  ['人改了多少', ['人工改档', '被捞回的', '被压下的', 'Van 勾选']],
  ['还挂着的', ['未结待核']],
]

export function Metrics() {
  const q = useSelectionMetrics()
  if (q.isLoading) return <Loading what="指标" />
  if (q.error) return <ErrorBox error={q.error} />
  const m = q.data ?? {}

  return (
    <section>
      <Head
        title="指标"
        desc="全是计数。「模型判了什么」与「人改成了什么」分开算——两者差得越多，说明框架离 Van 的口味越远。"
      />
      {GROUPS.map(([title, keys]) => (
        <div key={title}>
          <H2>{title}</H2>
          <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
            {keys.map((k) => (
              <Stat
                key={k}
                label={k}
                value={m[k] ?? 0}
                tone={
                  k === '没读到实图' && (m[k] ?? 0) > 0
                    ? 'warn'
                    : k === '未结待核' && (m[k] ?? 0) > 0
                      ? 'warn'
                      : undefined
                }
              />
            ))}
          </div>
        </div>
      ))}
      <p className="mt-4 text-[12.5px] text-muted">
        这里**不会**出现采用率、命中率这类比值：分母（Van 当期真正采用了哪几条）
        在引擎那边，等 0036–0040 部署、台账对得上之后再算，现在算出来的是假的。
      </p>
    </section>
  )
}
