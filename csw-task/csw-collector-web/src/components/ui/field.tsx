import { useState, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react'
import { I, type IconComponent } from '@/components/icons'

export function Field({
  label,
  hint,
  required,
  children,
  style,
  htmlFor,
}: {
  label?: ReactNode
  hint?: ReactNode
  required?: boolean
  children?: ReactNode
  style?: CSSProperties
  htmlFor?: string
}) {
  return (
    <div className="col" style={{ gap: 6, ...style }}>
      {label && (
        <label htmlFor={htmlFor} style={{ fontSize: 13, fontWeight: 550, color: 'var(--text)' }}>
          {label} {required && <span style={{ color: 'var(--red)' }}>*</span>}
        </label>
      )}
      {children}
      {hint && (
        <div className="t3 xs" style={{ lineHeight: 1.45 }}>
          {hint}
        </div>
      )}
    </div>
  )
}

const inputBase: CSSProperties = {
  width: '100%',
  padding: '7px 11px',
  fontSize: 13.5,
  color: 'var(--text)',
  background: 'var(--surface)',
  border: '1px solid var(--border-strong)',
  borderRadius: 'var(--r)',
  outline: 'none',
  transition: 'border-color .12s, box-shadow .12s',
}

export function TextInput({
  value,
  onChange,
  placeholder,
  type = 'text',
  icon: IconC,
  disabled,
  mono,
  style,
  onKeyDown,
  autoFocus,
  readOnly,
}: {
  value: string
  onChange?: (v: string) => void
  placeholder?: string
  type?: string
  icon?: IconComponent
  disabled?: boolean
  mono?: boolean
  style?: CSSProperties
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void
  autoFocus?: boolean
  readOnly?: boolean
}) {
  const [focus, setFocus] = useState(false)
  return (
    <div style={{ position: 'relative', display: 'flex', alignItems: 'center' }}>
      {IconC && (
        <span style={{ position: 'absolute', left: 10, color: 'var(--text-3)', display: 'flex' }}>
          <IconC size={16} />
        </span>
      )}
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        readOnly={readOnly}
        autoFocus={autoFocus}
        onKeyDown={onKeyDown}
        onChange={(e) => onChange && onChange(e.target.value)}
        onFocus={() => setFocus(true)}
        onBlur={() => setFocus(false)}
        style={{
          ...inputBase,
          paddingLeft: IconC ? 32 : 11,
          fontFamily: mono ? 'var(--mono)' : 'inherit',
          background: disabled || readOnly ? 'var(--surface-2)' : 'var(--surface)',
          color: disabled || readOnly ? 'var(--text-2)' : 'var(--text)',
          borderColor: focus ? 'var(--accent)' : 'var(--border-strong)',
          boxShadow: focus ? '0 0 0 3px var(--accent-weak)' : 'none',
          cursor: disabled ? 'not-allowed' : 'text',
          ...style,
        }}
      />
    </div>
  )
}

export function Textarea({
  value,
  onChange,
  placeholder,
  rows = 4,
  mono,
  style,
}: {
  value: string
  onChange?: (v: string) => void
  placeholder?: string
  rows?: number
  mono?: boolean
  style?: CSSProperties
}) {
  const [focus, setFocus] = useState(false)
  return (
    <textarea
      value={value}
      placeholder={placeholder}
      rows={rows}
      onChange={(e) => onChange && onChange(e.target.value)}
      onFocus={() => setFocus(true)}
      onBlur={() => setFocus(false)}
      style={{
        ...inputBase,
        resize: 'vertical',
        lineHeight: 1.6,
        fontFamily: mono ? 'var(--mono)' : 'inherit',
        fontSize: mono ? 12.5 : 13.5,
        borderColor: focus ? 'var(--accent)' : 'var(--border-strong)',
        boxShadow: focus ? '0 0 0 3px var(--accent-weak)' : 'none',
        ...style,
      }}
    />
  )
}

export type SelectOption = string | { value: string; label: string }

export function Select({
  value,
  onChange,
  options,
  disabled,
  style,
  placeholder,
  sm,
}: {
  value: string
  onChange?: (v: string) => void
  options: SelectOption[]
  disabled?: boolean
  style?: CSSProperties
  placeholder?: string
  sm?: boolean
}) {
  const [focus, setFocus] = useState(false)
  return (
    <div style={{ position: 'relative', display: 'inline-flex', width: '100%' }}>
      <select
        value={value}
        disabled={disabled}
        onChange={(e) => onChange && onChange(e.target.value)}
        onFocus={() => setFocus(true)}
        onBlur={() => setFocus(false)}
        style={{
          ...inputBase,
          appearance: 'none',
          padding: sm ? '5px 30px 5px 10px' : '7px 30px 7px 11px',
          fontSize: sm ? 12.5 : 13.5,
          background: disabled ? 'var(--surface-2)' : 'var(--surface)',
          color: disabled ? 'var(--text-2)' : 'var(--text)',
          borderColor: focus ? 'var(--accent)' : 'var(--border-strong)',
          boxShadow: focus ? '0 0 0 3px var(--accent-weak)' : 'none',
          cursor: disabled ? 'not-allowed' : 'pointer',
          ...style,
        }}
      >
        {placeholder && <option value="">{placeholder}</option>}
        {options.map((o) => {
          const val = typeof o === 'string' ? o : o.value
          const lab = typeof o === 'string' ? o : o.label
          return (
            <option key={val} value={val}>
              {lab}
            </option>
          )
        })}
      </select>
      <span style={{ position: 'absolute', right: 9, top: '50%', transform: 'translateY(-50%)', color: 'var(--text-3)', pointerEvents: 'none', display: 'flex' }}>
        <I.chevDown size={15} />
      </span>
    </div>
  )
}

export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
  sub,
}: {
  checked?: boolean
  onChange?: (v: boolean) => void
  label?: ReactNode
  disabled?: boolean
  sub?: ReactNode
}) {
  return (
    <label className="row" style={{ gap: 8, cursor: disabled ? 'not-allowed' : 'pointer', opacity: disabled ? 0.55 : 1, alignItems: sub ? 'flex-start' : 'center' }}>
      <span
        onClick={() => !disabled && onChange && onChange(!checked)}
        style={{
          width: 16,
          height: 16,
          flexShrink: 0,
          borderRadius: 4,
          marginTop: sub ? 2 : 0,
          border: `1.5px solid ${checked ? 'var(--accent)' : 'var(--border-strong)'}`,
          background: checked ? 'var(--accent)' : 'var(--surface)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: '#fff',
          transition: 'all .12s',
        }}
      >
        {checked && <I.check size={12} stroke={3} />}
      </span>
      {label && (
        <span className="col" style={{ gap: 1 }}>
          <span style={{ fontSize: 13.5 }}>{label}</span>
          {sub && <span className="t3 xs">{sub}</span>}
        </span>
      )}
    </label>
  )
}

export function Switch({ checked, onChange, disabled }: { checked?: boolean; onChange?: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      onClick={() => !disabled && onChange && onChange(!checked)}
      disabled={disabled}
      style={{
        width: 36,
        height: 20,
        borderRadius: 999,
        border: 'none',
        padding: 2,
        background: checked ? 'var(--accent)' : 'var(--border-strong)',
        display: 'flex',
        justifyContent: checked ? 'flex-end' : 'flex-start',
        transition: 'background .16s',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.6 : 1,
      }}
    >
      <span style={{ width: 16, height: 16, borderRadius: 999, background: '#fff', boxShadow: '0 1px 2px rgba(0,0,0,.2)', transition: 'all .16s' }} />
    </button>
  )
}

export function Radio({ checked, onChange, label }: { checked?: boolean; onChange?: () => void; label?: ReactNode }) {
  return (
    <label className="row" style={{ gap: 7, cursor: 'pointer' }} onClick={() => onChange && onChange()}>
      <span
        style={{
          width: 16,
          height: 16,
          borderRadius: 999,
          flexShrink: 0,
          border: `1.5px solid ${checked ? 'var(--accent)' : 'var(--border-strong)'}`,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          transition: 'all .12s',
        }}
      >
        {checked && <span style={{ width: 8, height: 8, borderRadius: 999, background: 'var(--accent)' }} />}
      </span>
      <span style={{ fontSize: 13.5 }}>{label}</span>
    </label>
  )
}

export function Segmented({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: SelectOption[] }) {
  return (
    <div className="row" style={{ background: 'var(--surface-2)', border: '1px solid var(--border)', borderRadius: 8, padding: 3, gap: 2, width: 'fit-content' }}>
      {options.map((o) => {
        const val = typeof o === 'string' ? o : o.value
        const lab = typeof o === 'string' ? o : o.label
        const on = value === val
        return (
          <button
            key={val}
            onClick={() => onChange(val)}
            style={{
              padding: '4px 13px',
              fontSize: 12.5,
              fontWeight: 550,
              borderRadius: 6,
              border: 'none',
              background: on ? 'var(--surface)' : 'transparent',
              color: on ? 'var(--text)' : 'var(--text-2)',
              boxShadow: on ? 'var(--shadow-sm)' : 'none',
              transition: 'all .12s',
            }}
          >
            {lab}
          </button>
        )
      })}
    </div>
  )
}
