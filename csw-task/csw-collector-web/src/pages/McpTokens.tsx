/**
 * MCP 令牌：给**其他机器上的** codex 查知识库用（`https://collector.aworld.ltd/mcp`）。
 *
 * 三条：
 * - **只给超级管理员。** 一枚令牌等于把知识库开给一台我们管不着的机器。
 * - **明文只在签发后显示这一次。** 库里只存哈希，关掉就再也看不到；丢了就吊销重签。
 * - **memory 不对外。** 选题准则卡里是 Van 的原话，这里根本不给选。
 */
import { useState } from 'react'

import { Empty, ErrorBox, H2, Head, Loading, Table, bj } from '@/components/Bits'
import { ConfirmDialog, Td, Th } from '@/components/ui'
import { errText } from '@/lib/api'
import { useIssueMcpToken, useMcpTokens, useRevokeMcpToken } from '@/lib/queries'
import type { McpToken, McpTokenIssued } from '@/lib/types'

const SCOPES: { key: string; label: string; desc: string }[] = [
  { key: 'magazine', label: '杂志背景', desc: 'kb_magazine_search、kb_image_search：杂志译文、商品、裁图' },
  { key: 'kb', label: '判断参考库', desc: 'kb_search、kb_similar_selected：已发布条目、范例、03 决定（含编辑部的选题判断）' },
]

const input = 'rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[13px]'

export function McpTokens() {
  const tokens = useMcpTokens()
  const issue = useIssueMcpToken()
  const revoke = useRevokeMcpToken()
  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<string[]>(['magazine', 'kb'])
  const [expire, setExpire] = useState(false)
  const [days, setDays] = useState(90)
  const [issued, setIssued] = useState<McpTokenIssued | null>(null)
  const [copied, setCopied] = useState(false)
  const [toRevoke, setToRevoke] = useState<McpToken | null>(null)
  const [promptCopied, setPromptCopied] = useState(false)

  const toggle = (k: string) =>
    setScopes((s) => (s.includes(k) ? s.filter((x) => x !== k) : [...s, k]))

  const submit = () => {
    setCopied(false)
    issue.mutate(
      { name: name.trim(), scopes, days: expire ? days : null },
      {
        onSuccess: (r) => {
          setIssued(r)
          setName('')
        },
      },
    )
  }

  const copy = async () => {
    if (!issued) return
    try {
      await navigator.clipboard.writeText(issued.token)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <section>
      <Head
        title="MCP 令牌"
        desc="给其他机器上的 codex 查知识库用。只读；令牌明文只在签发后显示一次。"
      />

      <H2>接入说明</H2>
      <div className="flex flex-wrap items-center gap-2 rounded-[var(--r)] border border-rule bg-surface p-3">
        <code className="mono select-all text-[12.5px]">{SETUP_URL}</code>
        <button
          onClick={() => void navigator.clipboard.writeText(SETUP_PROMPT).then(() => setPromptCopied(true))}
          className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px]"
        >
          {promptCopied ? '已复制' : '复制给智能体的话'}
        </button>
        <a href="/mcp-setup.md" target="_blank" rel="noreferrer" className="text-[12.5px]">
          打开看看
        </a>
        <span className="w-full text-[12.5px] text-muted">
          把这段话发给对方的 codex / Claude Code：它会照说明装好、向对方要令牌、验证，并学会以图搜图（含本机图片上传）。令牌另外私下给，不放进这段话里。
        </span>
      </div>

      <H2>签发</H2>
      <div className="space-y-2.5 rounded-[var(--r)] border border-rule bg-surface p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="w-[64px] text-[13px] text-muted">给谁用</span>
          <input
            value={name}
            maxLength={60}
            onChange={(e) => setName(e.target.value)}
            placeholder="人 + 机器，如「张三的 MacBook codex」"
            className={`${input} w-[280px]`}
          />
          <span className="text-[12.5px] text-muted">审计里按这个名字记。</span>
        </div>
        <div className="flex flex-wrap items-start gap-2">
          <span className="w-[64px] pt-0.5 text-[13px] text-muted">权限</span>
          <div className="space-y-1">
            {SCOPES.map((s) => (
              <label key={s.key} className="flex cursor-pointer items-start gap-2 text-[13px]">
                <input
                  type="checkbox"
                  className="mt-[3px]"
                  checked={scopes.includes(s.key)}
                  onChange={() => toggle(s.key)}
                />
                <span>
                  <b className="font-medium">{s.label}</b>
                  <span className="ml-2 text-[12px] text-muted">{s.desc}</span>
                </span>
              </label>
            ))}
            <div className="text-[12px] text-muted">
              选题记忆（memory_lookup）不对外：准则卡里是 Van 的原话。
            </div>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="w-[64px] text-[13px] text-muted">有效期</span>
          <label className="flex cursor-pointer items-center gap-1.5 text-[13px]">
            <input type="radio" checked={!expire} onChange={() => setExpire(false)} />
            不过期（靠吊销）
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-[13px]">
            <input type="radio" checked={expire} onChange={() => setExpire(true)} />
            <input
              type="number"
              min={1}
              max={3650}
              value={days}
              disabled={!expire}
              onChange={(e) => setDays(Number(e.target.value))}
              className={`${input} w-[72px] disabled:opacity-50`}
            />
            天后过期
          </label>
        </div>
        <div className="flex items-center gap-2 pl-[72px]">
          <button
            onClick={submit}
            disabled={issue.isPending || !name.trim() || scopes.length === 0}
            className="rounded-[var(--r-sm)] bg-accent px-3 py-1 text-[13px] text-white disabled:opacity-50"
          >
            签发
          </button>
          {issue.error && <span className="text-[12.5px] text-bad">{errText(issue.error)}</span>}
        </div>
      </div>

      {issued && (
        <div className="mt-3 rounded-[var(--r)] border border-warn-soft bg-warn-soft p-3">
          <div className="text-[13px] font-semibold text-warn">
            #{issued.id} {issued.name} 的令牌——只显示这一次，关掉就再也看不到
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <code className="mono select-all break-all rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-1 text-[12.5px]">
              {issued.token}
            </code>
            <button
              onClick={() => void copy()}
              className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px]"
            >
              {copied ? '已复制' : '复制'}
            </button>
            <button
              onClick={() => setIssued(null)}
              className="rounded-[var(--r-sm)] border border-rule bg-surface px-2.5 py-1 text-[12.5px]"
            >
              我已保存，关掉
            </button>
          </div>
          <p className="mt-2 text-[12.5px] text-ink">
            只私下交给使用者本人，<b>不要发群、不要写进文档</b>。对方把它放进环境变量 <code>CSW_KB_TOKEN</code>，
            codex 配置照下面写：
          </p>
          <pre className="mono mt-1.5 overflow-x-auto rounded-[var(--r-sm)] border border-rule bg-surface p-2 text-[12px] leading-[1.55]">
            {CLIENT_TOML}
          </pre>
        </div>
      )}

      <H2>已签的令牌</H2>
      {tokens.isLoading && <Loading what="令牌" />}
      {tokens.error && <ErrorBox error={tokens.error} />}
      {tokens.data?.length === 0 && <Empty>还没有签过令牌。</Empty>}
      {tokens.data && tokens.data.length > 0 && (
        <Table
          head={
            <tr>
              <Th>#</Th>
              <Th>给谁</Th>
              <Th>权限</Th>
              <Th>签发</Th>
              <Th>到期</Th>
              <Th>最近使用</Th>
              <Th>近 7 天调用</Th>
              <Th>状态</Th>
              <Th> </Th>
            </tr>
          }
        >
          {tokens.data.map((t) => (
            <tr key={t.id} className={`border-t border-rule ${t.active ? '' : 'text-muted'}`}>
              <Td className="tnum">{t.id}</Td>
              <Td>{t.name}</Td>
              <Td>{t.scopes.map((s) => SCOPES.find((x) => x.key === s)?.label ?? s).join('、')}</Td>
              <Td className="tnum">{bj(t.created_at)}</Td>
              <Td className="tnum">{t.expires_at ? bj(t.expires_at) : '不过期'}</Td>
              <Td className="tnum">{t.last_used_at ? bj(t.last_used_at) : '没用过'}</Td>
              <Td className="tnum">{t.calls_7d}</Td>
              <Td>
                {t.revoked_at ? (
                  <span>已吊销 {bj(t.revoked_at)}</span>
                ) : t.active ? (
                  <span className="text-ok">可用</span>
                ) : (
                  <span className="text-warn">已过期</span>
                )}
              </Td>
              <Td>
                {!t.revoked_at && (
                  <button
                    onClick={() => setToRevoke(t)}
                    className="rounded-[var(--r-sm)] border border-rule bg-surface px-2 py-0.5 text-[12.5px] text-bad"
                  >
                    吊销
                  </button>
                )}
              </Td>
            </tr>
          ))}
        </Table>
      )}
      {revoke.error && <div className="mt-2"><ErrorBox error={revoke.error} /></div>}
      <p className="mt-2 text-[12.5px] text-muted">
        每枚令牌每分钟最多 60 次，其中以图搜图 6 次；正式轮时段以图搜图暂停。吊销立即生效。
      </p>

      <ConfirmDialog
        open={toRevoke !== null}
        onClose={() => setToRevoke(null)}
        danger
        title="吊销这枚令牌？"
        confirmText="吊销"
        message={
          <>
            #{toRevoke?.id} {toRevoke?.name}：吊销后对方下一次请求就是 401，不能恢复，要用只能重签一枚。
          </>
        }
        onConfirm={() => {
          if (toRevoke) revoke.mutate(toRevoke.id)
          setToRevoke(null)
        }}
      />
    </section>
  )
}

const SETUP_URL = 'https://collector.aworld.ltd/mcp-setup.md'

// 发给对方智能体的整段话：装、要令牌、验证、以图搜图（本机图片走上传）。**不含令牌**
const SETUP_PROMPT = `请帮我在本机给你自己接上「营事编集室知识库」MCP（服务名 csw-kb，只读）。

1. 先读取接入说明：${SETUP_URL}
   用 curl 或你的网页读取工具都可以。之后按说明执行，说明和这里有出入时以说明为准。
2. 先判断你自己是哪种客户端（codex / Claude Code / 其他 MCP 客户端），按说明里对应的那一节配置：
   - 改配置文件前先读一遍。已有 csw-kb 就更新，不要重复追加。
   - codex 要把说明里列的 4 个工具配成免确认（approval_mode = "approve"）。
3. 配好后向我要令牌（形如 cswkb_ 开头）。我给你之前不要猜，也不要自己造。
4. 拿到令牌后：
   - 只把它写进环境变量 CSW_KB_TOKEN（按说明第 5 步，写进我的 shell 配置）。
   - 不要写进 config.toml、.mcp.json 或任何项目文件。
   - 不要在输出里回显令牌，只说「已配置」。
   - shell 里的 CSW_KB_TOKEN 和 MCP 连接用的必须是同一枚令牌，否则上传的图片会对不上。
5. 按说明第 6 步验证：
   - 用 curl 调一次 tools/list，确认能列出工具；
   - 再用 csw-kb 的 kb_magazine_search 查一次「snow peak 焚き火台」。
6. 记住以图搜图的用法（说明第 8 节），以后我给你图片时照这个做：
   - 图片有公网 https 地址：直接调 kb_image_search，参数 image_url。
   - 图片是本机文件：先用 shell 执行
       curl -sS -X POST https://collector.aworld.ltd/mcp/upload -H "Authorization: Bearer $CSW_KB_TOKEN" --data-binary @图片路径
     拿到返回的 upload_id（10 分钟内有效），再调 kb_image_search，参数 {"upload_id": "..."}。
   - 不要把图片转成 base64 填进工具参数。
   - 只收 jpg / png / webp，最大 10MB。
   如果当前目录或我指定的位置有图片，请用本机图片的方式实际搜一次，作为验证。
7. 最后告诉我：
   - 用的是哪种客户端，改了哪些文件；
   - 列出了哪几个工具，以及文字检索的前 3 条结果；
   - 如果做了以图搜图，报告前 3 条的刊期、页码和相似度；
   - 如果需要我重开终端或重启客户端，明确说出来。
   出错就对照说明第 7 节的出错对照处理，处理不了把原始报错告诉我。`

const CLIENT_TOML = `# ~/.codex/config.toml
[mcp_servers.csw-kb]
url = "https://collector.aworld.ltd/mcp"
bearer_token_env_var = "CSW_KB_TOKEN"

# 这几件都只读，免得每次调用都要人点同意
[mcp_servers.csw-kb.tools.kb_magazine_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_image_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_search]
approval_mode = "approve"
[mcp_servers.csw-kb.tools.kb_similar_selected]
approval_mode = "approve"`
