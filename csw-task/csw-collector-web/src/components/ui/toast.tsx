import { useEffect, useState } from 'react'
import { I, type IconComponent } from '@/components/icons'

export type ToastTone = 'success' | 'error' | 'warn' | 'default'

interface ToastItem {
  id: string
  msg: string
  tone: ToastTone
  desc?: string
}

/** 全局 toast：任意位置可调用，经 CustomEvent 投递给挂载的 ToastHost */
export function toast(msg: string, tone: ToastTone = 'default', desc?: string) {
  window.dispatchEvent(new CustomEvent('app-toast', { detail: { msg, tone, desc } }))
}

export function ToastHost() {
  const [items, setItems] = useState<ToastItem[]>([])
  useEffect(() => {
    const h = (e: Event) => {
      const ce = e as CustomEvent<{ msg: string; tone?: ToastTone; desc?: string }>
      const id = Math.random().toString(36).slice(2)
      const { msg, tone = 'default', desc } = ce.detail
      setItems((x) => [...x, { id, msg, tone, desc }])
      setTimeout(() => setItems((x) => x.filter((i) => i.id !== id)), 3400)
    }
    window.addEventListener('app-toast', h)
    return () => window.removeEventListener('app-toast', h)
  }, [])
  const iconFor: Record<ToastTone, IconComponent> = { success: I.checkCircle, error: I.xCircle, warn: I.warn, default: I.bolt }
  const colorFor: Record<ToastTone, string> = { success: 'var(--green)', error: 'var(--red)', warn: 'var(--amber)', default: 'var(--accent)' }
  return (
    <div style={{ position: 'fixed', bottom: 22, left: '50%', transform: 'translateX(-50%)', zIndex: 2000, display: 'flex', flexDirection: 'column', gap: 8, alignItems: 'center' }}>
      {items.map((it) => {
        const IconC = iconFor[it.tone] || iconFor.default
        return (
          <div
            key={it.id}
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              gap: 10,
              background: '#1f1f27',
              color: '#fff',
              padding: '11px 16px',
              borderRadius: 9,
              boxShadow: 'var(--shadow-pop)',
              animation: 'toastIn .2s',
              maxWidth: 460,
            }}
          >
            <span style={{ color: colorFor[it.tone], display: 'flex', marginTop: 1 }}>
              <IconC size={18} />
            </span>
            <div className="col" style={{ gap: 1 }}>
              <span style={{ fontSize: 13.5, fontWeight: 550 }}>{it.msg}</span>
              {it.desc && <span style={{ fontSize: 12, color: '#b5b5c0' }}>{it.desc}</span>}
            </div>
          </div>
        )
      })}
    </div>
  )
}
