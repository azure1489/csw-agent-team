# csw-collector-web · 情报收集员工作台前端

十一页 + Van 模式。后端是 `csw-task/csw-collector`（Rust，axum）。

- 界面视觉权威源：`docs/tools/plan_pages/build_mockup.py` 生成的界面设计稿。
- 接口契约：`csw-task/csw-collector/API.md`；数据类型由
  `csw-collector schema -o src/contract.json` 导出，**两边不各写一份**。

## 与 csw-task-web 的关系

底座是复制过来的（`components/ui/*`、`icons.tsx`、`lib/utils.ts`、构建配置），
不引 pnpm workspace——那会动到已经上线的管理后台。复制来的组件用的是那边的 token 名，
`globals.css` 末尾有一段兼容别名把两套名字对上；**新代码一律用新名字**。

## 最大的不同：没有 token

管理后台把引擎的 access JWT 存在内存里、401 自己续期。这里不是：

- 浏览器只拿一枚**不透明的 HttpOnly 会话 cookie**，引擎的 access 与 refresh
  只留在采集服务的服务端会话表里。
- 前端不续期、不存 token、也读不到它。最坏情况（XSS）丢的只是一个随时能作废的会话 id，
  不是引擎的管理身份。
- 写请求要回填 `X-CSW-CSRF` 头——`SameSite=Lax` 挡不住表单式的跨站 POST。

## 角色

`superadmin` > `operator` > `viewer`，另有 `van`。

`van` 是引擎里**没有**的角色：配置里列出的 viewer 用户名映射而来，`engine_role` 仍是
`viewer`，界面上明写出来。Van 与 viewer 同档——她只读加勾选，**勾选只写本地、不回写引擎**，
进不进评选由主编代录。

前端守卫只是体验，真正的权限在服务端：每个写接口都自己查会话与角色。

## 命令

```bash
pnpm install
pnpm dev     # Vite :5174（避开管理后台的 5173），/api 代理到本地 127.0.0.1:8090
pnpm build   # tsc --noEmit + vite build
pnpm lint    # 只有 tsc --noEmit
```

会话是 HttpOnly cookie，必须同源才带得上——所以开发走代理，不填绝对地址。
