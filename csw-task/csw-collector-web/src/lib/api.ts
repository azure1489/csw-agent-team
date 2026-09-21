import axios, { AxiosError } from 'axios'

/**
 * 与 csw-task-web 最大的不同：**这里没有 token。**
 *
 * 引擎的 access 与 refresh 只留在采集服务的服务端会话里，浏览器拿到的只有一枚
 * 不透明的 HttpOnly cookie。所以前端不做续期、不存 token、也读不到它——
 * 最坏情况（XSS）丢的只是一个我们随时能作废的会话 id，不是引擎的管理身份。
 *
 * 前端要做的只有两件事：
 * 1. 带上 cookie（`withCredentials`）。
 * 2. 写请求回填 CSRF 头——`SameSite=Lax` 挡不住表单式的跨站 POST。
 */
export const api = axios.create({
  baseURL: '/api',
  withCredentials: true,
  timeout: 30_000,
})

let csrfToken = ''

/** 登录或 /me 返回后记下 CSRF；登出时清掉。 */
export function setCsrf(token: string) {
  csrfToken = token
}

api.interceptors.request.use((config) => {
  const method = (config.method ?? 'get').toLowerCase()
  if (method !== 'get' && method !== 'head' && csrfToken) {
    config.headers.set('X-CSW-CSRF', csrfToken)
  }
  return config
})

/** 会话失效时通知上层跳登录。用回调而不是直接跳，免得在非路由环境里也硬跳。 */
let onUnauthorized: (() => void) | null = null
export function setUnauthorizedHandler(fn: () => void) {
  onUnauthorized = fn
}

api.interceptors.response.use(
  (r) => r,
  (error: AxiosError) => {
    if (error.response?.status === 401) {
      csrfToken = ''
      onUnauthorized?.()
    }
    return Promise.reject(error)
  },
)

/** 后端的错误体是 `{ error: "一句人话" }`。取不到就退回通用文案，不把栈抛给用户。 */
export function errText(e: unknown): string {
  const err = e as AxiosError<{ error?: string }>
  return err.response?.data?.error ?? err.message ?? '请求失败'
}
