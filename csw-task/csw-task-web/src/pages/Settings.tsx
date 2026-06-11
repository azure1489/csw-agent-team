import type { ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { I } from '@/components/icons'
import { Badge, Card, PageHeader, SectionCard } from '@/components/ui'
import { api } from '@/lib/api'

function SettingRow({ label, hint, children, last }: { label: ReactNode; hint?: ReactNode; children?: ReactNode; last?: boolean }) {
  return (
    <div className="row between" style={{ padding: '13px 0', borderBottom: last ? 'none' : '1px solid var(--border)', gap: 20, alignItems: 'center' }}>
      <div className="col" style={{ gap: 2, maxWidth: 460 }}>
        <span style={{ fontSize: 13.5, fontWeight: 550 }}>{label}</span>
        {hint && (
          <span className="t3 xs" style={{ lineHeight: 1.5 }}>
            {hint}
          </span>
        )}
      </div>
      <div style={{ flexShrink: 0 }}>{children}</div>
    </div>
  )
}

const chip = { background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 6, padding: '4px 10px', fontSize: 12.5 } as const

export function Settings() {
  const { data: s } = useQuery({ queryKey: ['settings'], queryFn: api.getSettings })

  const accessMin = s ? Math.round(s.jwt.access_ttl_seconds / 60) : 0
  const refreshDay = s ? Math.round(s.jwt.refresh_ttl_seconds / 86400) : 0
  const maxMB = s ? Math.round(s.files.max_upload_bytes / 1048576) : 0

  return (
    <div style={{ flex: 1, overflowY: 'auto' }}>
      <div style={{ maxWidth: 1200, margin: '0 auto', padding: '26px 32px 60px' }}>
        <PageHeader
          title="设置"
          desc="系统级配置，由环境变量驱动（CSW_*），此处为只读展示。修改请调整 env 后重启服务。仅超级管理员可访问。"
          badge={
            <Badge tone="violet" sm>
              <I.lock size={11} />
              superadmin
            </Badge>
          }
        />
        <div className="col gap16">
          <SectionCard title="JWT 会话" subtitle="access 无状态短期，refresh 存哈希、支持轮换 / 吊销">
            <SettingRow label="Access Token TTL" hint="无状态短期令牌，每请求验签不查库">
              <span className="mono t2" style={chip}>
                {accessMin} min
              </span>
            </SettingRow>
            <SettingRow label="Refresh Token TTL" hint="httpOnly cookie，刷新时轮换">
              <span className="mono t2" style={chip}>
                {refreshDay} d
              </span>
            </SettingRow>
            <SettingRow label="JWT 密钥" hint="CSW_JWT_SECRET；未设则启动随机生成（仅 dev）" last>
              <Badge tone={s?.jwt.secret_generated ? 'amber' : 'green'} sm>
                {s?.jwt.secret_generated ? '运行时生成（临时）' : 'env 已设'}
              </Badge>
            </SettingRow>
          </SectionCard>

          <SectionCard title="文件存储" subtitle="运行面上传 / 内容寻址">
            <SettingRow label="单文件大小上限" hint="超过将拒绝上传">
              <span className="mono t2" style={chip}>
                {maxMB} MB
              </span>
            </SettingRow>
            <SettingRow label="允许类型" hint="content_type 白名单">
              <div className="row gap6 wrap" style={{ justifyContent: 'flex-end' }}>
                {(s?.files.allowed_content_types || []).map((t) => (
                  <Badge key={t} tone="gray" sm>
                    {t.replace('application/', '')}
                  </Badge>
                ))}
              </div>
            </SettingRow>
            <SettingRow label="存储后端" hint="内容寻址：data/blobs/<分片>/<sha256>；BlobStore 接口预留 OSS" last>
              <span className="mono t2" style={chip}>
                {s?.files.storage || 'local'}
              </span>
            </SettingRow>
          </SectionCard>

          <SectionCard title="跨域 (CORS)" subtitle="前端 origin 白名单（CSW_ADMIN_CORS_ORIGIN）">
            <SettingRow label="允许的 origin" last>
              <div className="row gap6 wrap" style={{ justifyContent: 'flex-end' }}>
                {(s?.cors_origins || []).map((o) => (
                  <span key={o} className="mono t2" style={chip}>
                    {o}
                  </span>
                ))}
              </div>
            </SettingRow>
          </SectionCard>

          <SectionCard title="关于" subtitle="版本与健康检查">
            <SettingRow label="服务版本">
              <span className="mono t2 sm">csw-task-svc v{s?.version || '—'}</span>
            </SettingRow>
            <SettingRow label="数据库">
              <span className="mono t2 sm">SQLite · WAL · {s?.db_path || 'data/csw-task.db'}</span>
            </SettingRow>
            <SettingRow label="运行面 BASE_URL">
              <span className="mono t2 sm">{s?.base_url || '—'}</span>
            </SettingRow>
            <SettingRow label="健康检查" last>
              <span className="row gap6">
                <span style={{ width: 8, height: 8, borderRadius: 999, background: s ? 'var(--green)' : 'var(--gray)' }} />
                <span className="sm" style={{ color: s ? 'var(--green)' : 'var(--text-3)' }}>
                  {s ? '后台 API 正常' : '加载中…'}
                </span>
              </span>
            </SettingRow>
          </SectionCard>
        </div>
      </div>
    </div>
  )
}
