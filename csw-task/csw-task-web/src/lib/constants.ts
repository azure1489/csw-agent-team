// ============================================================
// 业务常量 — 状态/色调/导航/标签映射（迁自设计稿 ui.jsx + shell.jsx）
// ============================================================
import { I, type IconComponent } from '@/components/icons'

export type Tone = 'green' | 'blue' | 'amber' | 'red' | 'gray' | 'yellow' | 'violet'

export const TONES: Record<Tone, { c: string; bg: string; bd: string }> = {
  green: { c: 'var(--green)', bg: 'var(--green-bg)', bd: 'var(--green-bd)' },
  blue: { c: 'var(--blue)', bg: 'var(--blue-bg)', bd: 'var(--blue-bd)' },
  amber: { c: 'var(--amber)', bg: 'var(--amber-bg)', bd: 'var(--amber-bd)' },
  red: { c: 'var(--red)', bg: 'var(--red-bg)', bd: 'var(--red-bd)' },
  gray: { c: 'var(--gray)', bg: 'var(--gray-bg)', bd: 'var(--gray-bd)' },
  yellow: { c: 'var(--yellow)', bg: 'var(--yellow-bg)', bd: 'var(--yellow-bd)' },
  violet: { c: 'var(--accent-text)', bg: 'var(--accent-weak)', bd: 'var(--accent-weak-2)' },
}

export interface StatusMeta {
  label: string
  tone: Tone
  dot?: boolean
  outline?: boolean
}

// 业务状态 → 标签 + 色调（含 workflow / run / task / token / agent / user）
export const STATUS: Record<string, StatusMeta> = {
  // workflow
  draft: { label: '草稿', tone: 'gray', dot: true },
  active: { label: '已激活', tone: 'green', dot: true },
  archived: { label: '已归档', tone: 'gray', outline: true },
  // run
  run_active: { label: '进行中', tone: 'blue', dot: true },
  done: { label: '已完成', tone: 'green', dot: true },
  paused: { label: '已暂停', tone: 'yellow', dot: true },
  aborted: { label: '已中止', tone: 'red', dot: true },
  // task
  blocked: { label: '未就绪', tone: 'gray' },
  ready: { label: '待派工', tone: 'blue' },
  dispatched: { label: '已派工', tone: 'blue' },
  in_progress: { label: '进行中', tone: 'blue' },
  review: { label: '审核中', tone: 'amber' },
  passed: { label: '已通过', tone: 'green' },
  returned: { label: '已退回', tone: 'red' },
  // token / agent / user
  token_active: { label: '启用', tone: 'green', dot: true },
  revoked: { label: '已吊销', tone: 'gray' },
  expired: { label: '已过期', tone: 'yellow' },
  enabled: { label: '启用', tone: 'green', dot: true },
  disabled: { label: '禁用', tone: 'gray', dot: true },
}

export type AdminRole = 'superadmin' | 'operator' | 'viewer'

export const ROLE_LABEL: Record<string, string> = {
  superadmin: '超级管理员',
  operator: '运营',
  viewer: '访客（只读）',
}

export interface NavItem {
  key: string
  label: string
  icon: IconComponent
  roles: AdminRole[]
  badge?: string
  path: string
}

// 侧栏导航 + RBAC 可见性 + 路由
export const NAV: NavItem[] = [
  { key: 'dashboard', label: '仪表盘', icon: I.dashboard, roles: ['superadmin', 'operator', 'viewer'], path: '/' },
  { key: 'workflows', label: '工作流', icon: I.workflow, roles: ['superadmin', 'operator', 'viewer'], path: '/workflows' },
  { key: 'members', label: '角色与成员', icon: I.members, roles: ['superadmin', 'operator', 'viewer'], path: '/members' },
  { key: 'rosters', label: '通讯录', icon: I.building, roles: ['superadmin', 'operator', 'viewer'], path: '/rosters' },
  { key: 'admin-users', label: '后台用户', icon: I.adminUser, roles: ['superadmin'], badge: 'superadmin', path: '/admin-users' },
  { key: 'runs', label: '运行监控', icon: I.runs, roles: ['superadmin', 'operator', 'viewer'], path: '/runs' },
  { key: 'audit', label: '审计日志', icon: I.audit, roles: ['superadmin', 'operator', 'viewer'], path: '/audit' },
  { key: 'settings', label: '设置', icon: I.settings, roles: ['superadmin'], badge: 'superadmin', path: '/settings' },
]

// 审计动作 → 标签 + 色调（覆盖后端实际写入的 action）
export const ACTION_META: Record<string, { label: string; tone: Tone }> = {
  login: { label: '登录', tone: 'gray' },
  logout: { label: '登出', tone: 'gray' },
  workflow_create: { label: '创建工作流', tone: 'violet' },
  workflow_update: { label: '编辑工作流', tone: 'violet' },
  workflow_validate: { label: '校验工作流', tone: 'blue' },
  workflow_activate: { label: '激活工作流', tone: 'green' },
  workflow_archive: { label: '归档工作流', tone: 'gray' },
  workflow_clone: { label: '复制工作流', tone: 'violet' },
  role_create: { label: '创建角色', tone: 'blue' },
  role_update: { label: '修改角色', tone: 'violet' },
  agent_create: { label: '创建成员', tone: 'blue' },
  agent_update: { label: '修改成员', tone: 'violet' },
  token_issue: { label: '签发 token', tone: 'blue' },
  token_revoke: { label: '吊销 token', tone: 'amber' },
  user_create: { label: '创建用户', tone: 'blue' },
  user_update: { label: '修改用户', tone: 'violet' },
  user_reset_password: { label: '重置密码', tone: 'amber' },
}

// 时间线事件 → 圆点颜色
export const EVENT_TONE: Record<string, string> = {
  run_created: 'var(--accent)',
  task_ready: 'var(--gray)',
  dispatched: 'var(--blue)',
  file_uploaded: 'var(--gray)',
  submitted: 'var(--blue)',
  gate_passed: 'var(--green)',
  stage_passed: 'var(--green)',
  gate_returned: 'var(--red)',
  run_done: 'var(--green)',
  authorization_required: 'var(--amber)',
  authorization_granted: 'var(--green)',
  authorization_revoked: 'var(--amber)',
}

// 事件类型 → 人类可读模板（缺省回退到原始 type）
export const EVENT_LABEL: Record<string, string> = {
  run_created: '实例创建',
  task_ready: '任务就绪',
  dispatched: '任务派工',
  file_uploaded: '文件上传',
  submitted: '产出提交',
  gate_passed: '闸通过',
  stage_passed: '阶段通过',
  gate_returned: '闸退回',
  run_done: '实例完成',
  authorization_required: '等待授权',
  authorization_granted: '授权录入',
  authorization_revoked: '授权撤销',
}

// run 授权范围 → 中文
export const SCOPE_LABEL: Record<string, string> = {
  local_drill: '本地演练',
  wx_draft: '公众号草稿',
  wx_publish: '公众号发布',
  xhs_draft: '小红书草稿',
  xhs_publish: '小红书发布',
}

// 平台写动作可被哪些授权满足（发布授权涵盖同平台草稿）
export const SCOPE_SATISFIED_BY: Record<string, string[]> = {
  wx_draft: ['wx_draft', 'wx_publish'],
  xhs_draft: ['xhs_draft', 'xhs_publish'],
  wx_publish: ['wx_publish'],
  xhs_publish: ['xhs_publish'],
}

// 阶段产出类型候选（StageDrawer / NewWorkflowModal）
export const OUTPUT_OPTS: string[] = [
  '资讯包',
  '选题单',
  '文章',
  '封面图',
  '成品包',
  '发布回执',
  '笔记文本',
  '图卡',
  '其它',
]
