// ============================================================
// UI primitives (Linear-style)
// ============================================================
const { useState, useEffect, useRef, useCallback, createContext, useContext, useMemo } = React;

// ---------- Status badge mapping ----------
const TONES = {
  green:  { c: "var(--green)",  bg: "var(--green-bg)",  bd: "var(--green-bd)" },
  blue:   { c: "var(--blue)",   bg: "var(--blue-bg)",   bd: "var(--blue-bd)" },
  amber:  { c: "var(--amber)",  bg: "var(--amber-bg)",  bd: "var(--amber-bd)" },
  red:    { c: "var(--red)",    bg: "var(--red-bg)",    bd: "var(--red-bd)" },
  gray:   { c: "var(--gray)",   bg: "var(--gray-bg)",   bd: "var(--gray-bd)" },
  yellow: { c: "var(--yellow)", bg: "var(--yellow-bg)", bd: "var(--yellow-bd)" },
  violet: { c: "var(--accent-text)", bg: "var(--accent-weak)", bd: "var(--accent-weak-2)" },
};

const STATUS = {
  // workflow
  draft:    { label: "草稿",   tone: "gray",  dot: true },
  active:   { label: "已激活", tone: "green", dot: true },
  archived: { label: "已归档", tone: "gray",  outline: true },
  // run
  run_active:  { label: "进行中", tone: "blue",  dot: true },
  done:        { label: "已完成", tone: "green", dot: true },
  paused:      { label: "已暂停", tone: "yellow",dot: true },
  aborted:     { label: "已中止", tone: "red",   dot: true },
  // task
  blocked:      { label: "未就绪",  tone: "gray" },
  ready:        { label: "待派工",  tone: "blue" },
  dispatched:   { label: "已派工",  tone: "blue" },
  in_progress:  { label: "进行中",  tone: "blue" },
  review:       { label: "审核中",  tone: "amber" },
  passed:       { label: "已通过",  tone: "green" },
  returned:     { label: "已退回",  tone: "red" },
  // token / agent / user
  token_active:  { label: "启用",   tone: "green", dot: true },
  revoked:       { label: "已吊销", tone: "gray" },
  expired:       { label: "已过期", tone: "yellow" },
  enabled:       { label: "启用",   tone: "green", dot: true },
  disabled:      { label: "禁用",   tone: "gray",  dot: true },
};

function Badge({ tone = "gray", children, dot, outline, sm, style }) {
  const t = TONES[tone] || TONES.gray;
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 5,
      padding: sm ? "1px 7px" : "2px 9px", borderRadius: 999,
      fontSize: sm ? 11 : 12, fontWeight: 550, lineHeight: 1.55,
      color: t.c, background: outline ? "transparent" : t.bg,
      border: `1px solid ${outline ? "var(--border-strong)" : t.bd}`,
      whiteSpace: "nowrap", ...style,
    }}>
      {dot && <span style={{ width: 6, height: 6, borderRadius: 999, background: t.c }} />}
      {children}
    </span>
  );
}

// status badge from a known status key
function StatusBadge({ status, map, sm }) {
  const key = map ? map(status) : status;
  const s = STATUS[key] || { label: status, tone: "gray" };
  return <Badge tone={s.tone} dot={s.dot} outline={s.outline} sm={sm}>{s.label}</Badge>;
}

// ---------- Buttons ----------
function Btn({ variant = "default", size = "md", icon: IconC, iconRight: IconR, children, onClick, disabled, type, style, title, full }) {
  const [hover, setHover] = useState(false);
  const [press, setPress] = useState(false);
  const pad = size === "sm" ? "5px 10px" : size === "lg" ? "9px 16px" : "6.5px 13px";
  const fs = size === "sm" ? 12.5 : 14;
  const variants = {
    primary: {
      bg: disabled ? "#b9b9e8" : press ? "var(--accent-press)" : hover ? "var(--accent-hover)" : "var(--accent)",
      color: "#fff", border: "transparent",
      shadow: disabled ? "none" : "0 1px 2px rgba(60,60,140,.25), inset 0 1px 0 rgba(255,255,255,.12)",
    },
    default: {
      bg: press ? "var(--surface-3)" : hover ? "var(--surface-2)" : "var(--surface)",
      color: "var(--text)", border: "var(--border-strong)", shadow: "var(--shadow-sm)",
    },
    ghost: {
      bg: hover ? "var(--surface-2)" : "transparent",
      color: "var(--text-2)", border: "transparent", shadow: "none",
    },
    danger: {
      bg: press ? "#c12f2f" : hover ? "#cf3636" : "var(--surface)",
      color: hover ? "#fff" : "var(--red)", border: hover ? "transparent" : "var(--red-bd)", shadow: "var(--shadow-sm)",
    },
    "danger-solid": {
      bg: press ? "#c12f2f" : hover ? "#cf3636" : "var(--red)",
      color: "#fff", border: "transparent", shadow: "0 1px 2px rgba(170,40,40,.25)",
    },
  };
  const v = variants[variant] || variants.default;
  return (
    <button type={type || "button"} onClick={disabled ? undefined : onClick} disabled={disabled} title={title}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => { setHover(false); setPress(false); }}
      onMouseDown={() => setPress(true)} onMouseUp={() => setPress(false)}
      style={{
        display: full ? "flex" : "inline-flex", width: full ? "100%" : undefined,
        alignItems: "center", justifyContent: "center", gap: 6, padding: pad,
        fontSize: fs, fontWeight: 550, borderRadius: var_r(size),
        background: v.bg, color: v.color, border: `1px solid ${v.border}`,
        boxShadow: v.shadow, cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.65 : 1, transition: "background .13s, box-shadow .13s, color .1s",
        whiteSpace: "nowrap", ...style,
      }}>
      {IconC && <IconC size={size === "sm" ? 14 : 16} />}
      {children}
      {IconR && <IconR size={size === "sm" ? 14 : 16} />}
    </button>
  );
}
const var_r = (s) => (s === "lg" ? "9px" : "7px");

function IconBtn({ icon: IconC, onClick, title, active, size = 18, danger, style }) {
  const [hover, setHover] = useState(false);
  return (
    <button onClick={onClick} title={title} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 30, height: 30, borderRadius: 7, border: "1px solid transparent",
        background: active ? "var(--surface-3)" : hover ? "var(--surface-2)" : "transparent",
        color: danger && hover ? "var(--red)" : active ? "var(--text)" : "var(--text-2)",
        transition: "background .12s, color .12s", ...style,
      }}>
      <IconC size={size} />
    </button>
  );
}

// ---------- Cards ----------
function Card({ children, style, pad = 0, hover, onClick }) {
  const [h, setH] = useState(false);
  return (
    <div onClick={onClick} onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{
        background: "var(--surface)", border: "1px solid var(--border)",
        borderRadius: "var(--r-md)", boxShadow: hover && h ? "var(--shadow)" : "var(--shadow-sm)",
        padding: pad, transition: "box-shadow .15s, border-color .15s, transform .15s",
        cursor: onClick ? "pointer" : undefined,
        borderColor: hover && h ? "var(--border-2)" : "var(--border)",
        transform: hover && h && onClick ? "translateY(-1px)" : "none", ...style,
      }}>
      {children}
    </div>
  );
}

function SectionCard({ title, subtitle, actions, children, bodyStyle, headStyle, noPad }) {
  return (
    <Card>
      {(title || actions) && (
        <div className="row between" style={{ padding: "13px 16px", borderBottom: "1px solid var(--border)", ...headStyle }}>
          <div className="col" style={{ gap: 1 }}>
            {title && <div style={{ fontWeight: 600, fontSize: 14 }}>{title}</div>}
            {subtitle && <div className="t2 sm">{subtitle}</div>}
          </div>
          {actions && <div className="row gap8">{actions}</div>}
        </div>
      )}
      <div style={{ padding: noPad ? 0 : 16, ...bodyStyle }}>{children}</div>
    </Card>
  );
}

// ---------- Page header ----------
function PageHeader({ title, desc, actions, breadcrumb, badge }) {
  return (
    <div className="row between wrap" style={{ gap: 16, marginBottom: 20 }}>
      <div className="col" style={{ gap: 4 }}>
        {breadcrumb}
        <div className="row gap12" style={{ alignItems: "center" }}>
          <h1 style={{ margin: 0, fontSize: 21, fontWeight: 650, letterSpacing: "-.01em" }}>{title}</h1>
          {badge}
        </div>
        {desc && <div className="t2" style={{ fontSize: 13.5, maxWidth: 720 }}>{desc}</div>}
      </div>
      {actions && <div className="row gap8" style={{ flexShrink: 0 }}>{actions}</div>}
    </div>
  );
}

// ---------- Form fields ----------
function Field({ label, hint, required, children, style, htmlFor }) {
  return (
    <div className="col" style={{ gap: 6, ...style }}>
      {label && (
        <label htmlFor={htmlFor} style={{ fontSize: 13, fontWeight: 550, color: "var(--text)" }}>
          {label} {required && <span style={{ color: "var(--red)" }}>*</span>}
        </label>
      )}
      {children}
      {hint && <div className="t3 xs" style={{ lineHeight: 1.45 }}>{hint}</div>}
    </div>
  );
}

const inputBase = {
  width: "100%", padding: "7px 11px", fontSize: 13.5, color: "var(--text)",
  background: "var(--surface)", border: "1px solid var(--border-strong)",
  borderRadius: "var(--r)", outline: "none", transition: "border-color .12s, box-shadow .12s",
};

function TextInput({ value, onChange, placeholder, type = "text", icon: IconC, disabled, mono, style, onKeyDown, autoFocus, readOnly }) {
  const [focus, setFocus] = useState(false);
  return (
    <div style={{ position: "relative", display: "flex", alignItems: "center" }}>
      {IconC && <span style={{ position: "absolute", left: 10, color: "var(--text-3)", display: "flex" }}><IconC size={16} /></span>}
      <input type={type} value={value} placeholder={placeholder} disabled={disabled} readOnly={readOnly}
        autoFocus={autoFocus} onKeyDown={onKeyDown}
        onChange={(e) => onChange && onChange(e.target.value)}
        onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
        style={{
          ...inputBase, paddingLeft: IconC ? 32 : 11,
          fontFamily: mono ? "var(--mono)" : "inherit",
          background: disabled || readOnly ? "var(--surface-2)" : "var(--surface)",
          color: disabled || readOnly ? "var(--text-2)" : "var(--text)",
          borderColor: focus ? "var(--accent)" : "var(--border-strong)",
          boxShadow: focus ? "0 0 0 3px var(--accent-weak)" : "none",
          cursor: disabled ? "not-allowed" : "text", ...style,
        }} />
    </div>
  );
}

function Textarea({ value, onChange, placeholder, rows = 4, mono, style }) {
  const [focus, setFocus] = useState(false);
  return (
    <textarea value={value} placeholder={placeholder} rows={rows}
      onChange={(e) => onChange && onChange(e.target.value)}
      onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
      style={{
        ...inputBase, resize: "vertical", lineHeight: 1.6,
        fontFamily: mono ? "var(--mono)" : "inherit", fontSize: mono ? 12.5 : 13.5,
        borderColor: focus ? "var(--accent)" : "var(--border-strong)",
        boxShadow: focus ? "0 0 0 3px var(--accent-weak)" : "none", ...style,
      }} />
  );
}

function Select({ value, onChange, options, disabled, style, placeholder, sm }) {
  const [focus, setFocus] = useState(false);
  return (
    <div style={{ position: "relative", display: "inline-flex", width: "100%" }}>
      <select value={value} disabled={disabled} onChange={(e) => onChange && onChange(e.target.value)}
        onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
        style={{
          ...inputBase, appearance: "none", paddingRight: 30,
          padding: sm ? "5px 30px 5px 10px" : "7px 30px 7px 11px",
          fontSize: sm ? 12.5 : 13.5,
          background: disabled ? "var(--surface-2)" : "var(--surface)",
          color: disabled ? "var(--text-2)" : "var(--text)",
          borderColor: focus ? "var(--accent)" : "var(--border-strong)",
          boxShadow: focus ? "0 0 0 3px var(--accent-weak)" : "none",
          cursor: disabled ? "not-allowed" : "pointer", ...style,
        }}>
        {placeholder && <option value="">{placeholder}</option>}
        {options.map((o) => {
          const val = typeof o === "string" ? o : o.value;
          const lab = typeof o === "string" ? o : o.label;
          return <option key={val} value={val}>{lab}</option>;
        })}
      </select>
      <span style={{ position: "absolute", right: 9, top: "50%", transform: "translateY(-50%)", color: "var(--text-3)", pointerEvents: "none", display: "flex" }}>
        <I.chevDown size={15} />
      </span>
    </div>
  );
}

function Checkbox({ checked, onChange, label, disabled, sub }) {
  return (
    <label className="row" style={{ gap: 8, cursor: disabled ? "not-allowed" : "pointer", opacity: disabled ? 0.55 : 1, alignItems: sub ? "flex-start" : "center" }}>
      <span onClick={() => !disabled && onChange && onChange(!checked)}
        style={{
          width: 16, height: 16, flexShrink: 0, borderRadius: 4, marginTop: sub ? 2 : 0,
          border: `1.5px solid ${checked ? "var(--accent)" : "var(--border-strong)"}`,
          background: checked ? "var(--accent)" : "var(--surface)",
          display: "flex", alignItems: "center", justifyContent: "center",
          color: "#fff", transition: "all .12s",
        }}>
        {checked && <I.check size={12} stroke={3} />}
      </span>
      {label && (
        <span className="col" style={{ gap: 1 }}>
          <span style={{ fontSize: 13.5 }}>{label}</span>
          {sub && <span className="t3 xs">{sub}</span>}
        </span>
      )}
    </label>
  );
}

function Switch({ checked, onChange, disabled }) {
  return (
    <button onClick={() => !disabled && onChange && onChange(!checked)} disabled={disabled}
      style={{
        width: 36, height: 20, borderRadius: 999, border: "none", padding: 2,
        background: checked ? "var(--accent)" : "var(--border-strong)",
        display: "flex", justifyContent: checked ? "flex-end" : "flex-start",
        transition: "background .16s", cursor: disabled ? "not-allowed" : "pointer", opacity: disabled ? .6 : 1,
      }}>
      <span style={{ width: 16, height: 16, borderRadius: 999, background: "#fff", boxShadow: "0 1px 2px rgba(0,0,0,.2)", transition: "all .16s" }} />
    </button>
  );
}

function Radio({ checked, onChange, label }) {
  return (
    <label className="row" style={{ gap: 7, cursor: "pointer" }} onClick={() => onChange && onChange()}>
      <span style={{
        width: 16, height: 16, borderRadius: 999, flexShrink: 0,
        border: `1.5px solid ${checked ? "var(--accent)" : "var(--border-strong)"}`,
        display: "flex", alignItems: "center", justifyContent: "center", transition: "all .12s",
      }}>
        {checked && <span style={{ width: 8, height: 8, borderRadius: 999, background: "var(--accent)" }} />}
      </span>
      <span style={{ fontSize: 13.5 }}>{label}</span>
    </label>
  );
}

function Segmented({ value, onChange, options }) {
  return (
    <div className="row" style={{ background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 8, padding: 3, gap: 2, width: "fit-content" }}>
      {options.map((o) => {
        const val = typeof o === "string" ? o : o.value;
        const lab = typeof o === "string" ? o : o.label;
        const on = value === val;
        return (
          <button key={val} onClick={() => onChange(val)}
            style={{
              padding: "4px 13px", fontSize: 12.5, fontWeight: 550, borderRadius: 6, border: "none",
              background: on ? "var(--surface)" : "transparent", color: on ? "var(--text)" : "var(--text-2)",
              boxShadow: on ? "var(--shadow-sm)" : "none", transition: "all .12s",
            }}>{lab}</button>
        );
      })}
    </div>
  );
}

// ---------- Modal ----------
function Modal({ open, onClose, title, subtitle, children, footer, width = 480, icon: IconC, iconTone }) {
  useEffect(() => {
    if (!open) return;
    const h = (e) => e.key === "Escape" && onClose && onClose();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);
  if (!open) return null;
  const it = TONES[iconTone] || TONES.violet;
  return (
    <div onMouseDown={onClose} style={{
      position: "fixed", inset: 0, background: "rgba(20,20,32,.42)", backdropFilter: "blur(1.5px)",
      display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1000, animation: "fadeIn .15s",
    }}>
      <div onMouseDown={(e) => e.stopPropagation()} style={{
        background: "var(--surface)", borderRadius: "var(--r-lg)", boxShadow: "var(--shadow-pop)",
        width, maxWidth: "calc(100vw - 40px)", maxHeight: "calc(100vh - 60px)", overflow: "hidden",
        display: "flex", flexDirection: "column", animation: "popIn .18s cubic-bezier(.2,.8,.3,1)",
      }}>
        {(title || IconC) && (
          <div className="row" style={{ gap: 12, padding: "18px 20px 14px", alignItems: "flex-start" }}>
            {IconC && (
              <span style={{ width: 34, height: 34, borderRadius: 9, background: it.bg, color: it.c, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
                <IconC size={19} />
              </span>
            )}
            <div className="col grow" style={{ gap: 2 }}>
              <div style={{ fontSize: 15.5, fontWeight: 620 }}>{title}</div>
              {subtitle && <div className="t2 sm">{subtitle}</div>}
            </div>
            <IconBtn icon={I.x} onClick={onClose} />
          </div>
        )}
        <div style={{ padding: title ? "0 20px 4px" : 20, overflowY: "auto" }}>{children}</div>
        {footer && (
          <div className="row between" style={{ gap: 10, padding: "16px 20px", borderTop: "1px solid var(--border)", marginTop: 8 }}>
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- Drawer (right) ----------
function Drawer({ open, onClose, title, subtitle, children, footer, width = 540, badge }) {
  useEffect(() => {
    if (!open) return;
    const h = (e) => e.key === "Escape" && onClose && onClose();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, background: "rgba(20,20,32,.42)", backdropFilter: "blur(1.5px)", zIndex: 1000, animation: "fadeIn .15s", display: "flex", justifyContent: "flex-end" }}>
      <div onMouseDown={(e) => e.stopPropagation()} style={{
        background: "var(--surface)", width, maxWidth: "calc(100vw - 32px)", height: "100%",
        boxShadow: "var(--shadow-pop)", display: "flex", flexDirection: "column", animation: "slideIn .22s cubic-bezier(.2,.8,.3,1)",
      }}>
        <div className="row between" style={{ padding: "16px 22px", borderBottom: "1px solid var(--border)", gap: 12 }}>
          <div className="col" style={{ gap: 2 }}>
            <div className="row gap8">
              <span style={{ fontSize: 15.5, fontWeight: 620 }}>{title}</span>
              {badge}
            </div>
            {subtitle && <div className="t2 sm">{subtitle}</div>}
          </div>
          <IconBtn icon={I.x} onClick={onClose} />
        </div>
        <div className="grow" style={{ overflowY: "auto", padding: 22 }}>{children}</div>
        {footer && <div className="row between" style={{ gap: 10, padding: "14px 22px", borderTop: "1px solid var(--border)" }}>{footer}</div>}
      </div>
    </div>
  );
}

// ---------- Confirm dialog ----------
function ConfirmDialog({ open, onClose, onConfirm, title, message, confirmText = "确认", danger, icon }) {
  return (
    <Modal open={open} onClose={onClose} title={title} width={420} icon={icon || I.warn} iconTone={danger ? "red" : "violet"}
      footer={<>
        <span />
        <div className="row gap8">
          <Btn variant="default" onClick={onClose}>取消</Btn>
          <Btn variant={danger ? "danger-solid" : "primary"} onClick={() => { onConfirm && onConfirm(); }}>{confirmText}</Btn>
        </div>
      </>}>
      <div className="t2" style={{ fontSize: 13.5, lineHeight: 1.6, paddingBottom: 6 }}>{message}</div>
    </Modal>
  );
}

// ---------- Empty state ----------
function EmptyState({ icon: IconC, title, desc, action, compact }) {
  return (
    <div className="col center" style={{ alignItems: "center", textAlign: "center", padding: compact ? "32px 20px" : "56px 20px", gap: 4 }}>
      <div style={{ width: 52, height: 52, borderRadius: 14, background: "var(--surface-2)", border: "1px solid var(--border)", display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-3)", marginBottom: 8 }}>
        {IconC && <IconC size={24} />}
      </div>
      <div style={{ fontWeight: 600, fontSize: 14.5 }}>{title}</div>
      {desc && <div className="t2 sm" style={{ maxWidth: 320, lineHeight: 1.55 }}>{desc}</div>}
      {action && <div style={{ marginTop: 12 }}>{action}</div>}
    </div>
  );
}

// ---------- Avatar ----------
function Avatar({ name, size = 28, tone }) {
  const ch = (name || "?").slice(0, 1);
  const palette = ["#5b5bd6", "#3667d6", "#1f9d57", "#b5730a", "#d63b3b", "#8b5cf6", "#0891b2"];
  const idx = (name || "").split("").reduce((a, c) => a + c.charCodeAt(0), 0) % palette.length;
  const bg = tone || palette[idx];
  return (
    <span style={{ width: size, height: size, borderRadius: 7, background: bg, color: "#fff", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: size * 0.42, fontWeight: 600, flexShrink: 0 }}>
      {ch}
    </span>
  );
}

// ---------- Tabs ----------
function Tabs({ tabs, value, onChange }) {
  return (
    <div className="row" style={{ gap: 2, borderBottom: "1px solid var(--border)" }}>
      {tabs.map((t) => {
        const val = typeof t === "string" ? t : t.value;
        const lab = typeof t === "string" ? t : t.label;
        const on = value === val;
        return (
          <button key={val} onClick={() => onChange(val)}
            style={{
              padding: "9px 14px", fontSize: 13.5, fontWeight: 550, border: "none", background: "transparent",
              color: on ? "var(--text)" : "var(--text-2)", borderBottom: `2px solid ${on ? "var(--accent)" : "transparent"}`,
              marginBottom: -1, transition: "color .12s",
            }}>{lab}</button>
        );
      })}
    </div>
  );
}

// ---------- Toast ----------
function ToastHost() {
  const [items, setItems] = useState([]);
  useEffect(() => {
    const h = (e) => {
      const id = Math.random().toString(36).slice(2);
      const { msg, tone = "default", desc } = e.detail;
      setItems((x) => [...x, { id, msg, tone, desc }]);
      setTimeout(() => setItems((x) => x.filter((i) => i.id !== id)), 3400);
    };
    window.addEventListener("app-toast", h);
    return () => window.removeEventListener("app-toast", h);
  }, []);
  const iconFor = { success: I.checkCircle, error: I.xCircle, warn: I.warn, default: I.bolt };
  const colorFor = { success: "var(--green)", error: "var(--red)", warn: "var(--amber)", default: "var(--accent)" };
  return (
    <div style={{ position: "fixed", bottom: 22, left: "50%", transform: "translateX(-50%)", zIndex: 2000, display: "flex", flexDirection: "column", gap: 8, alignItems: "center" }}>
      {items.map((it) => {
        const IconC = iconFor[it.tone] || iconFor.default;
        return (
          <div key={it.id} style={{
            display: "flex", alignItems: "flex-start", gap: 10, background: "#1f1f27", color: "#fff",
            padding: "11px 16px", borderRadius: 9, boxShadow: "var(--shadow-pop)", animation: "toastIn .2s", maxWidth: 460,
          }}>
            <span style={{ color: colorFor[it.tone], display: "flex", marginTop: 1 }}><IconC size={18} /></span>
            <div className="col" style={{ gap: 1 }}>
              <span style={{ fontSize: 13.5, fontWeight: 550 }}>{it.msg}</span>
              {it.desc && <span style={{ fontSize: 12, color: "#b5b5c0" }}>{it.desc}</span>}
            </div>
          </div>
        );
      })}
    </div>
  );
}
const toast = (msg, tone, desc) => window.dispatchEvent(new CustomEvent("app-toast", { detail: { msg, tone, desc } }));

// ---------- Dropdown menu ----------
function Menu({ trigger, items, align = "right", width = 168 }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return;
    const h = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    window.addEventListener("mousedown", h);
    return () => window.removeEventListener("mousedown", h);
  }, [open]);
  return (
    <div ref={ref} style={{ position: "relative", display: "inline-flex" }}>
      <span onClick={() => setOpen((o) => !o)}>{trigger}</span>
      {open && (
        <div style={{
          position: "absolute", top: "calc(100% + 5px)", [align]: 0, width, zIndex: 50,
          background: "var(--surface)", border: "1px solid var(--border-2)", borderRadius: 9,
          boxShadow: "var(--shadow-pop)", padding: 5, animation: "popIn .14s",
        }}>
          {items.map((it, i) => it.divider ? (
            <div key={i} style={{ height: 1, background: "var(--border)", margin: "5px 0" }} />
          ) : (
            <button key={i} onClick={() => { setOpen(false); it.onClick && it.onClick(); }}
              style={{
                display: "flex", alignItems: "center", gap: 9, width: "100%", padding: "7px 9px",
                fontSize: 13, fontWeight: 500, border: "none", borderRadius: 6, background: "transparent",
                color: it.danger ? "var(--red)" : "var(--text)", textAlign: "left",
              }}
              onMouseEnter={(e) => e.currentTarget.style.background = it.danger ? "var(--red-bg)" : "var(--surface-2)"}
              onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}>
              {it.icon && <it.icon size={15} />}{it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------- Markdown (lightweight) ----------
function MarkdownView({ text }) {
  const html = useMemo(() => renderMd(text || ""), [text]);
  return <div className="md-body" style={{ fontSize: 13, lineHeight: 1.65, color: "var(--text)" }} dangerouslySetInnerHTML={{ __html: html }} />;
}
function renderMd(src) {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const lines = src.split("\n");
  let out = "", inUl = false, inCode = false;
  const inline = (s) => esc(s)
    .replace(/`([^`]+)`/g, '<code style="background:var(--surface-3);padding:1px 5px;border-radius:4px;font-family:var(--mono);font-size:.86em">$1</code>')
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  for (let ln of lines) {
    if (ln.trim().startsWith("```")) { inCode = !inCode; out += inCode ? '<pre style="background:var(--surface-2);border:1px solid var(--border);border-radius:7px;padding:10px 12px;overflow:auto;font-family:var(--mono);font-size:12px;margin:8px 0">' : "</pre>"; continue; }
    if (inCode) { out += esc(ln) + "\n"; continue; }
    const cb = ln.match(/^\s*-\s*\[( |x)\]\s+(.*)$/);
    if (cb) { if (!inUl) { out += '<ul style="list-style:none;padding-left:2px;margin:6px 0">'; inUl = true; } out += `<li style="display:flex;gap:7px;align-items:flex-start;padding:2px 0"><span style="margin-top:1px;color:${cb[1] === "x" ? "var(--green)" : "var(--text-3)"}">${cb[1] === "x" ? "☑" : "☐"}</span><span>${inline(cb[2])}</span></li>`; continue; }
    const h = ln.match(/^(#{1,4})\s+(.*)$/);
    if (h) { if (inUl) { out += "</ul>"; inUl = false; } const lv = h[1].length; const sz = [17, 15, 13.5, 13][lv - 1]; out += `<div style="font-weight:650;font-size:${sz}px;margin:12px 0 5px">${inline(h[2])}</div>`; continue; }
    const li = ln.match(/^\s*(\d+\.|[-*])\s+(.*)$/);
    if (li) { if (!inUl) { out += '<ul style="padding-left:18px;margin:5px 0">'; inUl = true; } out += `<li style="padding:1px 0">${inline(li[2])}</li>`; continue; }
    if (inUl) { out += "</ul>"; inUl = false; }
    if (ln.trim() === "") { out += '<div style="height:6px"></div>'; continue; }
    out += `<div style="margin:2px 0">${inline(ln)}</div>`;
  }
  if (inUl) out += "</ul>";
  if (inCode) out += "</pre>";
  return out;
}

Object.assign(window, {
  Badge, StatusBadge, STATUS, TONES, Btn, IconBtn, Card, SectionCard, PageHeader,
  Field, TextInput, Textarea, Select, Checkbox, Switch, Radio, Segmented,
  Modal, Drawer, ConfirmDialog, EmptyState, Avatar, Tabs, ToastHost, toast, Menu, MarkdownView,
});
