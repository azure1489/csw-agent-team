/**
 * 所有接口的取数与写入，集中在这里。
 *
 * **页面里不出现 `api.get('/…')`**：路径散在十一个页面里的话，
 * 后端改一个路径就要翻十一个文件，而漏掉的那个要等有人点到才发现。
 *
 * 缓存键的第一段就是资源名，写操作按那一段作废——
 * 改完档不刷新台账，人会以为没改上。
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api } from './api'
import type {
  AuditRow,
  Coverage,
  Exclusion,
  Health,
  ImageRow,
  JudgementDetail,
  JudgementRow,
  KbSearchResult,
  KbStatus,
  MemoryCase,
  MemoryRule,
  RoundBrief,
  RoundDetail,
  RoundExclusions,
  Rubric,
  Settings,
  StepRow,
  Tier,
  VanItem,
  VanMarkRow,
  WorkItem,
} from './types'

const get = async <T,>(url: string, params?: Record<string, unknown>): Promise<T> =>
  (await api.get<T>(url, { params })).data

// ─────────────────────────────── 轮次 ───────────────────────────────

export const useRounds = (params?: { kind?: string; status?: string; limit?: number }) =>
  useQuery({
    queryKey: ['rounds', params],
    queryFn: () => get<RoundBrief[]>('/rounds', params),
  })

export const useRound = (id?: number) =>
  useQuery({
    queryKey: ['rounds', id],
    queryFn: () => get<RoundDetail>(`/rounds/${id}`),
    enabled: !!id,
  })

export const useSteps = (id?: number) =>
  useQuery({
    queryKey: ['steps', id],
    queryFn: () => get<StepRow[]>(`/rounds/${id}/steps`),
    enabled: !!id,
  })

export const useCoverage = (id?: number) =>
  useQuery({
    queryKey: ['coverage', id],
    queryFn: () => get<Coverage>(`/rounds/${id}/coverage`),
    enabled: !!id,
  })

export const useRoundMedia = (id?: number, state?: string) =>
  useQuery({
    queryKey: ['media', id, state],
    queryFn: () => get<ImageRow[]>(`/rounds/${id}/media`, { state, limit: 500 }),
    enabled: !!id,
  })

// ─────────────────────────────── 台账 ───────────────────────────────

export const useJudgements = (id?: number, params?: { tier?: string; has_gap?: boolean }) =>
  useQuery({
    queryKey: ['judgements', id, params],
    queryFn: () => get<JudgementRow[]>(`/rounds/${id}/judgements`, { ...params, limit: 1000 }),
    enabled: !!id,
  })

export const useJudgementDetail = (id?: number, key?: string) =>
  useQuery({
    queryKey: ['judgement', id, key],
    queryFn: () => get<JudgementDetail>(`/rounds/${id}/judgements/${encodeURIComponent(key!)}`),
    enabled: !!id && !!key,
  })

export const useVanMarks = (id?: number) =>
  useQuery({
    queryKey: ['van-marks', id],
    queryFn: () => get<VanMarkRow[]>(`/rounds/${id}/van-marks`),
    enabled: !!id,
  })

export const usePendingCheck = () =>
  useQuery({ queryKey: ['pending-check'], queryFn: () => get<string[]>('/pending-check') })

/** 改档。**原判不动**，改动另存一行；改完把台账与详情一起作废 */
export function useOverride(roundId?: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { key: string; to_tier: Tier; reason: string }) =>
      api.post(`/rounds/${roundId}/judgements/${encodeURIComponent(v.key)}/override`, {
        to_tier: v.to_tier,
        reason: v.reason,
      }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['judgements'] })
      void qc.invalidateQueries({ queryKey: ['judgement'] })
      void qc.invalidateQueries({ queryKey: ['metrics'] })
      void qc.invalidateQueries({ queryKey: ['audit'] })
    },
  })
}

export function useSetFirstBatch(roundId?: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (keys: string[]) => api.post(`/rounds/${roundId}/first-batch`, { keys }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['judgements'] })
      void qc.invalidateQueries({ queryKey: ['judgement'] })
    },
  })
}

// ────────────────────────────── Van 模式 ──────────────────────────────

export const useVanToday = () =>
  useQuery({
    queryKey: ['van-today'],
    queryFn: () => get<{ round_id: number | null; items: VanItem[] }>('/van/today'),
  })

/** 勾选。**只写本地，不回写引擎**——进不进评选由主编代录并附原话 */
export function useVanMark() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { candidate_key: string; mark: string; note?: string; remove?: boolean }) =>
      api.post('/van/marks', v),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['van-today'] })
      void qc.invalidateQueries({ queryKey: ['van-marks'] })
    },
  })
}

// ─────────────────────────── 知识库与记忆 ───────────────────────────

export const useKbStatus = () =>
  useQuery({ queryKey: ['kb-status'], queryFn: () => get<KbStatus>('/kb/status') })

export const useKbSearch = (q: string) =>
  useQuery({
    queryKey: ['kb-search', q],
    queryFn: () => get<KbSearchResult>('/kb/search', { q, limit: 20 }),
    enabled: q.trim().length > 0,
    // 搜一次要占一次 GPU，别让它跟着重渲染重跑
    staleTime: 60_000,
  })

export const useKbBrand = (brand: string) =>
  useQuery({
    queryKey: ['kb-brand', brand],
    queryFn: () => get<{ brand: string; docs: KbSearchResult['docs'] }>(
      `/kb/brands/${encodeURIComponent(brand)}`,
      { limit: 50 },
    ),
    enabled: brand.trim().length > 0,
  })

export const useMemoryRules = () =>
  useQuery({ queryKey: ['memory-rules'], queryFn: () => get<MemoryRule[]>('/memory/rules') })

export const useMemoryCases = () =>
  useQuery({ queryKey: ['memory-cases'], queryFn: () => get<MemoryCase[]>('/memory/cases') })

// ───────────────────────── 框架、指标、运行 ─────────────────────────

export const useRubric = () =>
  useQuery({
    queryKey: ['rubric'],
    queryFn: () => get<Rubric>('/rubric'),
    // 它跟着代码走，不会自己变
    staleTime: Infinity,
  })

export const useSelectionMetrics = () =>
  useQuery({
    queryKey: ['metrics'],
    queryFn: () => get<Record<string, number>>('/metrics/selection'),
  })

export const useSettings = () =>
  useQuery({ queryKey: ['settings'], queryFn: () => get<Settings>('/settings') })

export const useHealth = () =>
  useQuery({
    queryKey: ['health'],
    // 走 /api/healthz 而不是裸的 /healthz：后者不鉴权（探活与看门狗要从回环读它），
    // 因此反代那层不往公网放，页面拿不到。两个是同一个 handler。
    queryFn: () => get<Health>('/healthz'),
    refetchInterval: 30_000,
  })

export const useWork = () =>
  useQuery({
    queryKey: ['work'],
    queryFn: () => get<WorkItem[]>('/work', { limit: 20 }),
    // 排队的活在后台做，页面要能看见它动
    refetchInterval: 10_000,
  })

export const useAudit = () =>
  useQuery({ queryKey: ['audit'], queryFn: () => get<AuditRow[]>('/audit', { limit: 50 }) })

/** 手动开一轮。**只排队**，真正干活的是常驻循环 */
export function useOpenRound() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { days?: number; window_start?: string; window_end?: string }) =>
      api.post<{ work_id: number; queued_ahead: number }>('/rounds', v),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['work'] }),
  })
}

/** 重跑一步。`step` 只认 harvest 与 judge；**登记与提交不动** */
export function useRerun() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { roundId: number; step: 'harvest' | 'judge' }) =>
      api.post<{ work_id: number; queued_ahead: number }>(
        `/rounds/${v.roundId}/steps/${v.step}/rerun`,
        {},
      ),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['work'] }),
  })
}

// ───────────────────────────── 硬性排除 ─────────────────────────────

export const useExclusions = () =>
  useQuery({ queryKey: ['exclusions'], queryFn: () => get<Exclusion[]>('/exclusions') })

export const useRoundExclusions = (roundId?: number) =>
  useQuery({
    queryKey: ['round-exclusions', roundId],
    queryFn: () => get<RoundExclusions>(`/rounds/${roundId}/exclusions`),
    enabled: !!roundId,
  })

export function useSetExclusionActive() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { id: number; active: boolean; reason: string }) =>
      api.post(`/exclusions/${v.id}/active`, { active: v.active, reason: v.reason }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['exclusions'] })
      void qc.invalidateQueries({ queryKey: ['audit'] })
    },
  })
}

export function useRestoreExcluded(roundId?: number) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: { key: string; reason: string }) =>
      api.post(`/rounds/${roundId}/exclusions/${encodeURIComponent(v.key)}/restore`, {
        reason: v.reason,
      }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['round-exclusions'] })
      void qc.invalidateQueries({ queryKey: ['judgements'] })
      void qc.invalidateQueries({ queryKey: ['audit'] })
    },
  })
}
