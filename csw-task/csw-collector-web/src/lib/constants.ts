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
