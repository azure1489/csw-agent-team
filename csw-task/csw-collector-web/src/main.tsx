import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { RouterProvider } from 'react-router-dom'

import { AuthProvider } from '@/lib/auth'
import { router } from '@/router'
import '@/styles/globals.css'

const qc = new QueryClient({
  defaultOptions: {
    queries: {
      // 一轮跑几十分钟，页面上的数据本来就不该每次聚焦都重拉
      refetchOnWindowFocus: false,
      staleTime: 15_000,
      retry: 1,
    },
  },
})

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <QueryClientProvider client={qc}>
      <AuthProvider>
        <RouterProvider router={router} />
      </AuthProvider>
    </QueryClientProvider>
  </React.StrictMode>,
)
