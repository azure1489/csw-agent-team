import type { Config } from 'tailwindcss'

// 设计令牌映射：颜色/圆角/阴影/字体一律引用 globals.css 的 CSS 变量（像素级权威源 = 设计稿 styles.css）
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: 'var(--bg)',
        surface: { DEFAULT: 'var(--surface)', 2: 'var(--surface-2)', 3: 'var(--surface-3)' },
        sidebar: 'var(--sidebar)',
        border: { DEFAULT: 'var(--border)', 2: 'var(--border-2)', strong: 'var(--border-strong)' },
        text: { DEFAULT: 'var(--text)', 2: 'var(--text-2)', 3: 'var(--text-3)' },
        accent: {
          DEFAULT: 'var(--accent)',
          hover: 'var(--accent-hover)',
          press: 'var(--accent-press)',
          weak: 'var(--accent-weak)',
          'weak-2': 'var(--accent-weak-2)',
          text: 'var(--accent-text)',
        },
        green: { DEFAULT: 'var(--green)', bg: 'var(--green-bg)', bd: 'var(--green-bd)' },
        blue: { DEFAULT: 'var(--blue)', bg: 'var(--blue-bg)', bd: 'var(--blue-bd)' },
        amber: { DEFAULT: 'var(--amber)', bg: 'var(--amber-bg)', bd: 'var(--amber-bd)' },
        red: { DEFAULT: 'var(--red)', bg: 'var(--red-bg)', bd: 'var(--red-bd)' },
        gray: { DEFAULT: 'var(--gray)', bg: 'var(--gray-bg)', bd: 'var(--gray-bd)' },
        yellow: { DEFAULT: 'var(--yellow)', bg: 'var(--yellow-bg)', bd: 'var(--yellow-bd)' },
      },
      borderRadius: { sm: 'var(--r-sm)', DEFAULT: 'var(--r)', md: 'var(--r-md)', lg: 'var(--r-lg)' },
      boxShadow: { sm: 'var(--shadow-sm)', DEFAULT: 'var(--shadow)', pop: 'var(--shadow-pop)' },
      fontFamily: { sans: 'var(--font)', mono: 'var(--mono)' },
      maxWidth: { content: '1200px', 'content-wide': '1440px' },
      keyframes: {
        fadeIn: { from: { opacity: '0' }, to: { opacity: '1' } },
        popIn: { from: { opacity: '0', transform: 'translateY(8px) scale(.98)' }, to: { opacity: '1', transform: 'none' } },
        slideIn: { from: { transform: 'translateX(100%)' }, to: { transform: 'none' } },
        toastIn: { from: { opacity: '0', transform: 'translateY(12px)' }, to: { opacity: '1', transform: 'none' } },
      },
      animation: {
        fadeIn: 'fadeIn .15s ease',
        popIn: 'popIn .18s cubic-bezier(.2,.8,.3,1)',
        slideIn: 'slideIn .22s cubic-bezier(.2,.8,.3,1)',
        toastIn: 'toastIn .2s ease',
      },
    },
  },
  plugins: [],
} satisfies Config
