import type { CSSProperties, ReactNode } from 'react'
import { TONES, STATUS, type Tone } from '@/lib/constants'

export function Badge({
  tone = 'gray',
  children,
  dot,
  outline,
  sm,
  style,
}: {
  tone?: Tone
  children?: ReactNode
  dot?: boolean
  outline?: boolean
  sm?: boolean
  style?: CSSProperties
}) {
  const t = TONES[tone] || TONES.gray
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        padding: sm ? '1px 7px' : '2px 9px',
        borderRadius: 999,
        fontSize: sm ? 11 : 12,
        fontWeight: 550,
        lineHeight: 1.55,
        color: t.c,
        background: outline ? 'transparent' : t.bg,
        border: `1px solid ${outline ? 'var(--border-strong)' : t.bd}`,
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {dot && <span style={{ width: 6, height: 6, borderRadius: 999, background: t.c }} />}
      {children}
    </span>
  )
}

/** 由已知 status key 渲染徽标；map 可把业务字段映射到 STATUS 键（如 run status → run_active） */
export function StatusBadge({ status, map, sm }: { status: string; map?: (s: string) => string; sm?: boolean }) {
  const key = map ? map(status) : status
  const s = STATUS[key] || { label: status, tone: 'gray' as Tone }
  return (
    <Badge tone={s.tone} dot={s.dot} outline={s.outline} sm={sm}>
      {s.label}
    </Badge>
  )
}
