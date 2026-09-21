import { useState, type CSSProperties, type ReactNode } from 'react'
import type { IconComponent } from '@/components/icons'

export type BtnVariant = 'primary' | 'default' | 'ghost' | 'danger' | 'danger-solid'
export type BtnSize = 'sm' | 'md' | 'lg'

const var_r = (s: BtnSize) => (s === 'lg' ? '9px' : '7px')

export function Btn({
  variant = 'default',
  size = 'md',
  icon: IconC,
  iconRight: IconR,
  children,
  onClick,
  disabled,
  type,
  style,
  title,
  full,
}: {
  variant?: BtnVariant
  size?: BtnSize
  icon?: IconComponent
  iconRight?: IconComponent
  children?: ReactNode
  onClick?: () => void
  disabled?: boolean
  type?: 'button' | 'submit'
  style?: CSSProperties
  title?: string
  full?: boolean
}) {
  const [hover, setHover] = useState(false)
  const [press, setPress] = useState(false)
  const padv = size === 'sm' ? '5px 10px' : size === 'lg' ? '9px 16px' : '6.5px 13px'
  const fs = size === 'sm' ? 12.5 : 14
  const variants: Record<BtnVariant, { bg: string; color: string; border: string; shadow: string }> = {
    primary: {
      bg: disabled ? '#b9b9e8' : press ? 'var(--accent-press)' : hover ? 'var(--accent-hover)' : 'var(--accent)',
      color: '#fff',
      border: 'transparent',
      shadow: disabled ? 'none' : '0 1px 2px rgba(60,60,140,.25), inset 0 1px 0 rgba(255,255,255,.12)',
    },
    default: {
      bg: press ? 'var(--surface-3)' : hover ? 'var(--surface-2)' : 'var(--surface)',
      color: 'var(--text)',
      border: 'var(--border-strong)',
      shadow: 'var(--shadow-sm)',
    },
    ghost: {
      bg: hover ? 'var(--surface-2)' : 'transparent',
      color: 'var(--text-2)',
      border: 'transparent',
      shadow: 'none',
    },
    danger: {
      bg: press ? '#c12f2f' : hover ? '#cf3636' : 'var(--surface)',
      color: hover ? '#fff' : 'var(--red)',
      border: hover ? 'transparent' : 'var(--red-bd)',
      shadow: 'var(--shadow-sm)',
    },
    'danger-solid': {
      bg: press ? '#c12f2f' : hover ? '#cf3636' : 'var(--red)',
      color: '#fff',
      border: 'transparent',
      shadow: '0 1px 2px rgba(170,40,40,.25)',
    },
  }
  const v = variants[variant] || variants.default
  return (
    <button
      type={type || 'button'}
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      title={title}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => {
        setHover(false)
        setPress(false)
      }}
      onMouseDown={() => setPress(true)}
      onMouseUp={() => setPress(false)}
      style={{
        display: full ? 'flex' : 'inline-flex',
        width: full ? '100%' : undefined,
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
        padding: padv,
        fontSize: fs,
        fontWeight: 550,
        borderRadius: var_r(size),
        background: v.bg,
        color: v.color,
        border: `1px solid ${v.border}`,
        boxShadow: v.shadow,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.65 : 1,
        transition: 'background .13s, box-shadow .13s, color .1s',
        whiteSpace: 'nowrap',
        ...style,
      }}
    >
      {IconC && <IconC size={size === 'sm' ? 14 : 16} />}
      {children}
      {IconR && <IconR size={size === 'sm' ? 14 : 16} />}
    </button>
  )
}

export function IconBtn({
  icon: IconC,
  onClick,
  title,
  active,
  size = 18,
  danger,
  style,
}: {
  icon: IconComponent
  onClick?: () => void
  title?: string
  active?: boolean
  size?: number
  danger?: boolean
  style?: CSSProperties
}) {
  const [hover, setHover] = useState(false)
  return (
    <button
      onClick={onClick}
      title={title}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 30,
        height: 30,
        borderRadius: 7,
        border: '1px solid transparent',
        background: active ? 'var(--surface-3)' : hover ? 'var(--surface-2)' : 'transparent',
        color: danger && hover ? 'var(--red)' : active ? 'var(--text)' : 'var(--text-2)',
        transition: 'background .12s, color .12s',
        ...style,
      }}
    >
      <IconC size={size} />
    </button>
  )
}
