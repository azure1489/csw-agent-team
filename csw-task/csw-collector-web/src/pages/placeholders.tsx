/**
 * 十一页的占位。每页在对应阶段接入真实接口时替换掉——
 * 这里写清「这页该回答什么问题」，免得接入时跑偏。
 */
function Page({ title, question, stage }: { title: string; question: string; stage: string }) {
  return (
    <section>
      <h1 className="m-0 text-[17px] font-bold">{title}</h1>
      <p className="mb-4 mt-1 max-w-[46em] text-[13px] text-muted">{question}</p>
      <p className="rounded-[var(--r)] border border-dashed border-rule bg-surface px-3 py-2 text-[12.5px] text-dim">
        接口在{stage}接入。
      </p>
    </section>
  )
}

export const Overview = () => (
  <Page
    title="今日总览"
    question="本期跑到哪一步了、有没有卡住、首批还有多久、哪些要人看。流程带十格各显示状态与计数，点开看产物、失败清单与重跑。"
    stage="阶段 6"
  />
)

export const Ledger = () => (
  <Page
    title="判断台账"
    question="窗口内每条候选判了什么、凭什么判。按档分组，六维各带依据，能改档、能捞回、首批留痕。没有分数栏，也不会有。"
    stage="阶段 4"
  />
)

export const Coverage = () => (
  <Page
    title="采集与覆盖"
    question="每个采集器取到多少、只图文多少、图识别了多少、耗时多久、结果如何；哪些来源没扫、哪些账号还没登记。"
    stage="阶段 2B"
  />
)

export const Pending = () => (
  <Page
    title="待核结转"
    question="因为缺材料或没读到实图而悬着的条目。待核不是淘汰——补齐后要能重评。"
    stage="阶段 4"
  />
)

export const Kb = () => (
  <Page
    title="知识库"
    question="已发条目、范例、生成过文章的贴文、03 决定，四类各同步到什么时候、有多少条、能不能搜到。"
    stage="阶段 3"
  />
)

export const Memory = () => (
  <Page
    title="选题记忆"
    question="准则卡与案例库。案例只存 Van 的原话；归纳放准则卡，且必须标明是否经她确认过。"
    stage="阶段 8"
  />
)

export const Rubric = () => (
  <Page
    title="判断框架与锚点"
    question="当前准则版本、六个维度的定义与锚点、回测结果。改框架要 Van 点头。"
    stage="阶段 4"
  />
)

export const Metrics = () => (
  <Page
    title="指标"
    question="采用率、首批命中、待核结转、审阅覆盖。数由代码算，不自我声明。"
    stage="阶段 8"
  />
)

export const Settings = () => (
  <Page
    title="运行与设置"
    question="采集方案与版本、各项开关、服务状态（引擎 / csw / 向量 / 网关 / codex）、手动开启、回放。密钥只显示「已配置」或「缺」，不显示值。"
    stage="阶段 6"
  />
)

export const Van = () => (
  <Page
    title="Van 模式"
    question="当期推荐与备选，每条三句话加代表图，手机上能一路划完。勾选只写本地，不回写引擎——进不进评选由主编代录。"
    stage="阶段 7"
  />
)
