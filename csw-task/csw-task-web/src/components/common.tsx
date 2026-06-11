import type { ReactNode } from 'react'
import { Badge } from '@/components/ui'
import { ACTION_META } from '@/lib/constants'

export function ActionTag({ action }: { action: string }) {
  const m = ACTION_META[action] || { label: action, tone: 'gray' as const }
  return (
    <Badge tone={m.tone} sm>
      {m.label}
    </Badge>
  )
}

export function ProgressPill({ text }: { text: ReactNode }) {
  return (
    <span
      className="row gap6 mono"
      style={{
        fontSize: 12,
        fontWeight: 600,
        color: 'var(--accent-text)',
        background: 'var(--accent-weak)',
        border: '1px solid var(--accent-weak-2)',
        padding: '2px 9px',
        borderRadius: 999,
      }}
    >
      {text}
    </span>
  )
}
