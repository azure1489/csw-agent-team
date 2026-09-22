import type { CSSProperties, ReactNode } from 'react'

export function Th({
  children,
  center,
  right,
  style,
  className,
}: {
  children?: ReactNode
  center?: boolean
  right?: boolean
  style?: CSSProperties
  className?: string
}) {
  return (
    <th
      className={className}
      style={{ textAlign: center ? 'center' : right ? 'right' : 'left', fontWeight: 550, padding: '10px 12px', ...style }}
    >
      {children}
    </th>
  )
}

export function Td({
  children,
  center,
  right,
  style,
  className,
}: {
  children?: ReactNode
  center?: boolean
  right?: boolean
  style?: CSSProperties
  className?: string
}) {
  return (
    <td
      className={className}
      style={{ textAlign: center ? 'center' : right ? 'right' : 'left', padding: '11px 12px', verticalAlign: 'middle', ...style }}
    >
      {children}
    </td>
  )
}
