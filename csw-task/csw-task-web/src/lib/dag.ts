// ============================================================
// DAG 计算 — 分层 + 环检测（迁自设计稿 dag.jsx，deps 用 stage id）
// ============================================================
import type { EditorStage } from '@/types'

export interface DagResult {
  byId: Record<number, EditorStage>
  layer: Record<number, number>
  entries: Set<number>
  terminals: Set<number>
  hasCycle: boolean
  cycleNodes: Set<number>
}

export function computeDag(stages: EditorStage[]): DagResult {
  const byId: Record<number, EditorStage> = {}
  stages.forEach((s) => (byId[s.id] = s))
  const ids = stages.map((s) => s.id)

  // cycle detection (DFS, white-gray-black)
  const WHITE = 0,
    GRAY = 1,
    BLACK = 2
  const color: Record<number, number> = {}
  ids.forEach((i) => (color[i] = WHITE))
  const cycleNodes = new Set<number>()
  let hasCycle = false
  const stack: number[] = []
  function dfs(u: number) {
    color[u] = GRAY
    stack.push(u)
    for (const d of byId[u]?.deps || []) {
      if (!byId[d]) continue
      if (color[d] === GRAY) {
        hasCycle = true
        const idx = stack.indexOf(d)
        stack.slice(idx).forEach((n) => cycleNodes.add(n))
      } else if (color[d] === WHITE) {
        dfs(d)
      }
    }
    color[u] = BLACK
    stack.pop()
  }
  ids.forEach((i) => {
    if (color[i] === WHITE) dfs(i)
  })

  // layering via longest path from roots (skip if cycle)
  const memo: Record<number, number> = {}
  function lp(u: number, seen: Set<number>): number {
    if (memo[u] != null) return memo[u]
    if (seen.has(u)) return 0
    seen.add(u)
    const deps = (byId[u]?.deps || []).filter((d) => byId[d])
    const v = deps.length ? Math.max(...deps.map((d) => lp(d, new Set(seen)) + 1)) : 0
    memo[u] = v
    return v
  }
  const layer: Record<number, number> = {}
  ids.forEach((i) => (layer[i] = hasCycle ? byId[i].seq - 1 : lp(i, new Set())))

  // entry (no deps) & terminal (no downstream)
  const hasDown = new Set<number>()
  stages.forEach((s) => (s.deps || []).forEach((d) => hasDown.add(d)))
  const entries = new Set(ids.filter((i) => (byId[i].deps || []).length === 0))
  const terminals = new Set(ids.filter((i) => !hasDown.has(i)))

  return { byId, layer, entries, terminals, hasCycle, cycleNodes }
}
