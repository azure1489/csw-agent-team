/**
 * 台账各页共用的小工具：新旧两版字段的兼容读、取图地址。
 *
 * 09-23 起判断输出换成 v2（三句话新键、缺口分级、查重 hits）；之前的轮次还是 v1。
 * 页面不该因为翻到一轮旧台账就显示一片空白，所以读的时候两版都认。
 */
import type { Gap, ThreeSentences } from './types'

/** 本地存的实图。要登录才取得到，同源请求自带会话 cookie */
export const mediaUrl = (hash: string) => `/api/media/${hash}`

/** 旧行的缺口是纯字符串：按「影响判断、收集员处理」读（与后端兜底一致） */
export function normGaps(gaps: (Gap | string)[] | undefined | null): Gap[] {
  return (gaps ?? []).map((g) =>
    typeof g === 'string' ? { level: 'decision', what: g, owner: 'collector', tried: '', next: '' } : g,
  )
}

export function three(t: ThreeSentences | undefined | null) {
  return {
    what: t?.what ?? t?.what_changed ?? '',
    why: t?.why_worth ?? t?.why_it_matters ?? '',
    grounds: t?.grounds ?? t?.how_different ?? '',
  }
}

/** 列表主信息：标题优先，没有就用「是什么」那句 */
export function titleOf(headline: string | undefined, t: ThreeSentences | undefined | null): string {
  return headline?.trim() || three(t).what
}
