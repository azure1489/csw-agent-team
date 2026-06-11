// ============================================================
// 应用框架 — Sidebar + Topbar + Content（迁自设计稿 shell.jsx，接 React Router）
// ============================================================
import { useState, type ReactNode } from 'react'
import { Outlet, useLocation, useNavigate } from 'react-router-dom'
import { I } from '@/components/icons'
import { Avatar, Menu, Badge, toast } from '@/components/ui'
import { NAV, ROLE_LABEL } from '@/lib/constants'
import { useAuth } from '@/lib/auth'

function isActive(pathname: string, navPath: string): boolean {
  if (navPath === '/') return pathname === '/'
  return pathname === navPath || pathname.startsWith(navPath + '/')
}

function Sidebar({ collapsed, setCollapsed }: { collapsed: boolean; setCollapsed: (f: (c: boolean) => boolean) => void }) {
  const { user } = useAuth()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const role = user?.role || 'viewer'
  return (
    <aside
      style={{
        width: collapsed ? 64 : 'var(--nav-w)',
        flexShrink: 0,
        height: '100%',
        background: 'var(--sidebar)',
        borderRight: '1px solid var(--border)',
        display: 'flex',
        flexDirection: 'column',
        transition: 'width .18s',
      }}
    >
      {/* brand */}
      <div
        className="row"
        style={{
          height: 'var(--top-h)',
          padding: collapsed ? '0' : '0 16px',
          gap: 10,
          justifyContent: collapsed ? 'center' : 'flex-start',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <div
          style={{
            width: 30,
            height: 30,
            borderRadius: 8,
            background: 'linear-gradient(150deg,#6d6df0,#5151c4)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            flexShrink: 0,
            boxShadow: '0 2px 5px rgba(80,80,180,.3)',
          }}
        >
          <I.flow size={18} style={{ color: '#fff' }} />
        </div>
        {!collapsed && (
          <div className="col" style={{ gap: 0, lineHeight: 1.2 }}>
            <span style={{ fontWeight: 650, fontSize: 13.5, letterSpacing: '-.01em' }}>营事编集室</span>
            <span className="t3" style={{ fontSize: 11 }}>任务流转后台</span>
          </div>
        )}
      </div>

      {/* nav */}
      <nav className="col" style={{ gap: 2, padding: collapsed ? '10px 8px' : '10px 12px', flex: 1, overflowY: 'auto' }}>
        {!collapsed && (
          <div className="t3 xs b" style={{ padding: '6px 8px 4px', letterSpacing: '.04em', textTransform: 'uppercase', fontSize: 10.5 }}>
            导航
          </div>
        )}
        {NAV.map((n) => {
          const visible = n.roles.includes(role)
          const on = isActive(pathname, n.path)
          const locked = !visible
          return (
            <button
              key={n.key}
              disabled={locked}
              onClick={() => !locked && navigate(n.path)}
              title={collapsed ? n.label : undefined}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: collapsed ? '9px' : '7px 9px',
                justifyContent: collapsed ? 'center' : 'flex-start',
                borderRadius: 7,
                border: 'none',
                background: on ? 'var(--accent-weak)' : 'transparent',
                color: locked ? 'var(--text-3)' : on ? 'var(--accent-text)' : 'var(--text-2)',
                fontSize: 13.5,
                fontWeight: on ? 600 : 500,
                opacity: locked ? 0.5 : 1,
                cursor: locked ? 'not-allowed' : 'pointer',
                transition: 'background .12s, color .12s',
                position: 'relative',
              }}
              onMouseEnter={(e) => {
                if (!on && !locked) e.currentTarget.style.background = 'var(--surface-2)'
              }}
              onMouseLeave={(e) => {
                if (!on) e.currentTarget.style.background = 'transparent'
              }}
            >
              {on && !collapsed && (
                <span style={{ position: 'absolute', left: -12, top: 8, bottom: 8, width: 3, borderRadius: 3, background: 'var(--accent)' }} />
              )}
              <n.icon size={18} />
              {!collapsed && (
                <span className="grow" style={{ textAlign: 'left' }}>
                  {n.label}
                </span>
              )}
              {!collapsed && locked && <I.lock size={13} />}
            </button>
          )
        })}
      </nav>

      {/* collapse toggle */}
      <div style={{ padding: collapsed ? 8 : 12, borderTop: '1px solid var(--border)' }}>
        <button
          onClick={() => setCollapsed((c) => !c)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            width: '100%',
            justifyContent: collapsed ? 'center' : 'flex-start',
            padding: '7px 9px',
            borderRadius: 7,
            border: 'none',
            background: 'transparent',
            color: 'var(--text-3)',
            fontSize: 12.5,
          }}
          onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-2)')}
          onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
        >
          <span style={{ transform: collapsed ? 'none' : 'rotate(180deg)', display: 'flex', transition: 'transform .2s' }}>
            <I.chevRight size={16} />
          </span>
          {!collapsed && <span>收起侧栏</span>}
        </button>
      </div>
    </aside>
  )
}

function titleFor(pathname: string): string {
  if (pathname.startsWith('/workflows/') && pathname.endsWith('/edit')) return '工作流编辑器'
  if (/^\/runs\/\d+/.test(pathname)) return '运行详情'
  const n = NAV.find((x) => x.path === pathname)
  return n?.label || '后台'
}

function Topbar() {
  const { user, logout } = useAuth()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const env = import.meta.env.DEV ? 'dev' : 'prod'
  return (
    <header
      className="row between"
      style={{
        height: 'var(--top-h)',
        flexShrink: 0,
        padding: '0 22px',
        borderBottom: '1px solid var(--border)',
        background: 'rgba(255,255,255,.8)',
        backdropFilter: 'blur(8px)',
        position: 'sticky',
        top: 0,
        zIndex: 40,
      }}
    >
      <div className="row gap8">
        <span className="t2" style={{ fontSize: 13.5, fontWeight: 550 }}>
          {titleFor(pathname)}
        </span>
      </div>
      <div className="row gap16">
        <span
          className="row gap6"
          style={{ fontSize: 12, color: 'var(--text-2)', padding: '3px 9px', background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 999 }}
        >
          <span style={{ width: 6, height: 6, borderRadius: 999, background: 'var(--green)' }} />
          环境 · {env}
        </span>
        <Menu
          align="right"
          width={216}
          triggerClassName="hover-row"
          triggerStyle={{ padding: '4px 6px', borderRadius: 8, gap: 8 }}
          trigger={
            <span className="row gap8">
              <Avatar name={user?.display_name || user?.username} size={28} />
              <span className="col" style={{ gap: 0, alignItems: 'flex-start', lineHeight: 1.2 }}>
                <span style={{ fontSize: 13, fontWeight: 600 }}>{user?.display_name || user?.username}</span>
                <span className="t3" style={{ fontSize: 11 }}>{ROLE_LABEL[user?.role || 'viewer']}</span>
              </span>
              <I.chevDown size={15} style={{ color: 'var(--text-3)' }} />
            </span>
          }
          items={[
            { icon: I.user, label: '我的资料', onClick: () => toast('我的资料（演示）') },
            { divider: true },
            {
              icon: I.logout,
              label: '退出登录',
              danger: true,
              onClick: async () => {
                await logout()
                navigate('/login')
              },
            },
          ]}
        />
      </div>
    </header>
  )
}

export function Content({ children, wide }: { children?: ReactNode; wide?: boolean }) {
  return (
    <div style={{ flex: 1, overflowY: 'auto', background: 'var(--bg)' }}>
      <div style={{ maxWidth: wide ? 1440 : 1200, margin: '0 auto', padding: '26px 32px 60px' }}>{children}</div>
    </div>
  )
}

export function Shell() {
  const [collapsed, setCollapsed] = useState(false)
  return (
    <div style={{ display: 'flex', height: '100vh', overflow: 'hidden' }}>
      <Sidebar collapsed={collapsed} setCollapsed={setCollapsed} />
      <div className="col" style={{ flex: 1, minWidth: 0, height: '100%' }}>
        <Topbar />
        <div className="col" style={{ flex: 1, minHeight: 0 }}>
          <Outlet />
        </div>
      </div>
    </div>
  )
}
