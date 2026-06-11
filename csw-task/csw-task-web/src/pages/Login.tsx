import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { I } from '@/components/icons'
import { Btn, Card, Checkbox, Field, TextInput, toast } from '@/components/ui'
import { useAuth } from '@/lib/auth'
import { getApiError } from '@/lib/api'

export function Login() {
  const { login } = useAuth()
  const navigate = useNavigate()
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [remember, setRemember] = useState(true)
  const [err, setErr] = useState('')
  const [loading, setLoading] = useState(false)

  const submit = async () => {
    if (!username || !password) {
      setErr('请输入用户名和密码')
      return
    }
    setErr('')
    setLoading(true)
    try {
      await login(username, password)
      navigate('/')
    } catch (e) {
      setErr(getApiError(e).message)
    } finally {
      setLoading(false)
    }
  }

  return (
    <div style={{ height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'radial-gradient(900px 500px at 50% -10%, #eeeefb, var(--bg))' }}>
      <div className="col" style={{ width: 380, gap: 22, animation: 'popIn .25s' }}>
        <div className="col" style={{ alignItems: 'center', gap: 12 }}>
          <div style={{ width: 46, height: 46, borderRadius: 12, background: 'linear-gradient(150deg,#6d6df0,#5151c4)', display: 'flex', alignItems: 'center', justifyContent: 'center', boxShadow: '0 6px 16px -4px rgba(80,80,180,.5)' }}>
            <I.flow size={26} style={{ color: '#fff' }} />
          </div>
          <div className="col" style={{ alignItems: 'center', gap: 2 }}>
            <div style={{ fontSize: 18, fontWeight: 650 }}>任务流转后台</div>
            <div className="t2 sm">营事编集室 · 工作流控制台</div>
          </div>
        </div>
        <Card style={{ padding: 26 }}>
          <div className="col gap16">
            <Field label="用户名" htmlFor="u">
              <TextInput value={username} onChange={setUsername} placeholder="请输入用户名" icon={I.user} autoFocus onKeyDown={(e) => e.key === 'Enter' && submit()} />
            </Field>
            <Field label="密码" htmlFor="p">
              <TextInput
                value={password}
                onChange={(v) => {
                  setPassword(v)
                  setErr('')
                }}
                type="password"
                placeholder="请输入密码"
                icon={I.lock}
                onKeyDown={(e) => e.key === 'Enter' && submit()}
              />
            </Field>
            <div className="row between">
              <Checkbox checked={remember} onChange={setRemember} label="记住我" />
              <a className="sm" style={{ color: 'var(--accent-text)', fontWeight: 550, cursor: 'pointer' }} onClick={() => toast('请联系超级管理员重置密码', 'warn')}>
                忘记密码？
              </a>
            </div>
            {err && (
              <div className="row gap8" style={{ background: 'var(--red-bg)', border: '1px solid var(--red-bd)', color: 'var(--red)', padding: '8px 11px', borderRadius: 7, fontSize: 12.5 }}>
                <I.xCircle size={15} />
                {err}
              </div>
            )}
            <Btn variant="primary" size="lg" full onClick={submit} disabled={loading}>
              {loading ? '登录中…' : '登 录'}
            </Btn>
          </div>
        </Card>
        <div className="t3 xs" style={{ textAlign: 'center', lineHeight: 1.6 }}>
          使用后台账号登录（JWT，与运行面 agent token 分离）
          <br />
          首个超级管理员由 <span className="mono">adminctl user create</span> 离线引导
        </div>
      </div>
    </div>
  )
}
