import type { CSSProperties, ReactNode } from 'react'
import * as Dialog from '@radix-ui/react-dialog'
import * as DropdownMenu from '@radix-ui/react-dropdown-menu'
import { I, type IconComponent } from '@/components/icons'
import { TONES, type Tone } from '@/lib/constants'
import { Btn } from './button'
import { IconBtn } from './button'

const srOnly: CSSProperties = {
  position: 'absolute',
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: 'hidden',
  clip: 'rect(0,0,0,0)',
  whiteSpace: 'nowrap',
  border: 0,
}

const overlayStyle: CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(20,20,32,.42)',
  backdropFilter: 'blur(1.5px)',
  zIndex: 1000,
  animation: 'fadeIn .15s',
}

// ---------- Modal（Radix Dialog 底座：焦点陷阱 / ESC / 点击遮罩关闭） ----------
export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  width = 480,
  icon: IconC,
  iconTone,
}: {
  open: boolean
  onClose?: () => void
  title?: ReactNode
  subtitle?: ReactNode
  children?: ReactNode
  footer?: ReactNode
  width?: number
  icon?: IconComponent
  iconTone?: Tone
}) {
  const it = TONES[iconTone || 'violet'] || TONES.violet
  return (
    <Dialog.Root open={open} onOpenChange={(o) => { if (!o) onClose?.() }}>
      <Dialog.Portal>
        <Dialog.Overlay style={overlayStyle} />
        <Dialog.Content
          aria-describedby={undefined}
          style={{
            position: 'fixed',
            top: '50%',
            left: '50%',
            transform: 'translate(-50%,-50%)',
            background: 'var(--surface)',
            borderRadius: 'var(--r-lg)',
            boxShadow: 'var(--shadow-pop)',
            width,
            maxWidth: 'calc(100vw - 40px)',
            maxHeight: 'calc(100vh - 60px)',
            overflow: 'hidden',
            display: 'flex',
            flexDirection: 'column',
            zIndex: 1000,
            animation: 'popIn .18s cubic-bezier(.2,.8,.3,1)',
          }}
        >
          {title || IconC ? (
            <div className="row" style={{ gap: 12, padding: '18px 20px 14px', alignItems: 'flex-start' }}>
              {IconC && (
                <span style={{ width: 34, height: 34, borderRadius: 9, background: it.bg, color: it.c, display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0 }}>
                  <IconC size={19} />
                </span>
              )}
              <div className="col grow" style={{ gap: 2 }}>
                <Dialog.Title style={{ fontSize: 15.5, fontWeight: 620, margin: 0 }}>{title}</Dialog.Title>
                {subtitle && <div className="t2 sm">{subtitle}</div>}
              </div>
              <IconBtn icon={I.x} onClick={onClose} />
            </div>
          ) : (
            <Dialog.Title style={srOnly}>对话框</Dialog.Title>
          )}
          <div style={{ padding: title ? '0 20px 4px' : 20, overflowY: 'auto' }}>{children}</div>
          {footer && (
            <div className="row between" style={{ gap: 10, padding: '16px 20px', borderTop: '1px solid var(--border)', marginTop: 8 }}>
              {footer}
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

// ---------- Drawer（右侧抽屉，Radix Dialog 底座） ----------
export function Drawer({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  width = 540,
  badge,
}: {
  open: boolean
  onClose?: () => void
  title?: ReactNode
  subtitle?: ReactNode
  children?: ReactNode
  footer?: ReactNode
  width?: number
  badge?: ReactNode
}) {
  return (
    <Dialog.Root open={open} onOpenChange={(o) => { if (!o) onClose?.() }}>
      <Dialog.Portal>
        <Dialog.Overlay style={{ ...overlayStyle, display: 'flex', justifyContent: 'flex-end' }} />
        <Dialog.Content
          aria-describedby={undefined}
          style={{
            position: 'fixed',
            top: 0,
            right: 0,
            background: 'var(--surface)',
            width,
            maxWidth: 'calc(100vw - 32px)',
            height: '100%',
            boxShadow: 'var(--shadow-pop)',
            display: 'flex',
            flexDirection: 'column',
            zIndex: 1000,
            animation: 'slideIn .22s cubic-bezier(.2,.8,.3,1)',
          }}
        >
          <div className="row between" style={{ padding: '16px 22px', borderBottom: '1px solid var(--border)', gap: 12 }}>
            <div className="col" style={{ gap: 2 }}>
              <div className="row gap8">
                <Dialog.Title style={{ fontSize: 15.5, fontWeight: 620, margin: 0 }}>{title}</Dialog.Title>
                {badge}
              </div>
              {subtitle && <div className="t2 sm">{subtitle}</div>}
            </div>
            <IconBtn icon={I.x} onClick={onClose} />
          </div>
          <div className="grow" style={{ overflowY: 'auto', padding: 22 }}>{children}</div>
          {footer && (
            <div className="row between" style={{ gap: 10, padding: '14px 22px', borderTop: '1px solid var(--border)' }}>
              {footer}
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

// ---------- Confirm dialog ----------
export function ConfirmDialog({
  open,
  onClose,
  onConfirm,
  title,
  message,
  confirmText = '确认',
  danger,
  icon,
}: {
  open: boolean
  onClose?: () => void
  onConfirm?: () => void
  title?: ReactNode
  message?: ReactNode
  confirmText?: string
  danger?: boolean
  icon?: IconComponent
}) {
  return (
    <Modal
      open={open}
      onClose={onClose}
      title={title}
      width={420}
      icon={icon || I.warn}
      iconTone={danger ? 'red' : 'violet'}
      footer={
        <>
          <span />
          <div className="row gap8">
            <Btn variant="default" onClick={onClose}>
              取消
            </Btn>
            <Btn variant={danger ? 'danger-solid' : 'primary'} onClick={() => onConfirm && onConfirm()}>
              {confirmText}
            </Btn>
          </div>
        </>
      }
    >
      <div className="t2" style={{ fontSize: 13.5, lineHeight: 1.6, paddingBottom: 6 }}>
        {message}
      </div>
    </Modal>
  )
}

// ---------- Dropdown menu（Radix DropdownMenu 底座） ----------
export type MenuItem =
  | { divider: true }
  | { icon?: IconComponent; label: ReactNode; onClick?: () => void; danger?: boolean; divider?: false }

export function Menu({
  trigger,
  items,
  align = 'right',
  width = 168,
  triggerStyle,
  triggerClassName,
}: {
  trigger: ReactNode
  items: MenuItem[]
  align?: 'right' | 'left'
  width?: number
  triggerStyle?: CSSProperties
  triggerClassName?: string
}) {
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger
        className={triggerClassName}
        style={{ border: 'none', background: 'transparent', cursor: 'pointer', display: 'inline-flex', alignItems: 'center', padding: 0, ...triggerStyle }}
      >
        {trigger}
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align={align === 'right' ? 'end' : 'start'}
          sideOffset={5}
          style={{
            width,
            zIndex: 50,
            background: 'var(--surface)',
            border: '1px solid var(--border-2)',
            borderRadius: 9,
            boxShadow: 'var(--shadow-pop)',
            padding: 5,
            animation: 'popIn .14s',
          }}
        >
          {items.map((it, i) =>
            'divider' in it && it.divider ? (
              <DropdownMenu.Separator key={i} style={{ height: 1, background: 'var(--border)', margin: '5px 0' }} />
            ) : (
              <DropdownMenu.Item
                key={i}
                onSelect={() => (it as Exclude<MenuItem, { divider: true }>).onClick?.()}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 9,
                  width: '100%',
                  padding: '7px 9px',
                  fontSize: 13,
                  fontWeight: 500,
                  border: 'none',
                  borderRadius: 6,
                  background: 'transparent',
                  outline: 'none',
                  color: (it as { danger?: boolean }).danger ? 'var(--red)' : 'var(--text)',
                  textAlign: 'left',
                  cursor: 'pointer',
                }}
                onMouseEnter={(e) => (e.currentTarget.style.background = (it as { danger?: boolean }).danger ? 'var(--red-bg)' : 'var(--surface-2)')}
                onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
              >
                {(() => {
                  const Ic = (it as { icon?: IconComponent }).icon
                  return Ic ? <Ic size={15} /> : null
                })()}
                {(it as { label?: ReactNode }).label}
              </DropdownMenu.Item>
            )
          )}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  )
}
