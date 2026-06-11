// ============================================================
// DAG 可视化 — 分层 SVG + 贝塞尔边 + 环检测（迁自设计稿 dag.jsx）
// ============================================================
import { useMemo } from 'react'
import { I } from '@/components/icons'
import { computeDag } from '@/lib/dag'
import { useRoleName } from '@/hooks/useRoles'
import type { EditorStage } from '@/types'

export function DagView({ stages }: { stages: EditorStage[] }) {
  const roleName = useRoleName()
  const { byId, layer, entries, terminals, hasCycle, cycleNodes } = useMemo(() => computeDag(stages), [stages])

  // group by layer
  const layers: Record<number, EditorStage[]> = {}
  stages.forEach((s) => {
    const l = layer[s.id]
    ;(layers[l] = layers[l] || []).push(s)
  })
  Object.values(layers).forEach((arr) => arr.sort((a, b) => a.seq - b.seq))
  const layerKeys = Object.keys(layers)
    .map(Number)
    .sort((a, b) => a - b)
  const maxRows = Math.max(...layerKeys.map((k) => layers[k].length), 1)

  const NW = 138,
    NH = 50,
    COLGAP = 54,
    ROWGAP = 22
  const colStep = NW + COLGAP,
    rowStep = NH + ROWGAP
  const pos: Record<number, { x: number; y: number }> = {}
  layerKeys.forEach((l, ci) => {
    const arr = layers[l]
    const offset = (maxRows - arr.length) / 2
    arr.forEach((s, ri) => {
      pos[s.id] = { x: 24 + ci * colStep, y: 20 + (ri + offset) * rowStep }
    })
  })
  const W = 24 + layerKeys.length * colStep + 10
  const H = Math.max(20 + maxRows * rowStep + 10, 120)

  const edges: [number, number][] = []
  stages.forEach((s) => (s.deps || []).forEach((d) => { if (pos[d] && pos[s.id]) edges.push([d, s.id]) }))

  return (
    <div style={{ position: 'relative' }}>
      {hasCycle && (
        <div className="row gap8" style={{ marginBottom: 12, background: 'var(--red-bg)', border: '1px solid var(--red-bd)', color: 'var(--red)', padding: '8px 12px', borderRadius: 7, fontSize: 12.5 }}>
          <I.warn size={15} />
          检测到环依赖，已用红色标出相关阶段。请在「阶段 / 依赖」中修正后再激活。
        </div>
      )}
      <div style={{ overflowX: 'auto', overflowY: 'hidden', background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 9, padding: 4 }}>
        <svg width={W} height={H} style={{ display: 'block', minWidth: '100%' }}>
          <defs>
            <marker id="arrow" markerWidth="9" markerHeight="9" refX="7" refY="4.5" orient="auto">
              <path d="M1,1 L8,4.5 L1,8" fill="none" stroke="#b9b9c4" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
            </marker>
          </defs>
          {edges.map(([a, b], i) => {
            const p1 = pos[a],
              p2 = pos[b]
            const x1 = p1.x + NW,
              y1 = p1.y + NH / 2
            const x2 = p2.x,
              y2 = p2.y + NH / 2
            const mx = (x1 + x2) / 2
            const bad = cycleNodes.has(a) && cycleNodes.has(b)
            return (
              <path
                key={i}
                d={`M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2 - 4},${y2}`}
                fill="none"
                stroke={bad ? 'var(--red)' : '#c9c9d2'}
                strokeWidth={bad ? 2 : 1.6}
                markerEnd="url(#arrow)"
              />
            )
          })}
          {stages.map((s) => {
            const p = pos[s.id]
            if (!p) return null
            const isEntry = entries.has(s.id),
              isTerm = terminals.has(s.id),
              bad = cycleNodes.has(s.id)
            const ring = bad ? 'var(--red)' : isEntry ? 'var(--green)' : isTerm ? 'var(--accent)' : 'var(--border-strong)'
            const rn = roleName(s.role_code)
            return (
              <g key={s.id} transform={`translate(${p.x},${p.y})`}>
                <rect width={NW} height={NH} rx="9" fill="var(--surface)" stroke={ring} strokeWidth={isEntry || isTerm || bad ? 1.8 : 1.2} />
                {s.is_merge && <rect x={NW - 7} y="0" width="7" height={NH} rx="3" fill="var(--accent-weak-2)" />}
                <circle cx="17" cy={NH / 2} r="11" fill={bad ? 'var(--red-bg)' : 'var(--accent-weak)'} />
                <text x="17" y={NH / 2 + 3.5} textAnchor="middle" fontSize="10.5" fontWeight="700" fill={bad ? 'var(--red)' : 'var(--accent-text)'}>
                  {s.seq}
                </text>
                <text x="34" y={NH / 2 - 3} fontSize="11.5" fontWeight="600" fill="var(--text)">
                  {s.name.length > 9 ? s.name.slice(0, 8) + '…' : s.name}
                </text>
                <text x="34" y={NH / 2 + 12} fontSize="10" fill="var(--text-3)">
                  {rn}
                  {s.is_merge ? ' · 合流' : ''}
                </text>
              </g>
            )
          })}
        </svg>
      </div>
      <div className="row gap16 t3 xs" style={{ marginTop: 10, flexWrap: 'wrap' }}>
        <span className="row gap6">
          <span style={{ width: 11, height: 11, borderRadius: 3, border: '1.8px solid var(--green)' }} />
          入口（无依赖）
        </span>
        <span className="row gap6">
          <span style={{ width: 11, height: 11, borderRadius: 3, border: '1.8px solid var(--accent)' }} />
          终点（无下游）
        </span>
        <span className="row gap6">
          <span style={{ width: 8, height: 11, borderRadius: 2, background: 'var(--accent-weak-2)' }} />
          合流阶段
        </span>
        {hasCycle && (
          <span className="row gap6" style={{ color: 'var(--red)' }}>
            <span style={{ width: 11, height: 11, borderRadius: 3, border: '1.8px solid var(--red)' }} />
            环依赖
          </span>
        )}
      </div>
    </div>
  )
}
