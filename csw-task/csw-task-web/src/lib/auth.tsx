// ============================================================
// 鉴权 Context — access token 存内存；启动静默 refresh；401 失败回调清态
// ============================================================
import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import { api, setAccessToken, setOnAuthFailed } from './api'
import type { AdminUser } from '@/types'

interface AuthCtxValue {
  user: AdminUser | null
  loading: boolean
  login: (username: string, password: string) => Promise<void>
  logout: () => Promise<void>
}

const AuthCtx = createContext<AuthCtxValue | null>(null)

export function useAuth(): AuthCtxValue {
  const ctx = useContext(AuthCtx)
  if (!ctx) throw new Error('useAuth 必须在 AuthProvider 内使用')
  return ctx
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<AdminUser | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    setOnAuthFailed(() => {
      setAccessToken(null)
      setUser(null)
    })
    // 启动：尝试用 refresh cookie 静默续期
    api
      .refresh()
      .then((r) => {
        setAccessToken(r.access_token)
        setUser(r.user)
      })
      .catch(() => {
        /* 未登录，留在登录页 */
      })
      .finally(() => setLoading(false))
  }, [])

  const login = async (username: string, password: string) => {
    const r = await api.login(username, password)
    setAccessToken(r.access_token)
    setUser(r.user)
  }

  const logout = async () => {
    try {
      await api.logout()
    } catch {
      /* 忽略 */
    }
    setAccessToken(null)
    setUser(null)
  }

  return <AuthCtx.Provider value={{ user, loading, login, logout }}>{children}</AuthCtx.Provider>
}
