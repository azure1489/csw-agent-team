// ============================================================
// 实体类型 — API 形态（与后端 dto.go 返回 JSON 一致）+ 编辑器形态（前端内部）
// ============================================================
import type { AdminRole } from '@/lib/constants'

export type { AdminRole }

// ---------- 鉴权 ----------
export interface AdminUser {
  id: number
  username: string
  display_name: string
  role: AdminRole
  status: string // active | disabled
  last_login_at?: string | null
  created_at: string
}

export interface LoginResp {
  access_token: string
  expires_in: number
  user: AdminUser
}

// ---------- 工作流：API 形态 ----------
export interface ApiGate {
  gate_order?: number
  reviewer_role: string
  relayed_by_hub: boolean
  name?: string
}

export interface ApiStage {
  id: number
  seq: number
  code: string
  name: string
  role_code: string
  output_type?: string
  is_merge: boolean
  instructions?: string
  self_check_criteria?: string
  acceptance?: string
  dispatch_mode?: string | null // 阶段级派工模式覆盖：空=继承工作流 / manual / auto
  action_class?: string // read | platform_write:<scope>
  sla_minutes?: number | null // 派工后时限（分钟），0/空=不设
  ack_minutes?: number | null // 派工后多久未接单提醒（分钟），0/空=默认 5
  idle_minutes?: number | null // 接单后多久无活动提醒（分钟），0/空=默认 10
  per_item?: boolean // 按条目生成任务
  deps: string[] // 上游阶段 code
  gates: ApiGate[] // 覆盖闸（空=用 default_gates）
}

export interface ApiWorkflowMeta {
  id: number
  wf_key: string
  name: string
  version: number
  hub_role: string
  dispatch_mode: string
  trigger_roles: string // CSV
  status: string // draft | active | archived
  common_instructions?: string
  common_acceptance?: string
}

export interface ApiWorkflowFull {
  workflow: ApiWorkflowMeta
  stages: ApiStage[]
  default_gates: ApiGate[]
}

export interface WorkflowListItem extends ApiWorkflowMeta {
  stage_count: number
}

// ---------- 工作流：编辑器形态（与设计稿数据模型对齐：deps=id[]、gate.relayed、统一三段命名） ----------
export interface EditorGate {
  gate_order: number
  reviewer_role: string
  relayed: boolean
  name: string
}

export interface EditorStage {
  id: number
  seq: number
  code: string
  name: string
  role_code: string
  output_type: string
  is_merge: boolean
  instructions: string
  self_check: string
  acceptance: string
  dispatch_mode: string // '' = 继承工作流
  action_class: string // read | platform_write:<scope>
  sla_minutes: number // 0 = 不设
  ack_minutes: number // 0 = 默认 5
  idle_minutes: number // 0 = 默认 10
  per_item: boolean
  deps: number[] // 上游阶段 id
  gate_override: EditorGate[] | null
}

export interface EditorWorkflow {
  id: number
  wf_key: string
  name: string
  version: number
  status: string
  hub_role: string
  dispatch_mode: string
  trigger_roles: string[]
  common_instructions: string
  common_acceptance: string
  default_gates: EditorGate[]
  stages: EditorStage[]
}

// ---------- 角色 / 成员 / token ----------
export interface Role {
  code: string
  name: string
  is_management: boolean
  is_human: boolean
  member_count: number
}

export interface TokenInfo {
  id: number
  label: string
  status: string // active | revoked | expired
  last_used_at?: string | null
  expires_at?: string | null
  created_at: string
}

export interface Agent {
  id: number
  name: string
  role_code: string
  active: boolean
  tokens: TokenInfo[]
}

export interface IssuedToken {
  token_id: number
  token: string
  label: string
}

// ---------- 通讯录（花名册）----------
export interface Chat {
  id: number
  chat_key: string
  name: string
  note: string
  member_count: number
  created_at: string
  updated_at: string
}

export interface ChatMember {
  id: number
  chat_id: number
  kind: 'mapped' | 'reserved'
  role_code: string // mapped 必有；reserved 为 ""
  open_id: string
  user_id: string // 仅人类（Van）有；否则 ""
  display_name: string
  bot_name: string
  is_human: boolean
  sort: number
}

export interface MemberBody {
  kind: 'mapped' | 'reserved'
  role_code: string
  open_id: string
  user_id?: string
  display_name: string
  bot_name: string
  is_human: boolean
  sort: number
}

// ---------- 监控 ----------
export interface RunListItem {
  id: number
  workflow_id: number
  workflow_name: string
  workflow_key: string
  subject: string
  status: string
  current_stage: string
  trigger_name: string
  created_at: string
}

export interface RunMeta {
  id: number
  workflow_id: number
  workflow_ver: number
  subject: string
  title?: string | null
  status: string
}

export interface TaskBrief {
  id: number
  run_id: number
  stage_code: string
  stage_name: string
  seq: number
  role_code: string
  is_merge: boolean
  status: string
  cur_version: number
  assignee_id?: number | null
  action_class?: string
  dispatch_mode?: string
  sla_minutes?: number
  ack_minutes?: number
  idle_minutes?: number
  item_key?: string
  fail_reason?: string
  due_at?: string
  rework_pending?: boolean
}

export interface RunAuthorization {
  id: number
  scope: string // local_drill | wx_draft | wx_publish | xhs_draft | xhs_publish
  source_quote: string
  granted_at: string
  expires_at?: string
  revoked_at?: string
  status: string // active | revoked | expired
}

export interface RunItem {
  item_key: string
  title: string
  brand?: string
  status: string // candidate | shortlisted | approved_write | approved_research | deferred | rejected | written | reviewed | published
  decided_at?: string
  decision_source?: string
}

export interface RunDetail {
  run: RunMeta & { target_count?: number }
  tasks: TaskBrief[]
  authorizations?: RunAuthorization[]
  items?: RunItem[]
}

export interface TimelineEvent {
  id: number
  type: string
  task_id?: number | null
  deliverable_id?: number | null
  actor_id?: number | null
  detail: string
  created_at: string
}

export interface Upstream {
  label: string
  url: string
  upstream_id?: number | null
}

export interface Review {
  id: number
  task_gate_id: number
  reviewer_id?: number | null
  verdict: string // pass | reject
  comment?: string
  return_direction?: string | null
  return_location?: string | null
  decision_type?: string
  source_quote?: string
}

export interface Deliverable {
  id: number
  task_id: number
  is_dispatch: boolean
  kind?: string // dispatch | output | supplement | edit
  affects_deliverable_id?: number | null
  edit_of?: number | null
  diff_summary?: string
  collab?: boolean
  version: number
  doc_type: string
  download_url?: string
  filename?: string
  title?: string
  summary?: string
  self_check?: string
  editor_note?: string
  cur_gate: number
  returned_at_gate?: number | null
  status: string
  upstreams: Upstream[]
  reviews: Review[]
}

export interface TaskDetail {
  task: TaskBrief
  instructions?: string
  self_check_criteria?: string
  acceptance?: string
  gates: ApiGate[]
  deliverables: Deliverable[]
}

// ---------- 审计 / 概览 / 设置 ----------
export interface AuditEntry {
  id: number
  user_id?: number | null
  username?: string | null
  action: string
  target: string
  detail: string
  created_at: string
}

export interface Overview {
  counts: {
    active_workflows: number
    active_runs: number
    done_today: number
    pending_alerts: number
  }
  recent_runs: RunListItem[]
  recent_audit: AuditEntry[]
}

export interface ValidateCheck {
  name: string
  ok: boolean
  detail?: string
}

export interface ValidateReport {
  all_ok: boolean
  checks: ValidateCheck[]
}

export interface Settings {
  version: string
  jwt: { access_ttl_seconds: number; refresh_ttl_seconds: number; secret_generated: boolean }
  files: { max_upload_bytes: number; allowed_content_types: string[]; storage: string }
  base_url: string
  db_path: string
  cors_origins: string[]
  readonly: boolean
}

export interface ApiError {
  code: string
  message: string
}
