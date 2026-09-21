import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'

import { api, setCsrf, setUnauthorizedHandler } from './api'

/**
 * 工作台角色。`van` 是引擎里没有的——它是配置里列出的 viewer 映射出来的，
 * `engineRole` 仍然是 `viewer`，界面上要能看出这一点。
 */
export type Role = 'superadmin' | 'operator' | 'viewer' | 'van'

export interface Me {
  username: string
  display_name: string
  role: Role
  engine_role: string
  csrf_token: string
}

interface AuthState {
  me: Me | null
  /** 首次探测还没回来时是 true：这期间不要跳登录，否则刷新页面会闪一下 */
  loading: boolean
  login: (username: string, password: string) => Promise<void>
  logout: () => Promise<void>
}

const Ctx = createContext<AuthState | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null)
  const [loading, setLoading] = useState(true)

  const adopt = useCallback((m: Me) => {
    setMe(m)
    setCsrf(m.csrf_token)
  }, [])

  useEffect(() => {
    setUnauthorizedHandler(() => {
      setMe(null)
      setCsrf('')
    })
    // 带着 cookie 问一次「我是谁」。角色以引擎当下的回答为准——
    // 后台里改了角色或停用，刷新一下就生效。
    api
      .get<Me>('/auth/me')
      .then((r) => adopt(r.data))
      .catch(() => setMe(null))
      .finally(() => setLoading(false))
  }, [adopt])

  const login = useCallback(
    async (username: string, password: string) => {
      const r = await api.post<Me>('/auth/login', { username, password })
      adopt(r.data)
    },
    [adopt],
  )

  const logout = useCallback(async () => {
    try {
      await api.post('/auth/logout')
    } finally {
      setMe(null)
      setCsrf('')
    }
  }, [])

  const value = useMemo(() => ({ me, loading, login, logout }), [me, loading, login, logout])
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useAuth(): AuthState {
  const v = useContext(Ctx)
  if (!v) throw new Error('useAuth 必须在 AuthProvider 里用')
  return v
}

const RANK: Record<Role, number> = { viewer: 0, van: 0, operator: 1, superadmin: 2 }

/**
 * 有没有到这个角色档位。
 *
 * `van` 与 `viewer` 同档：Van 在工作台上只读加勾选，**勾选只写本地、不回写引擎**，
 * 进不进评选由主编代录。给她 operator 权限既没必要，也会让「谁改了什么」变糊。
 */
export function atLeast(role: Role | undefined, need: Role): boolean {
  if (!role) return false
  return RANK[role] >= RANK[need]
}
