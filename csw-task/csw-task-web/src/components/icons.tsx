// ============================================================
// Icons — Linear-style line icons (stroke, currentColor)
// 迁自设计稿 csw-admin/icons.jsx
// ============================================================
import type { CSSProperties, ReactElement, ReactNode } from 'react'

export interface IconProps {
  size?: number
  fill?: string
  stroke?: number
  style?: CSSProperties
  className?: string
}

type IconInternalProps = IconProps & { d?: string; children?: ReactNode }

const Icon = ({ d, size = 18, fill, stroke = 1.6, children, style, className }: IconInternalProps) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill={fill || 'none'}
    stroke="currentColor"
    strokeWidth={stroke}
    strokeLinecap="round"
    strokeLinejoin="round"
    className={className}
    style={{ flexShrink: 0, ...style }}
  >
    {d ? <path d={d} /> : children}
  </svg>
)

export type IconComponent = (p: IconProps) => ReactElement

export const I: Record<string, IconComponent> = {
  dashboard: (p) => <Icon {...p}><rect x="3" y="3" width="7" height="9" rx="1.5" /><rect x="14" y="3" width="7" height="5" rx="1.5" /><rect x="14" y="12" width="7" height="9" rx="1.5" /><rect x="3" y="16" width="7" height="5" rx="1.5" /></Icon>,
  workflow: (p) => <Icon {...p}><rect x="3" y="4" width="6" height="6" rx="1.5" /><rect x="15" y="4" width="6" height="6" rx="1.5" /><rect x="9" y="14" width="6" height="6" rx="1.5" /><path d="M6 10v1.5a2 2 0 0 0 2 2h1M18 10v1.5a2 2 0 0 1-2 2h-1" /></Icon>,
  members: (p) => <Icon {...p}><circle cx="9" cy="8" r="3" /><path d="M3.5 19a5.5 5.5 0 0 1 11 0" /><path d="M16 6.5a2.8 2.8 0 0 1 0 5.4" /><path d="M17.5 14.2A5 5 0 0 1 21 19" /></Icon>,
  adminUser: (p) => <Icon {...p}><circle cx="12" cy="8" r="3.2" /><path d="M5.5 20a6.5 6.5 0 0 1 13 0" /><path d="m17 3 1 1.6L19.8 5 18.4 6.2 18.7 8 17 7.1 15.3 8l.3-1.8L14.2 5l1.8-.4z" fill="currentColor" stroke="none" /></Icon>,
  runs: (p) => <Icon {...p}><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3.5 2" /></Icon>,
  audit: (p) => <Icon {...p}><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /><path d="M9 13h6M9 17h4" /></Icon>,
  settings: (p) => <Icon {...p}><circle cx="12" cy="12" r="3" /><path d="M19.4 13a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1V19a2 2 0 1 1-4 0 1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0-1.1-2.7H2a2 2 0 1 1 0-4 1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H8a1.6 1.6 0 0 0 1-1.5V2a2 2 0 1 1 4 0 1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V8a1.6 1.6 0 0 0 1.5 1H22a2 2 0 1 1 0 4 1.6 1.6 0 0 0-1.5 1z" /></Icon>,
  search: (p) => <Icon {...p}><circle cx="11" cy="11" r="7" /><path d="m20 20-3.2-3.2" /></Icon>,
  plus: (p) => <Icon {...p}><path d="M12 5v14M5 12h14" /></Icon>,
  chevDown: (p) => <Icon {...p}><path d="m6 9 6 6 6-6" /></Icon>,
  chevRight: (p) => <Icon {...p}><path d="m9 6 6 6-6 6" /></Icon>,
  chevLeft: (p) => <Icon {...p}><path d="m15 6-6 6 6 6" /></Icon>,
  more: (p) => <Icon {...p}><circle cx="12" cy="5" r="1.4" fill="currentColor" stroke="none" /><circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" /><circle cx="12" cy="19" r="1.4" fill="currentColor" stroke="none" /></Icon>,
  edit: (p) => <Icon {...p}><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" /></Icon>,
  eye: (p) => <Icon {...p}><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z" /><circle cx="12" cy="12" r="3" /></Icon>,
  check: (p) => <Icon {...p}><path d="m20 6-11 11-5-5" /></Icon>,
  checkCircle: (p) => <Icon {...p}><circle cx="12" cy="12" r="9" /><path d="m8.5 12 2.3 2.3L16 9" /></Icon>,
  x: (p) => <Icon {...p}><path d="M18 6 6 18M6 6l12 12" /></Icon>,
  xCircle: (p) => <Icon {...p}><circle cx="12" cy="12" r="9" /><path d="m15 9-6 6M9 9l6 6" /></Icon>,
  warn: (p) => <Icon {...p}><path d="M10.3 3.8 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.8a2 2 0 0 0-3.4 0z" /><path d="M12 9v4M12 17h.01" /></Icon>,
  copy: (p) => <Icon {...p}><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h8" /></Icon>,
  download: (p) => <Icon {...p}><path d="M12 3v12m0 0 4-4m-4 4-4-4" /><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" /></Icon>,
  drag: (p) => <Icon {...p}><circle cx="9" cy="6" r="1.3" fill="currentColor" stroke="none" /><circle cx="15" cy="6" r="1.3" fill="currentColor" stroke="none" /><circle cx="9" cy="12" r="1.3" fill="currentColor" stroke="none" /><circle cx="15" cy="12" r="1.3" fill="currentColor" stroke="none" /><circle cx="9" cy="18" r="1.3" fill="currentColor" stroke="none" /><circle cx="15" cy="18" r="1.3" fill="currentColor" stroke="none" /></Icon>,
  logout: (p) => <Icon {...p}><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" /><path d="M16 17l5-5-5-5M21 12H9" /></Icon>,
  user: (p) => <Icon {...p}><circle cx="12" cy="8" r="3.5" /><path d="M5 20a7 7 0 0 1 14 0" /></Icon>,
  lock: (p) => <Icon {...p}><rect x="4" y="10" width="16" height="11" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" /></Icon>,
  key: (p) => <Icon {...p}><circle cx="8" cy="15" r="4" /><path d="M10.8 12.2 19 4m-3 0h3v3" /></Icon>,
  bolt: (p) => <Icon {...p}><path d="M13 2 4 14h7l-1 8 9-12h-7z" /></Icon>,
  hand: (p) => <Icon {...p}><path d="M18 11V6a1.5 1.5 0 0 0-3 0M15 6V4.5a1.5 1.5 0 0 0-3 0V6M12 6V5a1.5 1.5 0 0 0-3 0v7" /><path d="M9 12V8.5a1.5 1.5 0 0 0-3 0V14a7 7 0 0 0 7 7h0a7 7 0 0 0 7-7v-3" /></Icon>,
  refresh: (p) => <Icon {...p}><path d="M21 12a9 9 0 1 1-2.6-6.4M21 4v4h-4" /></Icon>,
  doc: (p) => <Icon {...p}><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /></Icon>,
  flow: (p) => <Icon {...p}><circle cx="5" cy="6" r="2.2" /><circle cx="5" cy="18" r="2.2" /><circle cx="19" cy="12" r="2.2" /><path d="M7 7l9 4M7 17l9-4" /></Icon>,
  inbox: (p) => <Icon {...p}><path d="M22 12h-6l-2 3h-4l-2-3H2" /><path d="M5.5 5h13l3.5 7v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-6z" /></Icon>,
  link: (p) => <Icon {...p}><path d="M9 15l6-6" /><path d="M11 6.5 13 4.5a3.5 3.5 0 0 1 5 5l-2 2M13 17.5l-2 2a3.5 3.5 0 0 1-5-5l2-2" /></Icon>,
  filter: (p) => <Icon {...p}><path d="M3 5h18l-7 8v6l-4-2v-4z" /></Icon>,
  clock: (p) => <Icon {...p}><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></Icon>,
  building: (p) => <Icon {...p}><rect x="5" y="3" width="14" height="18" rx="1.5" /><path d="M9 7h2M13 7h2M9 11h2M13 11h2M9 15h2M13 15h2" /></Icon>,
}
