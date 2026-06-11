# csw-task-web · 任务流转管理后台（前端）

「任务流转服务」管理后台的 Web 前端：人类操作员经浏览器 **JWT 登录**，定义/管理工作流、角色成员与 token、后台用户，并只读监控运行实例。消费后端 `cmd/adminsrv` 的 `/admin/*` JSON API（与运行面分进程、分凭证）。

> 视觉权威源：`../../任务流转后台_设计稿/`（像素级复刻）+ `../../管理后台_功能与线框.md` §1–§15。
> 后端契约：`../csw-task-svc`（§6.1 后台 API）。本工程是 Monorepo `csw-task/` 的第三轮交付。

## 技术栈

- **React 18 + Vite 5 + TypeScript**（严格模式）
- **Tailwind v3** + 设计令牌（CSS 变量，迁自设计稿 `styles.css`）
- **Radix**（`@radix-ui/react-dialog`/`react-dropdown-menu`）作弹层/菜单的可达性底座；其余 ~24 个 UI 原语由设计稿手写组件移植为 TSX（混合策略：像素级忠实 + 关键交互有焦点陷阱）
- **React Router v6**（路由 + RBAC 守卫）· **TanStack Query**（数据获取/缓存）· **axios**（含 401 自动 refresh 重试）

## 目录（`src/`）

```
main.tsx App.tsx router.tsx            入口 + QueryClient/AuthProvider + 路由 + RequireRole 守卫
styles/globals.css                     设计令牌 :root + utility classes + keyframes
lib/   api.ts auth.tsx adapters.ts dag.ts constants.ts utils.ts
       └ api：端点 + 401→refresh→重试；auth：access 内存 + 启动静默续期；
         adapters：API↔编辑器形态（deps id↔code、relayed↔relayed_by_hub、self_check↔self_check_criteria）
components/
  ui/        Badge/Btn/Card/Field/TextInput/Select/Modal/Drawer/Menu/Toast… (移植 ui.jsx)
  shell/     Sidebar(NAV+RBAC)/Topbar/Content
  dag/       DagView（SVG 分层 + 贝塞尔边 + 环检测）
  common.tsx ActionTag/ProgressPill
pages/       Login Dashboard Workflows WorkflowEditor Members AdminUsers Runs RunDetail Audit Settings
  editor/    StageDrawer ValidateModal lib(OUTPUT_OPTS + runChecks)
hooks/       useRoles
types/       实体类型（API 形态 + 编辑器形态）
```

## 开发与构建

```bash
pnpm install
pnpm dev      # Vite dev server :5173（端口固定，对齐后端 CORS）
pnpm build    # tsc 类型检查 + vite 生产打包 → dist/
pnpm lint     # 仅 tsc --noEmit 类型检查
```

环境变量（`.env.development`）：`VITE_API_BASE=http://localhost:8081`（后端 `cmd/adminsrv` 地址）。

## 与后端联调

```bash
# 1) 后端（另一终端，见 ../csw-task-svc）
cd ../csw-task-svc && make build
export CSW_JWT_SECRET=devsecret CSW_ADMIN_CORS_ORIGIN=http://localhost:5173
./bin/adminctl migrate up
./bin/adminctl user create admin --role superadmin --password '***'
./bin/adminsrv          # :8081（管理后台 API）
./bin/server            # :8080（运行面；触发 run 后监控页有数据）

# 2) 前端
pnpm dev                # 浏览器打开 http://localhost:5173 → 用上面账号登录
```

鉴权：登录拿 access JWT（存内存）+ refresh（httpOnly cookie，刷新轮换）；access 过期由拦截器静默 `/admin/refresh` 续期重试。RBAC：写按钮 `operator+`，「后台用户 / 设置」`superadmin`，`viewer` 只读；菜单按角色显隐、直访受守卫重定向。

## 实现说明（与 §15 线框的取舍）

- **DAG 用自绘 SVG**（移植设计稿 `dag.jsx` 的分层 + 贝塞尔 + 环检测），未引入 React Flow——设计稿的 DAG 是只读预览，拖动排序发生在阶段表格而非画布，SVG 最忠实且零额外依赖。
- **激活校验混合**：编辑器 `ValidateModal` 用前端 `runChecks` 做即时预览（含 warn / 定位到阶段）；「确认激活」先 `PUT` 保存草稿、再调后端 `activate`（后端权威校验 + 归档旧版）。
- **激活版本可就地编辑**：draft 与 active 均可在编辑器直接改并保存（active 为就地热改，仅影响此后新触发的实例；已在跑的不受影响），仅归档/访客只读。active 保存走非阻断校验——后端 `PUT` 回带 `validation`，未过时以 `warn` toast 提示「触发前请修复」而不拦截。操作栏：draft=`保存草稿/校验/校验并激活`，active=`另存为新版本/校验/保存`。
- **组件混合策略**：设计稿手写原语移植为 TSX（内联样式 + 设计令牌，像素级），仅弹层/下拉用 Radix 底座补足可达性。
- 后端 `settings` 由环境变量驱动，设置页为只读展示（PUT 在后端为 no-op）。
```
