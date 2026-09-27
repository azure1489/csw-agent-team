/**
 * 运行与设置：**服务在不在、配置是什么、手里有哪些活、谁改过什么。**
 *
 * 两条：
 * - **密钥只显示「已配置 / 缺」。** 返回值等于把密钥送到浏览器里。
 * - **手动开轮与重跑只排队。** 一轮四十分钟，请求等不了；排队那一行就是
 *   「排到哪了 / 为什么没成」的唯一来源。
 */
import { useState } from 'react'

import { Empty, ErrorBox, H2, Head, Loading, StepBadge, Table, bj, roundLabel } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useAudit, useHealth, useOpenRound, useRerun, useRounds, useSettings, useWork } from '@/lib/queries'

export function Settings() {
  const settings = useSettings()
  const health = useHealth()
  const work = useWork()
  const audit = useAudit()
  const rounds = useRounds({ limit: 20 })
  const openRound = useOpenRound()
  const rerun = useRerun()
  const [days, setDays] = useState(1)
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [rerunRound, setRerunRound] = useState<number | ''>('')
  const [rerunStep, setRerunStep] = useState<'harvest' | 'judge'>('judge')

  const s = settings.data
  return (
    <section>
      <Head title="运行与设置" desc="服务状态、配置、排队的活与留痕。" />

      <H2>服务状态</H2>
      {health.isLoading && <Loading what="状态" />}
      {health.data && (
        <div className="space-y-1.5">
          {health.data.deps.map((d) => (
            <div
              key={d.name}
              className="flex items-center gap-2 rounded-[var(--r)] border border-rule bg-surface px-3 py-2"
            >
              <span
                className={`inline-block h-2 w-2 rounded-full ${d.ok ? 'bg-ok' : 'bg-warn'}`}
              />
              <span className="text-[13px] font-medium">{d.name}</span>
              <span className="text-[12.5px] text-muted">{d.note}</span>
            </div>
          ))}
          <div className="text-[12.5px] text-muted">
            起来了 {Math.round(health.data.uptime_secs / 60)} 分钟
            {health.data.outbox_conflicts > 0 && (
              <b className="ml-2 text-bad">
                有 {health.data.outbox_conflicts} 条写引擎的记录卡在冲突上，要人核实
              </b>
            )}
          </div>
        </div>
      )}

      <H2>手动开一轮</H2>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[13px] text-muted">往回取</span>
        <input
          type="number"
          min={1}
          max={30}
          value={days}
          onChange={(e) => setDays(Number(e.target.value))}
          className="w-[64px] rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
        />
        <span className="text-[13px] text-muted">天</span>
        <button
          onClick={() => openRound.mutate({ days })}
          disabled={openRound.isPending}
          className="rounded-[var(--r-sm)] bg-accent px-3 py-1 text-[13px] text-white disabled:opacity-50"
        >
          排进队
        </button>
        <span className="text-[12.5px] text-muted">
          手动轮不写引擎，是拿来看的，不是拿来交的。
        </span>
      </div>

      {/* 按天开轮最少也是一整天（约三百多条候选、一个多小时、一笔模型钱）。
          先拿一个小窗口验证链路时，按天是不够用的——所以把接口本来就支持的
          自定义窗口露出来。 */}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <span className="text-[13px] text-muted">或指定窗口</span>
        <input
          type="datetime-local"
          value={from}
          onChange={(e) => setFrom(e.target.value)}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
        />
        <span className="text-[13px] text-muted">到</span>
        <input
          type="datetime-local"
          value={to}
          onChange={(e) => setTo(e.target.value)}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
        />
        <button
          onClick={() =>
            openRound.mutate({
              // 输入框给的是本地时间，接口要 RFC3339 UTC。
              // 去掉毫秒：库里存的窗口是拿来比对与展示的，毫秒只会让它更难读。
              window_start: new Date(from).toISOString().replace(/\.\d{3}Z$/, 'Z'),
              window_end: new Date(to).toISOString().replace(/\.\d{3}Z$/, 'Z'),
            })
          }
          disabled={openRound.isPending || !from || !to}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-3 py-1 text-[13px] disabled:opacity-50"
        >
          排进队
        </button>
        <span className="text-[12.5px] text-muted">
          窗口按**首次入库时间**算，不是发布时间。试链路用两三个小时就够。
        </span>
      </div>
      {openRound.error && <div className="mt-2"><ErrorBox error={openRound.error} /></div>}

      <H2>重跑一步</H2>
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={rerunRound}
          onChange={(e) => setRerunRound(Number(e.target.value) || '')}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
        >
          <option value="">挑一轮</option>
          {(rounds.data ?? []).map((r) => (
            <option key={r.id} value={r.id}>
              {roundLabel(r)}
            </option>
          ))}
        </select>
        <select
          value={rerunStep}
          onChange={(e) => setRerunStep(e.target.value as 'harvest' | 'judge')}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]"
        >
          <option value="judge">只重判</option>
          <option value="harvest">连识别一起重来</option>
        </select>
        <button
          disabled={!rerunRound || rerun.isPending}
          onClick={() => rerunRound && rerun.mutate({ roundId: rerunRound, step: rerunStep })}
          className="rounded-[var(--r-sm)] bg-accent px-3 py-1 text-[13px] text-white disabled:opacity-50"
        >
          排进队
        </button>
      </div>
      <p className="mt-1 text-[12.5px] text-muted">
        <b>登记与提交不动。</b>引擎那边已经收到的台账要改，只能走补件——那是人的决定。
      </p>
      {rerun.error && <div className="mt-2"><ErrorBox error={rerun.error} /></div>}

      <H2>排队的活</H2>
      {work.data?.length === 0 && <Empty>队里没有活。</Empty>}
      {work.data && work.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>活</Th>
              <Th>谁排的</Th>
              <Th>状态</Th>
              <Th>结果</Th>
              <Th>排队时间</Th>
            </tr>
          }
        >
          {work.data.map((w) => (
            <tr key={w.id} className="border-t border-rule">
              <Td>{w.kind === 'manual_round' ? '手动开轮' : `重跑第 ${w.round_id} 轮`}</Td>
              <Td>{w.actor}</Td>
              <Td>
                <StepBadge status={w.status === 'done' ? 'succeeded' : w.status} sm />
              </Td>
              <Td style={{ color: w.status === 'failed' ? 'var(--bad)' : undefined }}>
                {w.note || '——'}
              </Td>
              <Td className="tnum">{bj(w.created_at)}</Td>
            </tr>
          ))}
        </Table>
      )}

      <H2>配置</H2>
      {settings.error && <ErrorBox error={settings.error} />}
      {s && (
        <>
          <Table
            head={
              <tr>
                <Th>项</Th>
                <Th>值</Th>
              </tr>
            }
          >
            {[
              ['引擎', s.engine_base_url],
              ['csw', s.csw_base_url],
              ['向量服务', s.vector_base_url],
              ['生成模型', `${s.model}${s.fallback_model ? ` → ${s.fallback_model}` : ''}`],
              ['向量模型', s.embed_model],
              ['Jev', s.jev_enabled ? '开' : '关'],
            ].map(([k, v]) => (
              <tr key={k} className="border-t border-rule">
                <Td>{k}</Td>
                <Td className="mono text-[12.5px]">{v}</Td>
              </tr>
            ))}
          </Table>

          <H2>密钥</H2>
          <div className="flex flex-wrap gap-2">
            {s.secrets.map(([name, ok]) => (
              <span
                key={name}
                className={[
                  'rounded-[var(--r-sm)] border px-2.5 py-1 text-[12.5px]',
                  ok ? 'border-ok-soft bg-ok-soft text-ok' : 'border-warn-soft bg-warn-soft text-warn',
                ].join(' ')}
              >
                {name} {ok ? '已配置' : '缺'}
              </span>
            ))}
          </div>
          <p className="mt-1 text-[12.5px] text-muted">
            只显示「已配置 / 缺」——返回值等于把密钥送到浏览器里。
          </p>

          <H2>高风险开关</H2>
          <div className="flex flex-wrap gap-2">
            {Object.entries(s.features).map(([k, on]) => (
              <span
                key={k}
                className={[
                  'rounded-[var(--r-sm)] border px-2.5 py-1 text-[12.5px]',
                  on ? 'border-warn-soft bg-warn-soft text-warn' : 'border-rule bg-surface text-muted',
                ].join(' ')}
              >
                {FEATURE_CN[k] ?? k} {on ? '开着' : '关着'}
              </span>
            ))}
          </div>
        </>
      )}

      <H2>留痕</H2>
      {audit.data?.length === 0 && <Empty>还没有人改过什么。</Empty>}
      {audit.data && audit.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>谁</Th>
              <Th>做了什么</Th>
              <Th>对谁</Th>
              <Th>时间</Th>
            </tr>
          }
        >
          {audit.data.map((a) => (
            <tr key={a.id} className="border-t border-rule">
              <Td>{a.actor}</Td>
              <Td>
                {ACTION_CN[a.action] ?? a.action}
                <div className="text-[11.5px] text-muted">{a.detail_json}</div>
              </Td>
              <Td className="mono text-[12px]">{a.target || '——'}</Td>
              <Td className="tnum">{bj(a.created_at, true)}</Td>
            </tr>
          ))}
        </Table>
      )}
    </section>
  )
}

const FEATURE_CN: Record<string, string> = {
  xhs_collector: '小红书采集器',
  web_collector: '网页 / RSS 采集器',
  custom_collectors: '自定义采集器',
  mcp_for_hermes: '把 MCP 挂给 Hermes',
  deepcheck_web_tools: '深核用网页工具',
}

const ACTION_CN: Record<string, string> = {
  override: '改档',
  first_batch: '指定首批',
  van_mark: 'Van 勾选',
  van_mark_remove: '撤掉勾选',
  manual_round: '手动开轮',
  rerun: '重跑',
}
