import { useMemo } from 'react'
import type { SelectOption } from './field'

export function Avatar({ name, size = 28, tone }: { name?: string; size?: number; tone?: string }) {
  const ch = (name || '?').slice(0, 1)
  const palette = ['#5b5bd6', '#3667d6', '#1f9d57', '#b5730a', '#d63b3b', '#8b5cf6', '#0891b2']
  const idx = (name || '').split('').reduce((a, c) => a + c.charCodeAt(0), 0) % palette.length
  const bg = tone || palette[idx]
  return (
    <span
      style={{
        width: size,
        height: size,
        borderRadius: 7,
        background: bg,
        color: '#fff',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontSize: size * 0.42,
        fontWeight: 600,
        flexShrink: 0,
      }}
    >
      {ch}
    </span>
  )
}

export function Tabs({ tabs, value, onChange }: { tabs: SelectOption[]; value: string; onChange: (v: string) => void }) {
  return (
    <div className="row" style={{ gap: 2, borderBottom: '1px solid var(--border)' }}>
      {tabs.map((t) => {
        const val = typeof t === 'string' ? t : t.value
        const lab = typeof t === 'string' ? t : t.label
        const on = value === val
        return (
          <button
            key={val}
            onClick={() => onChange(val)}
            style={{
              padding: '9px 14px',
              fontSize: 13.5,
              fontWeight: 550,
              border: 'none',
              background: 'transparent',
              color: on ? 'var(--text)' : 'var(--text-2)',
              borderBottom: `2px solid ${on ? 'var(--accent)' : 'transparent'}`,
              marginBottom: -1,
              transition: 'color .12s',
            }}
          >
            {lab}
          </button>
        )
      })}
    </div>
  )
}

// ---------- Markdown（轻量；迁自设计稿 renderMd，像素级匹配） ----------
function renderMd(src: string): string {
  const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  const lines = src.split('\n')
  let out = '',
    inUl = false,
    inCode = false
  const inline = (s: string) =>
    esc(s)
      .replace(/`([^`]+)`/g, '<code style="background:var(--surface-3);padding:1px 5px;border-radius:4px;font-family:var(--mono);font-size:.86em">$1</code>')
      .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  for (const ln of lines) {
    if (ln.trim().startsWith('```')) {
      inCode = !inCode
      out += inCode
        ? '<pre style="background:var(--surface-2);border:1px solid var(--border);border-radius:7px;padding:10px 12px;overflow:auto;font-family:var(--mono);font-size:12px;margin:8px 0">'
        : '</pre>'
      continue
    }
    if (inCode) {
      out += esc(ln) + '\n'
      continue
    }
    const cb = ln.match(/^\s*-\s*\[( |x)\]\s+(.*)$/)
    if (cb) {
      if (!inUl) {
        out += '<ul style="list-style:none;padding-left:2px;margin:6px 0">'
        inUl = true
      }
      out += `<li style="display:flex;gap:7px;align-items:flex-start;padding:2px 0"><span style="margin-top:1px;color:${cb[1] === 'x' ? 'var(--green)' : 'var(--text-3)'}">${cb[1] === 'x' ? '☑' : '☐'}</span><span>${inline(cb[2])}</span></li>`
      continue
    }
    const h = ln.match(/^(#{1,4})\s+(.*)$/)
    if (h) {
      if (inUl) {
        out += '</ul>'
        inUl = false
      }
      const lv = h[1].length
      const sz = [17, 15, 13.5, 13][lv - 1]
      out += `<div style="font-weight:650;font-size:${sz}px;margin:12px 0 5px">${inline(h[2])}</div>`
      continue
    }
    const li = ln.match(/^\s*(\d+\.|[-*])\s+(.*)$/)
    if (li) {
      if (!inUl) {
        out += '<ul style="padding-left:18px;margin:5px 0">'
        inUl = true
      }
      out += `<li style="padding:1px 0">${inline(li[2])}</li>`
      continue
    }
    if (inUl) {
      out += '</ul>'
      inUl = false
    }
    if (ln.trim() === '') {
      out += '<div style="height:6px"></div>'
      continue
    }
    out += `<div style="margin:2px 0">${inline(ln)}</div>`
  }
  if (inUl) out += '</ul>'
  if (inCode) out += '</pre>'
  return out
}

export function MarkdownView({ text }: { text?: string }) {
  const html = useMemo(() => renderMd(text || ''), [text])
  return <div className="md-body" style={{ fontSize: 13, lineHeight: 1.65, color: 'var(--text)' }} dangerouslySetInnerHTML={{ __html: html }} />
}
