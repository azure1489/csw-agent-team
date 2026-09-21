import { useState, type CSSProperties, type ReactNode } from 'react'
import type { IconComponent } from '@/components/icons'

export function Card({
  children,
  style,
  pad = 0,
  hover,
  onClick,
  noPad,
}: {
  children?: ReactNode
  style?: CSSProperties
  pad?: number
  hover?: boolean
  onClick?: () => void
  noPad?: boolean
}) {
  void noPad // Card 默认无内边距（pad=0）；noPad 仅为与 SectionCard 语义一致而接受
  const [h, setH] = useState(false)
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setH(true)}
      onMouseLeave={() => setH(false)}
      style={{
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--r-md)',
        boxShadow: hover && h ? 'var(--shadow)' : 'var(--shadow-sm)',
        padding: pad,
        transition: 'box-shadow .15s, border-color .15s, transform .15s',
        cursor: onClick ? 'pointer' : undefined,
        borderColor: hover && h ? 'var(--border-2)' : 'var(--border)',
        transform: hover && h && onClick ? 'translateY(-1px)' : 'none',
        ...style,
      }}
    >
      {children}
    </div>
  )
}

export function SectionCard({
  title,
  subtitle,
  actions,
  children,
  bodyStyle,
  headStyle,
  noPad,
}: {
  title?: ReactNode
  subtitle?: ReactNode
  actions?: ReactNode
  children?: ReactNode
  bodyStyle?: CSSProperties
  headStyle?: CSSProperties
  noPad?: boolean
}) {
  return (
    <Card>
      {(title || actions) && (
        <div className="row between" style={{ padding: '13px 16px', borderBottom: '1px solid var(--border)', ...headStyle }}>
          <div className="col" style={{ gap: 1 }}>
            {title && <div style={{ fontWeight: 600, fontSize: 14 }}>{title}</div>}
            {subtitle && <div className="t2 sm">{subtitle}</div>}
          </div>
          {actions && <div className="row gap8">{actions}</div>}
        </div>
      )}
      <div style={{ padding: noPad ? 0 : 16, ...bodyStyle }}>{children}</div>
    </Card>
  )
}

export function PageHeader({
  title,
  desc,
  actions,
  breadcrumb,
  badge,
}: {
  title: ReactNode
  desc?: ReactNode
  actions?: ReactNode
  breadcrumb?: ReactNode
  badge?: ReactNode
}) {
  return (
    <div className="row between wrap" style={{ gap: 16, marginBottom: 20 }}>
      <div className="col" style={{ gap: 4 }}>
        {breadcrumb}
        <div className="row gap12" style={{ alignItems: 'center' }}>
          <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650, letterSpacing: '-.01em' }}>{title}</h1>
          {badge}
        </div>
        {desc && (
          <div className="t2" style={{ fontSize: 13.5, maxWidth: 720 }}>
            {desc}
          </div>
        )}
      </div>
      {actions && (
        <div className="row gap8" style={{ flexShrink: 0 }}>
          {actions}
        </div>
      )}
    </div>
  )
}

export function EmptyState({
  icon: IconC,
  title,
  desc,
  action,
  compact,
}: {
  icon?: IconComponent
  title: ReactNode
  desc?: ReactNode
  action?: ReactNode
  compact?: boolean
}) {
  return (
    <div className="col center" style={{ alignItems: 'center', textAlign: 'center', padding: compact ? '32px 20px' : '56px 20px', gap: 4 }}>
      <div
        style={{
          width: 52,
          height: 52,
          borderRadius: 14,
          background: 'var(--surface-2)',
          border: '1px solid var(--border)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--text-3)',
          marginBottom: 8,
        }}
      >
        {IconC && <IconC size={24} />}
      </div>
      <div style={{ fontWeight: 600, fontSize: 14.5 }}>{title}</div>
      {desc && (
        <div className="t2 sm" style={{ maxWidth: 320, lineHeight: 1.55 }}>
          {desc}
        </div>
      )}
      {action && <div style={{ marginTop: 12 }}>{action}</div>}
    </div>
  )
}
