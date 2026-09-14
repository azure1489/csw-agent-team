// ============================================================
// API 客户端 — axios + JWT（access 内存 / refresh cookie）+ 401 自动续期重试
// ============================================================
import axios, { type InternalAxiosRequestConfig } from 'axios'
import { uuid } from './utils'
import type { PutWorkflowBody } from './adapters'
import type {
  AdminUser,
  Agent,
  ApiWorkflowFull,
  AuditEntry,
  Chat,
  ChatMember,
  Deliverable,
  IssuedToken,
  LoginResp,
  MemberBody,
  Overview,
  Role,
  RunDetail,
  RunListItem,
  Settings,
  TaskDetail,
  TimelineEvent,
  ValidateReport,
  WorkflowListItem,
} from '@/types'

const API_BASE = (import.meta.env.VITE_API_BASE || 'http://localhost:8081').replace(/\/$/, '')
const ADMIN = `${API_BASE}/admin`

let accessToken: string | null = null
export function setAccessToken(t: string | null) {
  accessToken = t
}

let onAuthFailed: () => void = () => {}
export function setOnAuthFailed(fn: () => void) {
  onAuthFailed = fn
}

const http = axios.create({ baseURL: ADMIN, withCredentials: true })

http.interceptors.request.use((cfg: InternalAxiosRequestConfig) => {
  if (accessToken && cfg.headers) cfg.headers.Authorization = `Bearer ${accessToken}`
  const m = (cfg.method || 'get').toLowerCase()
  if (m !== 'get' && cfg.headers && !cfg.headers['Idempotency-Key']) cfg.headers['Idempotency-Key'] = uuid()
  return cfg
})

// 裸 axios 刷新（不经过 http 拦截器，避免循环）
let refreshing: Promise<string> | null = null
async function doRefresh(): Promise<string> {
  const { data } = await axios.post<LoginResp>(`${ADMIN}/refresh`, {}, { withCredentials: true })
  setAccessToken(data.access_token)
  return data.access_token
}

http.interceptors.response.use(
  (r) => r,
  async (error) => {
    const cfg = error.config as (InternalAxiosRequestConfig & { __isRetry?: boolean }) | undefined
    const status = error.response?.status
    const url: string = cfg?.url || ''
    if (status === 401 && cfg && !cfg.__isRetry && !url.includes('/refresh') && !url.includes('/login')) {
      cfg.__isRetry = true
      try {
        if (!refreshing) refreshing = doRefresh().finally(() => (refreshing = null))
        const tok = await refreshing
        if (cfg.headers) cfg.headers.Authorization = `Bearer ${tok}`
        return http(cfg)
      } catch {
        onAuthFailed()
        throw error
      }
    }
    throw error
  }
)

export interface ApiErr {
  code: string
  message: string
}

/** 把异常归一化为 {code,message}（后端统一错误体） */
export function getApiError(e: unknown): ApiErr {
  if (axios.isAxiosError(e) && e.response?.data) {
    const d = e.response.data as { code?: string; message?: string }
    if (d && typeof d.code === 'string') return { code: d.code, message: d.message || d.code }
  }
  return { code: 'network', message: '网络或服务器错误' }
}

// ---------- 端点封装 ----------
export const api = {
  // auth
  login: (username: string, password: string) => http.post<LoginResp>('/login', { username, password }).then((r) => r.data),
  refresh: () => axios.post<LoginResp>(`${ADMIN}/refresh`, {}, { withCredentials: true }).then((r) => r.data),
  me: () => http.get<{ user: AdminUser }>('/me').then((r) => r.data.user),
  logout: () => http.post('/logout').then((r) => r.data),

  // overview
  overview: () => http.get<Overview>('/overview').then((r) => r.data),

  // workflows
  listWorkflows: (params?: { search?: string; status?: string }) =>
    http.get<{ workflows: WorkflowListItem[] }>('/workflows', { params }).then((r) => r.data.workflows),
  getWorkflow: (id: number) => http.get<ApiWorkflowFull>(`/workflows/${id}`).then((r) => r.data),
  createWorkflow: (body: { wf_key: string; name: string; hub_role: string; dispatch_mode: string; trigger_roles: string }) =>
    http.post<{ id: number }>('/workflows', body).then((r) => r.data.id),
  putWorkflow: (id: number, body: PutWorkflowBody) => http.put(`/workflows/${id}`, body).then((r) => r.data),
  validateWorkflow: (id: number) => http.post<ValidateReport>(`/workflows/${id}/validate`).then((r) => r.data),
  activateWorkflow: (id: number) => http.post<{ ok: boolean; checks: ValidateReport['checks'] }>(`/workflows/${id}/activate`).then((r) => r.data),
  archiveWorkflow: (id: number) => http.post(`/workflows/${id}/archive`).then((r) => r.data),
  cloneWorkflow: (id: number) => http.post<{ id: number }>(`/workflows/${id}/clone`).then((r) => r.data.id),

  // roles
  listRoles: () => http.get<{ roles: Role[] }>('/roles').then((r) => r.data.roles),
  createRole: (body: { code: string; name: string; is_management: boolean; is_human: boolean }) => http.post('/roles', body).then((r) => r.data),
  updateRole: (code: string, body: { name: string; is_management: boolean; is_human: boolean }) => http.patch(`/roles/${code}`, body).then((r) => r.data),

  // agents / tokens
  listAgents: () => http.get<{ agents: Agent[] }>('/agents').then((r) => r.data.agents),
  createAgent: (body: { role_code: string; name: string }) => http.post<{ id: number }>('/agents', body).then((r) => r.data.id),
  updateAgent: (id: number, body: { name: string; role_code: string; active: boolean }) => http.patch(`/agents/${id}`, body).then((r) => r.data),
  issueToken: (id: number, body: { label?: string; expires_at?: string }) => http.post<IssuedToken>(`/agents/${id}/tokens`, body).then((r) => r.data),
  revokeToken: (id: number, tid: number) => http.post(`/agents/${id}/tokens/${tid}/revoke`).then((r) => r.data),

  // roster（通讯录）
  listChats: () => http.get<{ chats: Chat[] }>('/chats').then((r) => r.data.chats),
  listChatMembers: (id: number) => http.get<{ members: ChatMember[] }>(`/chats/${id}/members`).then((r) => r.data.members),
  createChat: (body: { chat_key: string; name: string; note?: string }) => http.post<{ id: number }>('/chats', body).then((r) => r.data.id),
  updateChat: (id: number, body: { name: string; note?: string }) => http.patch(`/chats/${id}`, body).then((r) => r.data),
  deleteChat: (id: number) => http.delete(`/chats/${id}`).then((r) => r.data),
  createChatMember: (cid: number, body: MemberBody) => http.post<{ id: number }>(`/chats/${cid}/members`, body).then((r) => r.data.id),
  updateChatMember: (cid: number, mid: number, body: MemberBody) => http.patch(`/chats/${cid}/members/${mid}`, body).then((r) => r.data),
  deleteChatMember: (cid: number, mid: number) => http.delete(`/chats/${cid}/members/${mid}`).then((r) => r.data),

  // admin users
  listUsers: () => http.get<{ users: AdminUser[] }>('/users').then((r) => r.data.users),
  createUser: (body: { username: string; password: string; display_name?: string; role: string }) => http.post<{ id: number }>('/users', body).then((r) => r.data.id),
  updateUser: (id: number, body: { display_name?: string; role: string; status: string }) => http.patch(`/users/${id}`, body).then((r) => r.data),
  resetPassword: (id: number) => http.post<{ temp_password: string; note: string }>(`/users/${id}/reset-password`).then((r) => r.data),

  // monitor
  listRuns: (params?: { workflow?: string; status?: string; date?: string }) =>
    http.get<{ runs: RunListItem[] }>('/runs', { params }).then((r) => r.data.runs),
  getRun: (id: number) => http.get<RunDetail>(`/runs/${id}`).then((r) => r.data),
  getTimeline: (id: number) => http.get<{ events: TimelineEvent[] }>(`/runs/${id}/timeline`).then((r) => r.data.events),
  getTask: (id: number) => http.get<TaskDetail>(`/tasks/${id}`).then((r) => r.data),
  getDeliverable: (id: number) => http.get<{ deliverable: Deliverable }>(`/deliverables/${id}`).then((r) => r.data.deliverable),

  // audit / settings
  listAudit: (params?: { user_id?: number; action?: string; date?: string }) =>
    http.get<{ audit: AuditEntry[] }>('/audit', { params }).then((r) => r.data.audit),
  getSettings: () => http.get<{ settings: Settings }>('/settings').then((r) => r.data.settings),
}
