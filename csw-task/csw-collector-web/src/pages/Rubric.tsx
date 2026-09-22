/**
 * 判断框架与锚点：**Van 的那把尺子长什么样。**
 *
 * 从代码里的常量来，不是从库里——它要跟着提示词一起走，
 * 改动经 `RUBRIC_VERSION` 进版本。这一页是给人核对「系统是不是按她的口径判的」，
 * 所以**一个字都不改写**。
 */
import { Empty, ErrorBox, H2, Head, Loading } from '@/components/Bits'
import { useRubric } from '@/lib/queries'

const DIM_CN: Record<string, string> = {
  change: '变化必须具体',
  use: '与真实使用有关',
  gain: '有 CSW 读者能理解的价值',
  compare: '最好有比较参照',
  explain: '能产生有事实支持的编辑判断',
  csw: 'CSW 视角（调性适配）',
}

export function Rubric() {
  const q = useRubric()
  if (q.isLoading) return <Loading what="判断框架" />
  if (q.error) return <ErrorBox error={q.error} />
  const r = q.data
  if (!r) return <Empty>没拿到框架。</Empty>

  return (
    <section>
      <Head title="判断框架与锚点" desc={`版本 ${r.version}。改动跟着提示词一起走。`} />

      <div className="rounded-[var(--r-md)] border border-accent-soft bg-accent-soft px-4 py-3">
        <div className="text-[12px] text-muted">核心问题</div>
        <div className="mt-1 text-[15px] font-semibold text-accent">{r.core_question}</div>
      </div>

      <H2>三问</H2>
      <ol className="m-0 space-y-1 pl-5 text-[13.5px]">
        {r.three_questions.map((x, i) => (
          <li key={i}>{x}</li>
        ))}
      </ol>

      <H2>六维与锚点</H2>
      <div className="grid gap-2 md:grid-cols-2">
        {r.dim_anchors.map((d) => (
          <div key={d.dim} className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
            <div className="text-[13px] font-semibold">{DIM_CN[d.dim] ?? d.dim}</div>
            <div className="mt-0.5 text-[12.5px] leading-relaxed text-muted">{d.anchor}</div>
          </div>
        ))}
      </div>

      <div className="mt-4 grid gap-4 md:grid-cols-2">
        <div>
          <H2>七类优先关注</H2>
          <ul className="m-0 space-y-0.5 pl-5 text-[13px]">
            {r.priority.map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </div>
        <div>
          <H2>七类降低优先级</H2>
          <ul className="m-0 space-y-0.5 pl-5 text-[13px]">
            {r.lower.map((x, i) => (
              <li key={i}>{x}</li>
            ))}
          </ul>
        </div>
      </div>
      <p className="mt-2 text-[12.5px] text-muted">
        这两类是<b>倾向，不是黑名单</b>：命中「降低优先级」的条目照样判、照样在台账里。
      </p>

      <H2>不是维度的四样</H2>
      <div className="flex flex-wrap gap-2">
        {r.not_dimensions.map((x) => (
          <span
            key={x}
            className="rounded-[var(--r-sm)] border border-rule bg-surface-2 px-2.5 py-1 text-[12.5px] text-muted line-through"
          >
            {x}
          </span>
        ))}
      </div>
      <p className="mt-2 text-[12.5px] text-muted">
        它们最容易被偷偷用上——点赞数高的看着就像值得写，但那不是 Van 的判据。
      </p>
    </section>
  )
}
