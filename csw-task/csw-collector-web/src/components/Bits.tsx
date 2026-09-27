/**
 * 页面里反复出现的小零件。
 *
 * 放在这里而不是各页自己写：**同一个东西在两页长得不一样**，
 * 人会以为是两个东西——台账里的「推荐」和 Van 页里的「推荐」必须是同一个。
 */
import type { ReactNode } from 'react'

import { Badge } from '@/components/ui'
import type { Tone } from '@/lib/constants'
import { STATUS } from '@/lib/constants'

/**
 * 四档徽标。**没有分数色阶**：不推荐是中性灰，它是一个正常结论，不是错误。
 *
 * `awaiting`：备选里还挂着「影响选题判断」的缺口——深核后仍缺关键资料的就落在这里，
 * 显示成「备选·待补证」，免得和资料齐全的备选混在一起。
 */
export function TierBadge({ tier, sm, awaiting }: { tier: string; sm?: boolean; awaiting?: boolean }) {
  const s = STATUS[tier] ?? { label: tier, tone: 'gray' as Tone }
  return (
    <Badge tone={s.tone} dot={s.dot} outline={s.outline} sm={sm}>
      {tier === 'alternate' && awaiting ? '备选·待补证' : s.label}
    </Badge>
  )
}

/** 步骤状态徽标 */
export function StepBadge({ status, sm }: { status: string; sm?: boolean }) {
  return <TierBadge tier={status} sm={sm} />
}

/** 北京时间。**接口一律给 UTC**，换算只在这一处做 */
export function bj(iso?: string | null, withSeconds = false): string {
  if (!iso) return '——'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  const p = (n: number) => String(n).padStart(2, '0')
  const s = `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(
    d.getMinutes(),
  )}`
  return withSeconds ? `${s}:${p(d.getSeconds())}` : s
}

/** 轮次的叫法。**窗口终点对测试轮没有意义**（回测、重判的窗口是占位的），所以一律给开工时间 */
const ROUND_KIND: Record<string, string> = {
  task: '正式',
  prefetch: '预取',
  manual: '手动',
  replay: '回放',
  backtest: '测试',
}
const ROUND_STATUS: Record<string, string> = {
  running: '进行中',
  failed: '失败',
  cancelled: '已取消',
}

export function roundLabel(r: {
  id: number
  kind: string
  run_id: number | null
  status: string
  created_at: string
}): string {
  const kind = ROUND_KIND[r.kind] ?? r.kind
  const run = r.run_id ? ` r${r.run_id}` : ''
  const st = ROUND_STATUS[r.status] ? `（${ROUND_STATUS[r.status]}）` : ''
  return `第 ${r.id} 轮 · ${kind}${run} · ${bj(r.created_at)}${st}`
}

/** 只给日期那一半 */
export function bjDate(iso?: string | null): string {
  return bj(iso).split(' ')[0] ?? '——'
}

export function Loading({ what }: { what?: string }) {
  return <div className="py-8 text-center text-[13px] text-muted">正在取{what ?? '数据'}…</div>
}

/** 出错就把后端那句人话原样显示。**不换成「加载失败」**——那句话是写给人看的 */
export function ErrorBox({ error }: { error: unknown }) {
  const msg =
    (error as { response?: { data?: { error?: string } }; message?: string })?.response?.data
      ?.error ??
    (error as { message?: string })?.message ??
    '出错了'
  return (
    <div className="rounded-[var(--r)] border border-bad-soft bg-bad-soft px-3 py-2 text-[13px] text-bad">
      {msg}
    </div>
  )
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-[var(--r)] border border-dashed border-rule bg-surface px-3 py-6 text-center text-[12.5px] text-muted">
      {children}
    </div>
  )
}

/** 一个数字 + 一行字。总览与指标页共用 */
export function Stat({
  label,
  value,
  note,
  tone,
}: {
  label: string
  value: ReactNode
  note?: ReactNode
  tone?: 'ok' | 'warn' | 'bad'
}) {
  const color = tone ? { ok: 'text-ok', warn: 'text-warn', bad: 'text-bad' }[tone] : 'text-ink'
  return (
    <div className="rounded-[var(--r-md)] border border-rule bg-surface px-3.5 py-3">
      <div className="text-[12px] text-muted">{label}</div>
      <div className={`tnum mt-0.5 text-[22px] font-semibold leading-tight ${color}`}>{value}</div>
      {note && <div className="mt-0.5 text-[11.5px] text-dim">{note}</div>}
    </div>
  )
}

/** 小标题 */
export function H2({ children }: { children: ReactNode }) {
  return <h2 className="mb-2 mt-5 text-[14px] font-semibold">{children}</h2>
}

/** 页头 */
export function Head({
  title,
  desc,
  right,
}: {
  title: string
  desc?: ReactNode
  right?: ReactNode
}) {
  return (
    <div className="mb-4 flex flex-wrap items-start justify-between gap-3">
      <div>
        <h1 className="m-0 text-[18px] font-bold leading-tight">{title}</h1>
        {desc && <p className="mb-0 mt-1 max-w-[52em] text-[12.5px] text-muted">{desc}</p>}
      </div>
      {right && <div className="flex items-center gap-2">{right}</div>}
    </div>
  )
}

/** 一张表。列宽交给浏览器，只保证横向能滚——窄屏上表格宁可滚动也不要挤成一团 */
export function Table({ head, children }: { head: ReactNode; children: ReactNode }) {
  return (
    <div className="overflow-x-auto rounded-[var(--r-md)] border border-rule bg-surface">
      <table className="w-full border-collapse text-[13px]">
        <thead className="bg-surface-2 text-[12px] text-muted">{head}</thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  )
}
