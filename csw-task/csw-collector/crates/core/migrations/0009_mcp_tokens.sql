-- 知识库 MCP 对外（远程 HTTP，docs/情报收集员工作台_知识库MCP对外方案_20261016.md §5）。
--
-- mcp_tokens：给其他机器上的 MCP 客户端（codex）的令牌。**库里只存 SHA-256**，明文只在签发时打印一次。
-- scopes 逗号分隔：magazine（杂志检索、以图搜图）、kb（参考库）。memory 不对外，签发时就拒。
-- mcp_calls：每次 /mcp 请求一行（含鉴权失败），与人工操作的 audit 表分开；保留 90 天。
CREATE TABLE mcp_tokens (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL,
  token_sha256 TEXT NOT NULL UNIQUE,
  scopes       TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  expires_at   TEXT,                   -- NULL = 不过期
  revoked_at   TEXT,
  last_used_at TEXT
);

CREATE TABLE mcp_calls (
  id         INTEGER PRIMARY KEY,
  token_id   INTEGER,                  -- 鉴权失败为 NULL
  ip         TEXT NOT NULL DEFAULT '',
  method     TEXT NOT NULL,            -- initialize / tools/list / tools/call / auth_failed …
  tool       TEXT NOT NULL DEFAULT '',
  args_brief TEXT NOT NULL DEFAULT '', -- 查询词前 200 字；图片只记来源与字节数
  ok         INTEGER NOT NULL,
  ms         INTEGER NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX mcp_calls_created ON mcp_calls(created_at DESC);
