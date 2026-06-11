import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api'

export function useRoles() {
  return useQuery({ queryKey: ['roles'], queryFn: api.listRoles })
}

/** 返回一个 code → 角色显示名 的解析器 */
export function useRoleName(): (code: string) => string {
  const { data } = useRoles()
  return (code: string) => data?.find((r) => r.code === code)?.name || code
}
