import { useMemo } from 'react'
import { I, type IconComponent } from '@/components/icons'
import { Btn, Modal } from '@/components/ui'
import { useRoles } from '@/hooks/useRoles'
import { runChecks } from './lib'
import type { EditorWorkflow } from '@/types'

export function ValidateModal({
  open,
  onClose,
  draft,
  onActivate,
  gotoStage,
  activating,
}: {
  open: boolean
  onClose: () => void
  draft: EditorWorkflow
  onActivate: () => void
  gotoStage: (id: number) => void
  activating?: boolean
}) {
  const { data: roles } = useRoles()
  const checks = useMemo(() => (open ? runChecks(draft, roles || []) : []), [open, draft, roles])
  const fails = checks.filter((c) => c.status === 'fail').length
  const warns = checks.filter((c) => c.status === 'warn').length
  const icon: Record<string, IconComponent> = { pass: I.checkCircle, warn: I.warn, fail: I.xCircle }
  const color: Record<string, string> = { pass: 'var(--green)', warn: 'var(--amber)', fail: 'var(--red)' }
  const bg: Record<string, string> = { pass: 'transparent', warn: 'var(--amber-bg)', fail: 'var(--red-bg)' }

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={540}
      icon={fails ? I.xCircle : I.checkCircle}
      iconTone={fails ? 'red' : 'green'}
      title={`校验：${draft.name} v${draft.version}`}
      subtitle={fails ? `${fails} 项未通过，修正后才能激活` : warns ? `全部通过，${warns} 条提示` : '全部检查通过，可以激活'}
      footer={
        <>
          <span className="t3 sm">激活后同 key 的旧 active 版本将自动归档</span>
          <div className="row gap8">
            <Btn onClick={onClose}>取消</Btn>
            <Btn variant="primary" icon={I.bolt} disabled={fails > 0 || activating} onClick={onActivate}>
              确认激活（旧版归档）
            </Btn>
          </div>
        </>
      }
    >
      <div className="col gap6" style={{ padding: '4px 0 14px' }}>
        {checks.map((c, i) => {
          const IconC = icon[c.status]
          return (
            <div
              key={i}
              className="row gap10"
              onClick={() => c.stageId && gotoStage(c.stageId)}
              style={{ padding: '9px 11px', borderRadius: 8, background: bg[c.status], cursor: c.stageId ? 'pointer' : 'default', alignItems: 'flex-start' }}
            >
              <span style={{ color: color[c.status], display: 'flex', marginTop: 1 }}>
                <IconC size={17} />
              </span>
              <span className="grow" style={{ fontSize: 13, lineHeight: 1.5 }}>
                {c.text}
              </span>
              {c.stageId && (
                <span className="row gap4 xs" style={{ color: 'var(--accent-text)', fontWeight: 550, whiteSpace: 'nowrap' }}>
                  定位
                  <I.chevRight size={12} />
                </span>
              )}
            </div>
          )
        })}
      </div>
    </Modal>
  )
}
