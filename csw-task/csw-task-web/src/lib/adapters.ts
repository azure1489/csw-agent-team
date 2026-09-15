// ============================================================
// 适配层 — 后端 API 形态 ↔ 编辑器形态（字段名映射 + deps 的 id↔code 转换）
// ============================================================
import type { ApiWorkflowFull, ApiGate, EditorGate, EditorStage, EditorWorkflow } from '@/types'

function csvToArr(s?: string): string[] {
  return (s || '')
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean)
}

function apiGateToEditor(g: ApiGate, order: number): EditorGate {
  return {
    gate_order: g.gate_order ?? order,
    reviewer_role: g.reviewer_role,
    relayed: !!g.relayed_by_hub,
    name: g.name || '',
  }
}

function editorGateToApi(g: EditorGate, order: number): ApiGate {
  return {
    gate_order: order,
    reviewer_role: g.reviewer_role,
    relayed_by_hub: g.relayed,
    name: g.name || '',
  }
}

/** 后端 get-full → 编辑器工作流（deps code→id、relayed_by_hub→relayed、self_check_criteria→self_check） */
export function apiToEditor(full: ApiWorkflowFull): EditorWorkflow {
  const w = full.workflow
  const codeToId: Record<string, number> = {}
  full.stages.forEach((s) => (codeToId[s.code] = s.id))

  const stages: EditorStage[] = full.stages.map((s) => ({
    id: s.id,
    seq: s.seq,
    code: s.code,
    name: s.name,
    role_code: s.role_code,
    output_type: s.output_type || '',
    is_merge: !!s.is_merge,
    instructions: s.instructions || '',
    self_check: s.self_check_criteria || '',
    acceptance: s.acceptance || '',
    dispatch_mode: s.dispatch_mode || '',
    action_class: s.action_class || 'read',
    sla_minutes: s.sla_minutes || 0,
    per_item: !!s.per_item,
    deps: (s.deps || []).map((c) => codeToId[c]).filter((x) => x != null),
    gate_override: s.gates && s.gates.length ? s.gates.map((g, i) => apiGateToEditor(g, i + 1)) : null,
  }))

  return {
    id: w.id,
    wf_key: w.wf_key,
    name: w.name,
    version: w.version,
    status: w.status,
    hub_role: w.hub_role,
    dispatch_mode: w.dispatch_mode,
    trigger_roles: csvToArr(w.trigger_roles),
    common_instructions: w.common_instructions || '',
    common_acceptance: w.common_acceptance || '',
    default_gates: (full.default_gates || []).map((g, i) => apiGateToEditor(g, i + 1)),
    stages,
  }
}

export interface PutWorkflowBody {
  name: string
  hub_role: string
  dispatch_mode: string
  trigger_roles: string
  common_instructions: string
  common_acceptance: string
  stages: {
    code: string
    name: string
    role_code: string
    output_type: string
    instructions: string
    self_check_criteria: string
    acceptance: string
    is_merge: boolean
    dispatch_mode: string
    action_class: string
    sla_minutes: number
    per_item: boolean
    deps: string[]
    gates: ApiGate[]
  }[]
  default_gates: ApiGate[]
}

/** 编辑器工作流 → PUT body（deps id→code、relayed→relayed_by_hub、self_check→self_check_criteria、trigger_roles→CSV） */
export function editorToPutBody(wf: EditorWorkflow): PutWorkflowBody {
  const idToCode: Record<number, string> = {}
  wf.stages.forEach((s) => (idToCode[s.id] = s.code))

  return {
    name: wf.name,
    hub_role: wf.hub_role,
    dispatch_mode: wf.dispatch_mode,
    trigger_roles: wf.trigger_roles.join(','),
    common_instructions: wf.common_instructions,
    common_acceptance: wf.common_acceptance,
    stages: wf.stages.map((s) => ({
      code: s.code,
      name: s.name,
      role_code: s.role_code,
      output_type: s.output_type,
      instructions: s.instructions,
      self_check_criteria: s.self_check,
      acceptance: s.acceptance,
      is_merge: s.is_merge,
      dispatch_mode: s.dispatch_mode || '',
      action_class: s.action_class || 'read',
      sla_minutes: s.sla_minutes || 0,
      per_item: !!s.per_item,
      deps: (s.deps || []).map((id) => idToCode[id]).filter(Boolean),
      gates: s.gate_override ? s.gate_override.map((g, i) => editorGateToApi(g, i + 1)) : [],
    })),
    default_gates: wf.default_gates.map((g, i) => editorGateToApi(g, i + 1)),
  }
}
