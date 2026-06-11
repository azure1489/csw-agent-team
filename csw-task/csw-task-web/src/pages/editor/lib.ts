// ============================================================
// 编辑器辅助 — 产出类型选项 + 前端即时校验 runChecks（迁自设计稿）
// 注：激活由后端 activate 权威校验；此处为编辑期即时预览（含 warn / 定位）
// ============================================================
import { computeDag } from '@/lib/dag'
import type { EditorWorkflow, Role } from '@/types'

export const OUTPUT_OPTS = ['资讯包', '选题成品', '文章', '封面', '成品', '发布物', '小红书文本', '卡片', '素材包', '提纲', '初稿', '配图']

export interface FrontCheck {
  status: 'pass' | 'warn' | 'fail'
  text: string
  stageId?: number
}

export function runChecks(draft: EditorWorkflow, roles: Role[]): FrontCheck[] {
  const checks: FrontCheck[] = []
  const { entries, terminals, hasCycle } = computeDag(draft.stages)
  const byId: Record<number, EditorWorkflow['stages'][number]> = {}
  draft.stages.forEach((s) => (byId[s.id] = s))

  checks.push({ status: hasCycle ? 'fail' : 'pass', text: hasCycle ? 'DAG 存在环依赖' : 'DAG 无环' })

  // reachability from entries
  const adj: Record<number, number[]> = {}
  draft.stages.forEach((s) => (adj[s.id] = []))
  draft.stages.forEach((s) => (s.deps || []).forEach((d) => { if (adj[d]) adj[d].push(s.id) }))
  const reach = new Set<number>()
  const q: number[] = [...entries]
  entries.forEach((e) => reach.add(e))
  while (q.length) {
    const u = q.shift()!
    ;(adj[u] || []).forEach((v) => {
      if (!reach.has(v)) {
        reach.add(v)
        q.push(v)
      }
    })
  }
  const allReach = draft.stages.every((s) => reach.has(s.id))
  const entryArr = [...entries].map((id) => byId[id]?.name).join('、')
  if (entries.size === 1) checks.push({ status: allReach ? 'pass' : 'fail', text: `单一入口（${entryArr}）${allReach ? '可达所有阶段' : '无法到达全部阶段'}` })
  else if (entries.size === 0) checks.push({ status: 'fail', text: '没有入口阶段（所有阶段都有依赖，构成环）' })
  else checks.push({ status: 'warn', text: `存在 ${entries.size} 个入口（${entryArr}）——多入口，请确认是否预期` })

  const termArr = [...terminals].map((id) => byId[id]?.name).join('、')
  checks.push({ status: terminals.size ? 'pass' : 'fail', text: terminals.size ? `终点（${termArr}）可达` : '没有终点阶段' })

  const hub = roles.find((r) => r.code === draft.hub_role)
  checks.push({ status: hub && hub.is_management ? 'pass' : 'fail', text: hub && hub.is_management ? `中枢角色「${hub.name}」为管理类` : '中枢角色非管理类或不存在' })

  const allGates = [...draft.default_gates, ...draft.stages.flatMap((s) => s.gate_override || [])]
  const badGate = allGates.find((g) => !roles.find((r) => r.code === g.reviewer_role))
  checks.push({ status: badGate ? 'fail' : 'pass', text: badGate ? `审核角色「${badGate.reviewer_role}」不存在` : '各闸审核角色均存在' })

  const emptyStages = draft.stages.filter((s) => !((s.instructions || '').trim() && (s.self_check || '').trim() && (s.acceptance || '').trim()))
  if (emptyStages.length === 0) checks.push({ status: 'pass', text: '每阶段 instructions / self_check / acceptance 均非空' })
  else emptyStages.forEach((s) => checks.push({ status: 'fail', text: `${s.name}：作业手册 / 自检 / 验收 存在空项`, stageId: s.id }))

  const codes = draft.stages.map((s) => s.code)
  const dup = codes.find((c, i) => codes.indexOf(c) !== i)
  if (dup) checks.push({ status: 'fail', text: `阶段 code「${dup}」重复` })

  if (draft.stages.length === 0) checks.push({ status: 'fail', text: '至少需要 1 个阶段' })

  const mergeHub = draft.stages.filter((s) => s.is_merge && s.role_code === draft.hub_role)
  if (mergeHub.length) checks.push({ status: 'warn', text: `${mergeHub.map((s) => s.seq).join('、')} 为合流且责任=中枢，将自动派工给中枢自己（提示）` })

  return checks
}
