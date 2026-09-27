/**
 * 后端接口的返回形状。
 *
 * **只写页面真的会读的字段**，不照抄整个响应——照抄的那部分没人维护，
 * 后端改了名字这边也不会报错，等到页面上出现 `undefined` 才发现。
 *
 * 契约在 `csw-task/csw-collector/API.md`；`core::types` 的那部分由
 * `csw-collector schema` 导出到 `contract.json`，这里不重复。
 */

/** 四档。**没有第五档，也没有分数。** */
export type Tier = 'recommend' | 'alternate' | 'not_recommend' | 'pending_check'

export interface RoundBrief {
  id: number
  kind: 'task' | 'manual' | 'prefetch' | 'replay' | 'backtest'
  trigger: string
  run_id: number | null
  task_id: number | null
  window_start: string
  window_end: string
  status: string
  note: string
  created_at: string
}

export interface RoundDetail extends RoundBrief {
  rubric_version: string
  kb_snapshot: string
  instructions_hash: string
  /** [档, 条数]，按档名排序（模型原判） */
  tiers: [string, number][]
  /** [档, 条数]，叠加人工改档后的有效档。页面上的数字以它为准 */
  effective_tiers: [string, number][]
  /** 推荐贴文数与独立选题数分开统计 */
  topics: TopicCounts
  candidates: number
  carried: number
  unjudged: number
}

export interface StepRow {
  step: string
  attempt: number
  status: string
  input_hash: string
  counts: Record<string, unknown>
  error: string
  started_at: string | null
  ended_at: string | null
}

export interface DimJudgement {
  verdict: 'yes' | 'no' | 'unclear'
  basis: string
}

export interface TopicCounts {
  recommend_posts: number
  recommend_topics: number
  alternate_posts: number
  alternate_topics: number
  topics: number
}

/** 三句话（v2）：是什么 / 为什么值得看 / 依据是什么。旧行只有 v1 的三个键 */
export interface ThreeSentences {
  what?: string
  why_worth?: string
  grounds?: string
  what_changed?: string
  why_it_matters?: string
  how_different?: string
}

export type GapLevel = 'decision' | 'production' | 'boundary'
export type GapOwner = 'collector' | 'editor' | 'van'

/** 一条缺口。旧行是纯字符串，用 `normGaps` 统一成这个形状 */
export interface Gap {
  level: GapLevel
  what: string
  owner: GapOwner
  tried: string
  next: string
}

export interface ComparisonHit {
  ref_no: string
  title: string
  url: string
  state: 'published' | 'draft' | 'generated' | 'decision' | 'unknown'
  published_at: string
  body_available: boolean
  dup_fact: string
}

export interface Comparison {
  verdict: string
  against: string
  note: string
  hits?: ComparisonHit[]
}

export interface Novelty {
  kind?: 'existing_feature' | 'evidenced_change' | 'explainable_design'
  basis?: string
  prior_evidence?: string
}

export interface Readiness {
  fact_source?: 'primary' | 'reshared' | 'brand_claim_only' | 'unknown'
  usable_images?: number
  material_complete?: boolean
  note?: string
}

export interface MemberNote {
  candidate_key: string
  new_info: string
  is_duplicate: boolean
}

/** 选题：同产品、同事件的多帖合成一个报道对象 */
export interface Topic {
  topic_key: string
  primary_key: string
  members: string[]
  merge_note: string
  tier: Tier | null
  headline: string
  synthesis: { shared_facts: string[]; per_member: MemberNote[]; unsupported: string[] }
}

export interface JudgementRow {
  candidate_key: string
  /** 模型的原判。**人工改过档也不动它** */
  tier: Tier
  /** 现在生效的那一档 */
  effective_tier: Tier
  override_reason: string
  override_actor: string
  first_batch: boolean
  /** [维度, 判断] 的数组，键固定六个 */
  dims: [string, DimJudgement][]
  three_sentences: ThreeSentences
  comparison: Comparison
  heat_note: string
  image_seen: boolean
  gaps: (Gap | string)[]
  check_flags: string[]
  account: string
  url: string
  /** 「具体对象｜一句推荐理由」 */
  headline: string
  novelty: Novelty
  readiness: Readiness
  /** 代表图的内容哈希，取图走 /api/media/{hash} */
  cover: string | null
  topic_key: string | null
  posted_at: string | null
  decision_gaps: number
}

export interface ImageRow {
  blake3: string
  ordinal: number
  url: string
  failed: boolean
  kind: string
  content: string
  matches_text: string
  missing_from_text: string
  usable_as_figure: boolean
  model: string
  prompt_version: string
}

export interface JudgementDetail {
  candidate: Record<string, unknown>
  judgement: (Pick<
    JudgementRow,
    | 'tier'
    | 'dims'
    | 'three_sentences'
    | 'comparison'
    | 'heat_note'
    | 'image_seen'
    | 'gaps'
    | 'check_flags'
    | 'headline'
    | 'novelty'
    | 'readiness'
  > & {
    unanswered: string
    look: string
    priority_hits: string[]
    lower_hits: string[]
    jev_disagreement: string
    kb_refs: string[]
    memory_refs: string[]
    inputs_hash: string
    model: string
    rubric_version: string
    created_at: string
    rejudged: boolean
  }) | null
  effective_tier: Tier | ''
  topic: Topic | null
  /** [地址, 结果] */
  refetches: [string, string][]
  overrides: OverrideRow[]
  marks: VanMarkRow[]
  images: ImageRow[]
  deepcheck: { status: string; result: unknown; started_at: string; ended_at: string } | null
  model_calls: ModelCallRow[]
  first_batch: boolean
}

export interface OverrideRow {
  candidate_key: string
  from_tier: string
  to_tier: string
  reason: string
  actor: string
  created_at: string
}

export interface VanMarkRow {
  candidate_key: string
  mark: 'like' | 'doubt' | 'note'
  note: string
  actor: string
  created_at: string
}

export interface ModelCallRow {
  purpose: string
  model: string
  input_tokens: number
  output_tokens: number
  latency_ms: number
  attempts: number
  status: string
  error: string
  created_at: string
}

export interface SweepRow {
  sweep_key: string
  platform: string
  source_key: string
  query: string
  found: number
  fetched_unique: number
  in_window: number
  reviewed: number
  /** **应当是 0**：工作台每条都判。不是 0 就说明那一步没跑完 */
  unreviewed: number
  registered: number
  result: string
  error: string
  paged_to_end: boolean
}

export interface Coverage {
  sweeps: SweepRow[]
  harvest: Record<string, number>
}

/** 待补录清单：出现在选题里、却不在 csw 在册名单里的品牌 */
export interface BackfillEvidence {
  kind: string
  title: string
  date: string | null
  url: string
  /** 决定的结论码；已发条目与范例是空 */
  conclusion: string
}

export interface BackfillRow {
  brand: string
  adopted: number
  mentions: number
  latest: string | null
  evidence: BackfillEvidence[]
}

export interface Backfill {
  registered_brands: number
  rows: BackfillRow[]
}

export interface VanItem {
  candidate_key: string
  tier: Tier
  title: string
  brand: string
  url: string
  three_sentences: ThreeSentences
  heat_note: string
  look: string
  marks: string[]
  dedup: string
  decision_gaps: string[]
  cover: string | null
  images: string[]
  topic_key: string | null
  /** 同一选题的其余帖子：不另占卡片 */
  also: { candidate_key: string; url: string; title: string }[]
}

export interface PendingRow {
  candidate_key: string
  round_id: number
  headline: string
  account: string
  url: string
  cover: string | null
  rounds_pending: number
  first_pending_at: string
  gaps: Gap[]
  refetches: [string, string][]
}

export interface Health {
  ok: boolean
  uptime_secs: number
  deps: { name: string; ok: boolean; note: string }[]
  outbox_conflicts: number
}

export interface WorkItem {
  id: number
  kind: string
  round_id: number | null
  payload_json: string
  actor: string
  status: 'queued' | 'running' | 'done' | 'failed'
  note: string
  created_at: string
}

export interface AuditRow {
  id: number
  actor: string
  action: string
  target: string
  detail_json: string
  created_at: string
}

export interface Settings {
  engine_base_url: string
  csw_base_url: string
  vector_base_url: string
  model: string
  fallback_model: string
  embed_model: string
  jev_enabled: boolean
  features: Record<string, boolean>
  /** [env 名, 有没有配]。**不返回值** */
  secrets: [string, boolean][]
}

export interface Rubric {
  version: string
  core_question: string
  three_questions: string[]
  four_questions: string[]
  not_a_checklist: string
  china_reader: string
  readiness_note: string
  priority: string[]
  lower: string[]
  dim_anchors: { dim: string; anchor: string }[]
  /** 四项「不是维度」的东西：这套框架里最容易被偷偷用上的 */
  not_dimensions: string[]
}

export interface MemoryRule {
  rule_key: string
  text: string
  version: string
  confirmed_by_van: boolean
  updated_at: string
}

export interface MemoryCase {
  case_key: string
  decision: string
  /** Van 原话，一字不改 */
  quote: string
  source_url: string
  decided_at: string
}

export interface KbDocRow {
  id: number
  kind: string
  ref_id: string
  title: string
  brand: string
  url: string
  published_at: string | null
  publish_state: string
  is_reference: boolean
  snippet: string
  routes?: string[]
  backfilled?: boolean
}

export interface KbSearchResult {
  brands_hit: string[]
  missing_kinds: string[]
  counts: { vector: number; brand: number; fts: number; merged: number; truncated: number }
  docs: KbDocRow[]
}

export interface KbStatus {
  cursors: { source: string; cursor: string; synced_at: string }[]
  docs: number
  docs_by_kind: Record<string, number>
  品牌: number
  别名: number
  待算向量: number
  embed_model: string
  /** 被标成范例的篇数（应有）；docs_by_kind.example 是实有 */
  examples_expected: number
  /** 标成范例、却没有整篇入库的 post_id */
  examples_missing: string[]
}

/**
 * 一条硬性排除规则 = 一次 Van 的否决。
 *
 * `active` 为假有两种情形，页面上要分开说：没有原话（进表但不自动生效，等人确认）、
 * 人工停用（`inactive_reason` 写着为什么）。
 */
export interface Exclusion {
  id: number
  decision_ref: string
  item_key: string
  title: string
  brand: string
  source_url: string
  /** Van 的原话。**从本地库直读，没经过任何外部服务** */
  quote: string
  reason: string
  reason_code: string
  decided_at: string
  actor_role: string
  active: boolean
  inactive_reason: string
  changed_by: string
  changed_at: string
}

/** 这一轮被规则挡下的那几条。 */
export interface RoundExclusions {
  挡下: number
  捞回: number
  明细: {
    候选: string
    规则: number
    否过的条目: string
    原话: string
    同一事实: number
    新料: number
    已捞回: boolean
    捞回人: string
    捞回理由: string
  }[]
}
