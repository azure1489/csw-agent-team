import type { CSSProperties, ReactNode } from 'react'

export function Th({ children, center, right, style }: { children?: ReactNode; center?: boolean; right?: boolean; style?: CSSProperties }) {
  return <th style={{ textAlign: center ? 'center' : right ? 'right' : 'left', fontWeight: 550, padding: '10px 12px', ...style }}>{children}</th>
}

export function Td({ children, center, right, style }: { children?: ReactNode; center?: boolean; right?: boolean; style?: CSSProperties }) {
  return <td style={{ textAlign: center ? 'center' : right ? 'right' : 'left', padding: '11px 12px', verticalAlign: 'middle', ...style }}>{children}</td>
}
