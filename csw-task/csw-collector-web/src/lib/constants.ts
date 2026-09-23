/**
 * 色调与状态映射。
 *
 * `components/ui/*` 从 csw-task-web 原样复制，依赖这两个导出。
 * 颜色一律走 CSS 变量，跟着主题翻——不在这里写死十六进制。
 *
 * 与那边不同的是 `STATUS`：这里的状态是**判断结论档**与**轮次步骤**，
 * 不是任务流转的那一套。
 */
export type Tone = 'green' | 'blue' | 'amber' | 'red' | 'gray' | 'yellow' | 'violet'

export const TONES: Record<Tone, { c: string; bg: string; bd: string }> = {
  green: { c: 'var(--ok)', bg: 'var(--ok-soft)', bd: 'var(--ok-soft)' },
  blue: { c: 'var(--accent)', bg: 'var(--accent-soft)', bd: 'var(--accent-soft)' },
  amber: { c: 'var(--warn)', bg: 'var(--warn-soft)', bd: 'var(--warn-soft)' },
  red: { c: 'var(--bad)', bg: 'var(--bad-soft)', bd: 'var(--bad-soft)' },
  gray: { c: 'var(--muted)', bg: 'var(--surface-2)', bd: 'var(--rule)' },
  yellow: { c: 'var(--warn)', bg: 'var(--warn-soft)', bd: 'var(--warn-soft)' },
  violet: { c: 'var(--van)', bg: 'var(--van-soft)', bd: 'var(--van-soft)' },
}

export interface StatusMeta {
  label: string
  tone: Tone
  dot?: boolean
  outline?: boolean
}

/**
 * 四档结论 + 轮次步骤状态。
 *
 * 四档刻意不给「分数色阶」——不是绿黄橙红的温度计，
 * 「不推荐」用中性灰而不是红：它是一个正常结论，不是错误。
 */
export const STATUS: Record<string, StatusMeta> = {
  recommend: { label: '推荐', tone: 'green', dot: true },
  alternate: { label: '备选', tone: 'blue', dot: true },
  pending_check: { label: '待核', tone: 'amber', dot: true },
  not_recommend: { label: '不推荐', tone: 'gray' },

  // 六维
  yes: { label: '成立', tone: 'green' },
  unclear: { label: '不明', tone: 'gray', outline: true },
  no: { label: '不成立', tone: 'gray' },

  // 轮次步骤
  pending: { label: '待跑', tone: 'gray', outline: true },
  running: { label: '进行中', tone: 'blue', dot: true },
  succeeded: { label: '完成', tone: 'green' },
  partial: { label: '部分完成', tone: 'amber' },
  failed: { label: '失败', tone: 'red', dot: true },
  skipped: { label: '跳过', tone: 'gray', outline: true },
  stale: { label: '已作废', tone: 'gray', outline: true },
  interrupted: { label: '被打断', tone: 'amber', outline: true },
}

/**
 * 六维（v2，Van 09-23 认可的定义）。键名不变，含义换了。
 * `short` 用在台账的小格上，`full` 用在依据表里。
 */
export const DIMS: Record<string, { short: string; full: string }> = {
  change: { short: '点', full: '具体看点' },
  use: { short: '者', full: '与读者有关' },
  gain: { short: '值', full: '值得推荐的价值（核心）' },
  compare: { short: '景', full: '差异与背景' },
  explain: { short: '角', full: '报道角度与依据' },
  csw: { short: 'C', full: 'CSW 适配度' },
}

export const TIER_LABEL: Record<string, string> = {
  recommend: '推荐',
  alternate: '备选',
  pending_check: '待核',
  not_recommend: '不推荐',
}

/** 查重结论。**未确认不是无重复**：历史正文缺失或只有生成稿时就是它 */
export const DEDUP: Record<string, { label: string; tone: Tone }> = {
  unrelated: { label: '无重复', tone: 'gray' },
  same_brand_with_gain: { label: '同品牌有增量', tone: 'blue' },
  same_fact_no_gain: { label: '同事实无增量', tone: 'red' },
  unconfirmed: { label: '查重未确认', tone: 'amber' },
}

export const HIT_STATE: Record<string, string> = {
  published: '正式发布',
  draft: '已推草稿箱',
  generated: '仅生成稿',
  decision: '03 决定',
  unknown: '状态不详',
}

export const GAP_LEVEL: Record<string, { label: string; tone: Tone }> = {
  decision: { label: '影响判断', tone: 'amber' },
  production: { label: '影响成稿', tone: 'blue' },
  boundary: { label: '表达边界', tone: 'gray' },
}

export const OWNER: Record<string, string> = {
  collector: '收集员',
  editor: '主编',
  van: 'Van',
}

export const NOVELTY: Record<string, string> = {
  existing_feature: '产品现有特点',
  evidenced_change: '有证据的新变化',
  explainable_design: '值得解释的设计或文化内容',
}

export const FACT_SOURCE: Record<string, string> = {
  primary: '原始发布',
  reshared: '转载',
  brand_claim_only: '仅品牌自述',
  unknown: '不详',
}
