import { useState } from 'react'
import { Navigate } from 'react-router-dom'

import { errText } from '@/lib/api'
import { useAuth } from '@/lib/auth'

export function Login() {
  const { login, me } = useAuth()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit(e: React.FormEvent) {
    e.preventDefault()
    setBusy(true)
    setErr('')
    try {
      await login(username, password)
    } catch (e) {
      setErr(errText(e))
    } finally {
      setBusy(false)
    }
  }

  // **登录成功后必须离开这一页。**
  //
  // `login()` 只把 me 放进 context，路由不会自己动——少了这一句，
  // 点完登录接口返回 200、状态也对，但人还停在登录表单上，
  // 看着就像「一闪就没了」，其实是从来没走。
  //
  // 用 Navigate 而不是在 submit 里 navigate：它同时管住「已经登录的人直接
  // 打开 /login」那种情况，否则他们会对着一张永远进不去的表单反复试。
  if (me) return <Navigate to="/" replace />

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <form
        onSubmit={submit}
        className="w-full max-w-[340px] rounded-[var(--r-lg)] border border-rule bg-surface p-6 shadow"
      >
        <h1 className="m-0 text-[17px] font-bold">情报收集员工作台</h1>
        <p className="mb-5 mt-1 text-[12.5px] text-muted">
          用任务流转后台的账号登录。工作台不另设账号。
        </p>

        <label className="block text-[12.5px] text-muted" htmlFor="u">
          用户名
        </label>
        <input
          id="u"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          autoComplete="username"
          className="mb-3 mt-1 w-full rounded-[var(--r)] border border-rule bg-surface-2 px-2.5 py-1.5 text-ink"
        />

        <label className="block text-[12.5px] text-muted" htmlFor="p">
          口令
        </label>
        <input
          id="p"
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          autoComplete="current-password"
          className="mb-4 mt-1 w-full rounded-[var(--r)] border border-rule bg-surface-2 px-2.5 py-1.5 text-ink"
        />

        {err ? (
          <p role="alert" className="mb-3 mt-0 rounded-[var(--r)] bg-bad-soft px-2.5 py-1.5 text-[12.5px] text-bad">
            {err}
          </p>
        ) : null}

        <button
          type="submit"
          disabled={busy || !username || !password}
          className="w-full rounded-[var(--r)] bg-accent px-3 py-2 font-medium text-white disabled:opacity-50"
        >
          {busy ? '登录中…' : '登录'}
        </button>
      </form>
    </div>
  )
}
