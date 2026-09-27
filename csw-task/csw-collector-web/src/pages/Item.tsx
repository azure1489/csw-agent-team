/**
 * 事件详情：**这一条的全貌。**
 *
 * 一次拿全（候选原文、每张图与描述、判断、改档历史、Van 勾选、深核、模型调用）——
 * 分七个接口去拿，这一页上就是七个各自转圈的小方块。
 *
 * 顺序照 09-22 反馈第六项：**先让人直接看图、读原文**，再看发布时间、关键缺口、
 * 查重与推荐理由；六维依据、制作条件、图片描述表、深核与模型调用这些技术记录折叠在后面。
 * 图片描述用于辅助核查，不能代替对外观、设计和审美的直接判断。
 */
import { Link, useParams } from 'react-router-dom'

import { Empty, ErrorBox, H2, Head, Loading, Table, TierBadge, bj } from '@/components/Bits'
import { DedupBadge, Fold, Gallery, GapList, Hits } from '@/components/Judged'
import { Td, Th } from '@/components/ui'
import { DIMS, FACT_SOURCE, NOVELTY, TIER_LABEL } from '@/lib/constants'
import { normGaps, three, titleOf } from '@/lib/judgement'
import { useJudgementDetail } from '@/lib/queries'

const KIND_CN: Record<string, string> = {
  product: '产品',
  detail: '细节',
  scene: '场景',
  poster: '海报',
  outfit: '穿着',
  screenshot: '截图',
  unrelated: '无关',
}

const REFETCH_CN: Record<string, string> = {
  ok: '取到',
  blocked: '地址不安全，未访问',
  http_error: '对方返回错误',
  too_large: '页面过大',
  bad_type: '不是网页',
  failed: '访问失败',
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
  const t = three(j?.three_sentences)
  const gaps = normGaps(j?.gaps)
  const decision = gaps.filter((g) => g.level === 'decision')
  const rest = gaps.filter((g) => g.level !== 'decision')
  const pics = d.images.filter((m) => !m.failed && m.blake3).map((m) => m.blake3)
  const topic = d.topic && d.topic.members.length > 1 ? d.topic : null

  return (
    <section>
      <Head
        title={titleOf(j?.headline, j?.three_sentences) || String(c.account ?? key)}
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
            {d.effective_tier && (
              <TierBadge
                tier={d.effective_tier}
                awaiting={decision.length > 0}
              />
            )}
            {d.first_batch && <span className="text-[12px] text-accent">已进首批</span>}
          </>
        }
      />

      {j && j.tier !== d.effective_tier && (
        <div className="mb-3 rounded-[var(--r)] border border-warn-soft bg-warn-soft px-3 py-2 text-[12.5px] text-warn">
          模型原判是「{TIER_LABEL[j.tier] ?? j.tier}」，现在显示的是人改过的「
          {TIER_LABEL[d.effective_tier] ?? d.effective_tier}」。原判一个字没动。
        </div>
      )}

      {topic && (
        <div className="mb-3 rounded-[var(--r)] border border-rule bg-accent-soft px-3 py-2 text-[12.5px]">
          这一帖属于选题「{topic.headline || topic.topic_key}」，同一选题共 {topic.members.length} 帖，
          推荐位只算一个：
          {topic.members
            .filter((m) => m !== key)
            .map((m) => (
              <Link
                key={m}
                to={`/ledger/${round}/${encodeURIComponent(m)}`}
                className="ml-1.5 text-accent no-underline"
              >
                {m}
              </Link>
            ))}
        </div>
      )}

      <H2>实图（{pics.length} 张）</H2>
      <Gallery hashes={pics} />

      <H2>原文</H2>
      <div className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2">
        <div className="text-[12px] text-muted">
          {String(c.account ?? '')} · 原始披露 {bj(c.posted_at as string | null)} · 首次入库{' '}
          {bj(c.ingested_at as string | null)} ·{' '}
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

      {!j && <Empty>这一轮没有判过这一条。</Empty>}

      {j && (
        <>
          <H2>影响判断的缺口</H2>
          <GapList gaps={decision} empty="没有影响判断的缺口。" />
          {j.unanswered && j.unanswered !== 'none' && (
            <p className="mt-1 text-[12.5px] text-warn">
              答不清的地方：{UNANSWERED_CN[j.unanswered] ?? j.unanswered}
            </p>
          )}

          <H2>
            <span className="inline-flex items-center gap-2">
              查重 <DedupBadge verdict={j.comparison.verdict} sm />
            </span>
          </H2>
          <Hits hits={j.comparison.hits ?? []} />
          {j.comparison.note && <p className="mt-1 text-[12.5px] text-muted">{j.comparison.note}</p>}

          <H2>推荐理由</H2>
          <ol className="m-0 space-y-1 pl-5 text-[13.5px] leading-relaxed">
            <li>
              <span className="text-muted">是什么：</span>
              {t.what || '——'}
            </li>
            <li>
              <span className="text-muted">为什么值得看：</span>
              {t.why || '——'}
            </li>
            <li>
              <span className="text-muted">依据：</span>
              {t.grounds || '——'}
            </li>
          </ol>
          {j.novelty?.kind && (
            <p className="mt-1 text-[12.5px] text-muted">
              看点类型：{NOVELTY[j.novelty.kind] ?? j.novelty.kind}
              {j.novelty.prior_evidence && `（旧款依据：${j.novelty.prior_evidence}）`}
            </p>
          )}

          {j.check_flags.length > 0 && (
            <>
              <H2>留痕</H2>
              <ul className="m-0 space-y-0.5 pl-5 text-[13px] text-warn">
                {j.check_flags.map((f, i) => (
                  <li key={i}>{f}</li>
                ))}
              </ul>
              <p className="mt-1 text-[12.5px] text-muted">
                「【口径】」开头的是代码按口径改过或删过的地方；其余是 Jev 核过材料后认为「材料不支持」的依据。
                <b>标出来给人看，不自动淘汰。</b>
                {j.rejudged && ' 这一条因口径违例退回重判过一次。'}
              </p>
            </>
          )}

          <Fold title="六维依据（值得推荐的价值是核心，其余为支撑）">
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
                  <Td>{DIMS[dim]?.full ?? dim}</Td>
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
          </Fold>

          <Fold title="制作条件、影响成稿的缺口与表达边界">
            <p className="mt-0 text-[12.5px] text-muted">
              事实来源 {FACT_SOURCE[j.readiness?.fact_source ?? 'unknown'] ?? '不详'} · 可作配图{' '}
              {j.readiness?.usable_images ?? 0} 张 · 资料{j.readiness?.material_complete ? '齐' : '未齐'}
              {j.readiness?.note && ` · ${j.readiness.note}`}（不参与定档）
            </p>
            <GapList gaps={rest} empty="没有。" />
          </Fold>

          <Fold title="热度与实图所见">
            <p className="m-0 text-[12.5px]">热度：{j.heat_note || '——'}（是输入的呈现，不是维度）</p>
            {j.look && <p className="mb-0 mt-1 text-[12.5px]">实图所见：{j.look}</p>}
          </Fold>
        </>
      )}

      {d.refetches.length > 0 && (
        <Fold title={`定点补读（${d.refetches.length} 个外链）`}>
          <ul className="m-0 space-y-0.5 pl-5 text-[12.5px]">
            {d.refetches.map(([u, st], i) => (
              <li key={i}>
                <span className="mono break-all">{u}</span>：{REFETCH_CN[st] ?? st}
              </li>
            ))}
          </ul>
        </Fold>
      )}

      <Fold title={`图片描述（${d.images.length} 张，辅助核查用）`}>
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
                <Td className="tnum">{m.ordinal + 1}</Td>
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
      </Fold>

      {d.overrides.length > 0 && (
        <Fold title={`改档历史（${d.overrides.length} 次）`} open>
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
                <Td>{TIER_LABEL[o.from_tier] ?? o.from_tier}</Td>
                <Td>{TIER_LABEL[o.to_tier] ?? o.to_tier}</Td>
                <Td>{o.reason}</Td>
                <Td>{o.actor}</Td>
                <Td className="tnum">{bj(o.created_at)}</Td>
              </tr>
            ))}
          </Table>
        </Fold>
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
        <Fold title="深核">
          <div className="rounded-[var(--r)] border border-rule bg-surface px-3 py-2 text-[12.5px]">
            <div>
              状态：{d.deepcheck.status} · {bj(d.deepcheck.started_at)} →{' '}
              {bj(d.deepcheck.ended_at)}
            </div>
            <pre className="mono mb-0 mt-1 overflow-x-auto whitespace-pre-wrap text-[12px]">
              {JSON.stringify(d.deepcheck.result, null, 2)}
            </pre>
          </div>
        </Fold>
      )}

      {d.model_calls.length > 0 && (
        <Fold title={`这一轮的模型调用（最近 ${d.model_calls.length} 次）`}>
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
        </Fold>
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
