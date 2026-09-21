import type { Config } from 'tailwindcss'

/**
 * 颜色一律引用 CSS 变量，不在这里写死十六进制——
 * 主题有三态（浅 / 深 / 跟随系统），写死就没法随 token 一起翻。
 * 权威源是 src/styles/globals.css，那里的权威源是界面设计稿。
 */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        ground: 'var(--ground)',
        bg: 'var(--bg)',
        surface: { DEFAULT: 'var(--surface)', 2: 'var(--surface-2)', 3: 'var(--surface-3)' },
        sidebar: 'var(--sidebar)',
        rule: 'var(--rule)',
        border: { DEFAULT: 'var(--border)', 2: 'var(--border-2)', strong: 'var(--border-strong)' },
        ink: 'var(--ink)',
        muted: 'var(--muted)',
        dim: 'var(--dim)',
        text: { DEFAULT: 'var(--text)', 2: 'var(--text-2)', 3: 'var(--text-3)' },
        accent: {
          DEFAULT: 'var(--accent)',
          soft: 'var(--accent-soft)',
          hover: 'var(--accent-hover)',
          press: 'var(--accent-press)',
          weak: 'var(--accent-weak)',
          'weak-2': 'var(--accent-weak-2)',
          text: 'var(--accent-text)',
        },
        ok: { DEFAULT: 'var(--ok)', soft: 'var(--ok-soft)' },
        warn: { DEFAULT: 'var(--warn)', soft: 'var(--warn-soft)' },
        bad: { DEFAULT: 'var(--bad)', soft: 'var(--bad-soft)' },
        // Van 的动作要一眼认出来，所以给它自己一个色
        van: { DEFAULT: 'var(--van)', soft: 'var(--van-soft)' },
        green: { DEFAULT: 'var(--green)', bg: 'var(--green-bg)', bd: 'var(--green-bd)' },
        blue: { DEFAULT: 'var(--blue)', bg: 'var(--blue-bg)', bd: 'var(--blue-bd)' },
        amber: { DEFAULT: 'var(--amber)', bg: 'var(--amber-bg)', bd: 'var(--amber-bd)' },
        red: { DEFAULT: 'var(--red)', bg: 'var(--red-bg)', bd: 'var(--red-bd)' },
        gray: { DEFAULT: 'var(--gray)', bg: 'var(--gray-bg)', bd: 'var(--gray-bd)' },
        yellow: { DEFAULT: 'var(--yellow)', bg: 'var(--yellow-bg)', bd: 'var(--yellow-bd)' },
      },
      borderRadius: { sm: 'var(--r-sm)', DEFAULT: 'var(--r)', md: 'var(--r-md)', lg: 'var(--r-lg)' },
      boxShadow: { sm: 'var(--shadow-sm)', DEFAULT: 'var(--shadow)', pop: 'var(--shadow-pop)' },
      fontFamily: { sans: 'var(--sans)', mono: 'var(--mono)' },
    },
  },
  plugins: [],
} satisfies Config
