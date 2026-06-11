import { type ReactNode } from 'react'
import { createBrowserRouter, Navigate, Outlet } from 'react-router-dom'
import { Shell } from '@/components/shell/Shell'
import { useAuth } from '@/lib/auth'
import type { AdminRole } from '@/types'
import { Login } from '@/pages/Login'
import { Dashboard } from '@/pages/Dashboard'
import { Workflows } from '@/pages/Workflows'
import { WorkflowEditor } from '@/pages/WorkflowEditor'
import { Members } from '@/pages/Members'
import { AdminUsers } from '@/pages/AdminUsers'
import { Runs } from '@/pages/Runs'
import { RunDetailPage } from '@/pages/RunDetail'
import { Audit } from '@/pages/Audit'
import { Settings } from '@/pages/Settings'

function FullLoader() {
  return (
    <div className="row center" style={{ height: '100vh', color: 'var(--text-3)', fontSize: 13.5 }}>
      加载中…
    </div>
  )
}

function LoginRoute() {
  const { user, loading } = useAuth()
  if (loading) return <FullLoader />
  if (user) return <Navigate to="/" replace />
  return <Login />
}

function ProtectedLayout() {
  const { user, loading } = useAuth()
  if (loading) return <FullLoader />
  if (!user) return <Navigate to="/login" replace />
  return <Shell />
}

function RequireRole({ roles, children }: { roles: AdminRole[]; children: ReactNode }) {
  const { user } = useAuth()
  if (!user || !roles.includes(user.role)) return <Navigate to="/" replace />
  return <>{children}</>
}

export const router = createBrowserRouter([
  { path: '/login', element: <LoginRoute /> },
  {
    element: <ProtectedLayout />,
    children: [
      { path: '/', element: <Dashboard /> },
      { path: '/workflows', element: <Workflows /> },
      { path: '/workflows/:id/edit', element: <WorkflowEditor /> },
      { path: '/members', element: <Members /> },
      {
        path: '/admin-users',
        element: (
          <RequireRole roles={['superadmin']}>
            <AdminUsers />
          </RequireRole>
        ),
      },
      { path: '/runs', element: <Runs /> },
      { path: '/runs/:id', element: <RunDetailPage /> },
      { path: '/audit', element: <Audit /> },
      {
        path: '/settings',
        element: (
          <RequireRole roles={['superadmin']}>
            <Settings />
          </RequireRole>
        ),
      },
      { path: '*', element: <Navigate to="/" replace /> },
    ],
  },
])

// 占位以满足某些 lint 规则下的 Outlet 引用（实际由 Shell 渲染）
export const _Outlet = Outlet
