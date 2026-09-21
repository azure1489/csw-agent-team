import { NavLink, Outlet } from 'react-router-dom'

import { useAuth } from '@/lib/auth'
import type { Role } from '@/lib/auth'

/** 十一页。`need` 是最低角色；`van` 只看得到给她的那一页。 */
const NAV: { to: string; label: string; need: Role; vanOnly?: boolean }[] = [
  { to: '/van', label: 'Van 模式', need: 'viewer', vanOnly: true },
  { to: '/', label: '今日总览', need: 'viewer' },
  { to: '/ledger', label: '判断台账', need: 'viewer' },
  { to: '/coverage', label: '采集与覆盖', need: 'viewer' },
  { to: '/pending', label: '待核结转', need: 'viewer' },
  { to: '/kb', label: '知识库', need: 'viewer' },
  { to: '/memory', label: '选题记忆', need: 'viewer' },
  { to: '/rubric', label: '判断框架', need: 'viewer' },
  { to: '/metrics', label: '指标', need: 'viewer' },
  { to: '/settings', label: '运行与设置', need: 'operator' },
]

const RANK: Record<Role, number> = { viewer: 0, van: 0, operator: 1, superadmin: 2 }

export function Shell() {
  const { me, logout } = useAuth()
  const role = me?.role ?? 'viewer'
  // Van 的视图只给 van；其余人看不到那一页（看了也没用，它是手机优先的单页）
  const items = NAV.filter((n) => (n.vanOnly ? role === 'van' : RANK[role] >= RANK[n.need]))

  return (
    <div className="grid min-h-screen grid-cols-1 md:grid-cols-[var(--nav-w)_minmax(0,1fr)]">
      <aside className="flex flex-col border-b border-rule bg-surface md:border-b-0 md:border-r">
        <div className="flex items-center gap-2.5 border-b border-rule px-4 py-3.5">
          <span className="inline-block h-[26px] w-[26px] rounded-[7px] bg-accent" />
          <span>
            <b className="block text-[13.5px] leading-tight">情报收集员工作台</b>
            <span className="text-[11px] text-muted">营事编集室</span>
          </span>
        </div>
        <nav className="flex flex-wrap gap-1 p-2 md:flex-col md:flex-nowrap">
          {items.map((n) => (
            <NavLink
              key={n.to}
              to={n.to}
              end={n.to === '/'}
              className={({ isActive }) =>
                [
                  'rounded-[6px] px-3 py-1.5 text-[13.5px] no-underline',
                  isActive
                    ? n.vanOnly
                      ? 'bg-van-soft font-semibold text-van'
                      : 'bg-accent-soft font-semibold text-accent'
                    : 'text-ink hover:bg-ground',
                ].join(' ')
              }
            >
              {n.label}
            </NavLink>
          ))}
        </nav>
        <div className="mt-auto border-t border-rule px-4 py-3 text-[12px] text-muted">
          <div className="truncate">{me?.display_name || me?.username}</div>
          <div className="mt-0.5">
            {roleLabel(role)}
            {/* van 本质是个只读账号，界面上不藏着 */}
            {role === 'van' && me?.engine_role ? `（引擎里是 ${me.engine_role}）` : null}
          </div>
          <button
            type="button"
            onClick={() => void logout()}
            className="mt-2 rounded-[5px] border border-rule bg-surface px-2 py-1 text-[12px] text-ink hover:bg-ground"
          >
            退出
          </button>
        </div>
      </aside>
      <main className="min-w-0 px-4 py-4 md:px-6 md:py-5">
        <Outlet />
      </main>
    </div>
  )
}

function roleLabel(r: Role): string {
  return { superadmin: '超级管理员', operator: '操作员', viewer: '只读', van: 'Van' }[r]
}
