import { Navigate, createBrowserRouter } from 'react-router-dom'

import { Shell } from '@/components/shell/Shell'
import { useAuth } from '@/lib/auth'
import type { Role } from '@/lib/auth'
import { atLeast } from '@/lib/auth'
import { Login } from '@/pages/Login'
import {
  Coverage,
  Kb,
  Ledger,
  Memory,
  Metrics,
  Overview,
  Pending,
  Rubric,
  Settings,
  Van,
} from '@/pages/placeholders'

/**
 * 守卫。
 *
 * 首次探测没回来之前**什么都不跳**——否则刷新页面会先闪一下登录页。
 * 前端守卫只是体验，真正的权限在服务端：每个写接口都自己查会话与角色。
 */
function Guard({ need, children }: { need: Role; children: React.ReactNode }) {
  const { me, loading } = useAuth()
  if (loading) return <div className="p-6 text-muted">…</div>
  if (!me) return <Navigate to="/login" replace />
  if (!atLeast(me.role, need)) return <Navigate to="/" replace />
  return <>{children}</>
}

/** Van 登录后直接进她那一页，不必先看一眼不属于她的总览。 */
function Home() {
  const { me, loading } = useAuth()
  if (loading) return <div className="p-6 text-muted">…</div>
  if (me?.role === 'van') return <Navigate to="/van" replace />
  return <Overview />
}

export const router = createBrowserRouter([
  { path: '/login', element: <Login /> },
  {
    path: '/',
    element: (
      <Guard need="viewer">
        <Shell />
      </Guard>
    ),
    children: [
      { index: true, element: <Home /> },
      { path: 'van', element: <Van /> },
      { path: 'ledger', element: <Ledger /> },
      { path: 'coverage', element: <Coverage /> },
      { path: 'pending', element: <Pending /> },
      { path: 'kb', element: <Kb /> },
      { path: 'memory', element: <Memory /> },
      { path: 'rubric', element: <Rubric /> },
      { path: 'metrics', element: <Metrics /> },
      {
        path: 'settings',
        element: (
          <Guard need="operator">
            <Settings />
          </Guard>
        ),
      },
      { path: '*', element: <Navigate to="/" replace /> },
    ],
  },
])
