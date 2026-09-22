/**
 * 事件详情：**这一条的全貌。**
 *
 * 一次拿全（候选原文、每张图与描述、判断、改档历史、Van 勾选、深核、模型调用）——
 * 分七个接口去拿，这一页上就是七个各自转圈的小方块。
 *
 * 这是人在「这条为什么是这个档」上打转的地方，所以**依据排在最前面**：
 * 六维各自凭什么、三句话说了什么、图里到底有没有那个东西。
 */
import { Link, useParams } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Table, TierBadge, bj } from '@/components/Bits'
import { Td, Th } from '@/components/ui'
import { useJudgementDetail } from '@/lib/queries'

const DIM_CN: Record<string, string> = {
  change: '变化具体',
  use: '与真实使用有关',
  gain: '读者能理解的价值',
  compare: '有比较参照',
  explain: '能形成编辑判断',
  csw: 'CSW 调性',
}

const KIND_CN: Record<string, string> = {
  product: '产品',
  detail: '细节',
  scene: '场景',
  poster: '海报',
  outfit: '穿着',
  screenshot: '截图',
  unrelated: '无关',
}

export function Item() {
  const { round, key } = useParams()
  const q = useJudgementDetail(Number(round), key)

  if (q.isLoading) return <Loading what="这一条" />
  if (q.error) return <ErrorBox error={q.error} />
  const d = q.data
  if (!d) return <Empty>没有这一条。</Empty>

  const c = d.candidate as Record<string, string | number | null>
  const j = d.judgement

  return (
    <section>
      <Head
        title={String(j?.three_sentences?.what_changed ?? c.account ?? key)}
        desc={
          <>
            <Link to={`/ledger?round=${round}`} className="text-accent no-underline">
              回第 {round} 轮的台账
            </Link>
            {' · '}
            <span className="mono">{key}</span>
          </>
        }
        right={
          <>
            {d.effective_tier && <TierBadge tier={d.effective_tier} />}
            {d.first_batch && <span className="text-[12px] text-accent">已进首批</span>}
          </>
        }
      />

      {!j && <Empty>这一轮没有判过这一条。</Empty>}

      {j && (
        <>
          {j.tier !== d.effective_tier && (
            <div className="mb-3 rounded-[var(--r)] border border-warn-soft bg-warn-soft px-3 py-2 text-[12.5px] text-warn">
              模型原判是「{j.tier}」，现在显示的是人改过的「{d.effective_tier}」。原判一个字没动。
            </div>
          )}

          <H2>三句话</H2>
          <ol className="m-0 space-y-1 pl-5 text-[13.5px] leading-relaxed">
            <li>{j.three_sentences.what_changed}</li>
            <li>{j.three_sentences.why_it_matters}</li>
            <li>{j.three_sentences.how_different}</li>
          </ol>
          {j.unanswered && j.unanswered !== 'none' && (
            <p className="mt-1 text-[12.5px] text-warn">
              答不清的地方：{UNANSWERED_CN[j.unanswered] ?? j.unanswered}
            </p>
          )}

          <H2>六维与依据</H2>
          <Table
            head={
              <tr>
                <Th>维度</Th>
                <Th>成立吗</Th>
                <Th>依据</Th>
              </tr>
            }
          >
            {j.dims.map(([dim, dj]) => (
              <tr key={dim} className="border-t border-rule align-top">
                <Td>{DIM_CN[dim] ?? dim}</Td>
                <Td>
                  <span
                    className={
                      dj.verdict === 'yes' ? 'text-ok' : dj.verdict === 'no' ? 'text-dim' : 'text-muted'
                    }
                  >
                    {{ yes: '成立', no: '不成立', unclear: '不明' }[dj.verdict] ?? dj.verdict}
                  </span>
                </Td>
                <Td className="text-[12.5px]">{dj.basis}</Td>
              </tr>
            ))}
          </Table>

          {(j.gaps.length > 0 || j.check_flags.length > 0) && (
            <>
              <H2>缺口与被标出的依据</H2>
              <ul className="m-0 space-y-0.5 pl-5 text-[13px] text-warn">
                {j.gaps.map((g, i) => (
                  <li key={`g${i}`}>{g}</li>
                ))}
                {j.check_flags.map((f, i) => (
                  <li key={`f${i}`}>{f}</li>
                ))}
              </ul>
              <p className="mt-1 text-[12.5px] text-muted">
                被标出的依据是 Jev 核过材料后认为「材料不支持」的那几条。
                <b>标出来给人看，不自动淘汰。</b>
              </p>
            </>
          )}
        </>
      )}

      <H2>图（{d.images.length} 张）</H2>
      {d.images.length === 0 ? (
        <Empty>这一条没有图，或者图还没下下来。</Empty>
      ) : (
        <Table
          head={
            <tr>
              <Th>序</Th>
              <Th>类型</Th>
              <Th>画面里有什么</Th>
              <Th>对应正文</Th>
              <Th>可作配图</Th>
            </tr>
          }
        >
          {d.images.map((m) => (
            <tr key={`${m.blake3}-${m.ordinal}`} className="border-t border-rule align-top">
              <Td className="tnum">{m.ordinal}</Td>
              <Td>
                {m.failed ? (
                  <span className="text-bad">没识别成</span>
                ) : (
                  (KIND_CN[m.kind] ?? m.kind ?? '——')
                )}
              </Td>
              <Td className="text-[12.5px]">{m.content || '——'}</Td>
              <Td className="text-[12.5px] text-muted">
                {m.matches_text || '——'}
                {m.missing_from_text && (
                  <div className="text-warn">正文提到但画面没有：{m.missing_from_text}</div>
                )}
              </Td>
              <Td>{m.failed ? '——' : m.usable_as_figure ? '是' : '否'}</Td>
            </tr>
          ))}
        </Table>
      )}

      <H2>原文</H2>
      <div className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
        <div className="text-[12px] text-muted">
          {String(c.account ?? '')} ·{' '}
          <a
            href={String(c.url ?? '')}
            target="_blank"
            rel="noreferrer"
            className="text-accent no-underline"
          >
            看原贴
          </a>
        </div>
        <p className="mb-0 mt-1 whitespace-pre-wrap text-[13px] leading-relaxed">
          {String(c.text ?? '')}
        </p>
        {c.translated ? (
          <p className="mb-0 mt-2 whitespace-pre-wrap text-[12.5px] text-muted">
            {String(c.translated)}
          </p>
        ) : null}
      </div>

      {d.overrides.length > 0 && (
        <>
          <H2>改档历史</H2>
          <Table
            head={
              <tr>
                <Th>从</Th>
                <Th>到</Th>
                <Th>理由</Th>
                <Th>谁</Th>
                <Th>时间</Th>
              </tr>
            }
          >
            {d.overrides.map((o, i) => (
              <tr key={i} className="border-t border-rule">
                <Td>{o.from_tier}</Td>
                <Td>{o.to_tier}</Td>
                <Td>{o.reason}</Td>
                <Td>{o.actor}</Td>
                <Td className="tnum">{bj(o.created_at)}</Td>
              </tr>
            ))}
          </Table>
        </>
      )}

      {d.marks.length > 0 && (
        <>
          <H2>Van 的勾选</H2>
          <ul className="m-0 space-y-0.5 pl-5 text-[13px]">
            {d.marks.map((m, i) => (
              <li key={i}>
                {{ like: '要这条', doubt: '存疑', note: '说了一句' }[m.mark] ?? m.mark}
                {m.note && `：「${m.note}」`}
                <span className="ml-2 text-[11.5px] text-dim">{bj(m.created_at)}</span>
              </li>
            ))}
          </ul>
        </>
      )}

      {d.deepcheck && (
        <>
          <H2>深核</H2>
          <div className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2 text-[12.5px]">
            <div>
              状态：{d.deepcheck.status} · {bj(d.deepcheck.started_at)} →{' '}
              {bj(d.deepcheck.ended_at)}
            </div>
            <pre className="mono mb-0 mt-1 overflow-x-auto whitespace-pre-wrap text-[12px]">
              {JSON.stringify(d.deepcheck.result, null, 2)}
            </pre>
          </div>
        </>
      )}

      {d.model_calls.length > 0 && (
        <>
          <H2>这一轮的模型调用（最近 {d.model_calls.length} 次）</H2>
          <Table
            head={
              <tr>
                <Th>用途</Th>
                <Th>模型</Th>
                <Th right>入 / 出 token</Th>
                <Th right>耗时</Th>
                <Th>结果</Th>
              </tr>
            }
          >
            {d.model_calls.map((m, i) => (
              <tr key={i} className="border-t border-rule">
                <Td>{PURPOSE_CN[m.purpose] ?? m.purpose}</Td>
                <Td className="mono text-[12px]">{m.model}</Td>
                <Td right className="tnum">
                  {m.input_tokens} / {m.output_tokens}
                </Td>
                <Td right className="tnum">
                  {m.latency_ms} ms
                </Td>
                <Td style={{ color: m.status === 'failed' ? 'var(--bad)' : undefined }}>
                  {m.status}
                  {m.attempts > 1 && ` · 第 ${m.attempts} 次`}
                  {m.error && <div className="text-[11.5px]">{m.error}</div>}
                </Td>
              </tr>
            ))}
          </Table>
        </>
      )}

      {j && (
        <p className="mt-4 text-[11.5px] text-dim">
          准则 {j.rubric_version} · 模型 {j.model} · 输入指纹{' '}
          <span className="mono">{j.inputs_hash.slice(0, 16)}</span> · 判于 {bj(j.created_at)}
        </p>
      )}
    </section>
  )
}

const UNANSWERED_CN: Record<string, string> = {
  missing_material: '缺材料',
  angle_not_formed: '角度还没成形',
  low_value: '价值不足',
}

const PURPOSE_CN: Record<string, string> = {
  recognize: '识别图片',
  judge: '逐条判断',
  deepcheck: '深核',
  triage: '初评',
  verify: '核对',
  embed: '向量化',
  rerank: '重排',
}
