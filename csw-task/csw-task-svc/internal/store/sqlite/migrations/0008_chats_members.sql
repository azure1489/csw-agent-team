-- +goose Up
-- 花名册/通讯录：群 + 成员（角色↔飞书 open_id 映射）。后台可增删改查，运行面 /roster 供 agent 实时读。
-- 统一容纳三类成员：mapped（映射到引擎 role_code 的 agent）/ reserved（群内有但未接入工作流的 bot）/ 人类 van（is_human）。
-- 数据转录自 skill/roster.json（CSW编辑部，7 mapped + 3 reserved）。

CREATE TABLE chats (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_key   TEXT NOT NULL UNIQUE,                 -- lark chat_id（oc_...）
  name       TEXT NOT NULL,
  note       TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

CREATE TABLE chat_members (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_id      INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL DEFAULT 'mapped' CHECK (kind IN ('mapped','reserved')),
  role_code    TEXT REFERENCES roles(code),        -- mapped 必填；reserved 为 NULL
  open_id      TEXT NOT NULL,                       -- 飞书 open_id（@ 用）
  user_id      TEXT,                                -- 仅 van 有
  display_name TEXT NOT NULL,                        -- 引擎角色名/展示名（如「文案」）
  bot_name     TEXT NOT NULL,                        -- 群内 bot 显示名（如「深度内容创作者」）
  is_human     INTEGER NOT NULL DEFAULT 0 CHECK (is_human IN (0,1)),
  sort         INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  CHECK (kind='reserved' OR role_code IS NOT NULL)
);
-- 一群内每个 role_code 唯一（仅 mapped）；reserved 之间靠 bot_name 唯一。
CREATE UNIQUE INDEX uq_chat_member_role    ON chat_members(chat_id, role_code) WHERE role_code IS NOT NULL;
CREATE UNIQUE INDEX uq_chat_member_botname ON chat_members(chat_id, bot_name)  WHERE role_code IS NULL;
CREATE INDEX idx_chat_members_chat ON chat_members(chat_id, sort);

-- seed：CSW编辑部
INSERT INTO chats (chat_key, name, note) VALUES
  ('oc_c7484358fbf366ba1c806e9d65db3103', 'CSW编辑部', '资讯日更编辑部群；群协作播报用');

-- mapped（7）+ reserved（3）。role_code 经 0002 seed 已存在。
INSERT INTO chat_members (chat_id, kind, role_code, open_id, user_id, display_name, bot_name, is_human, sort)
SELECT c.id, m.kind, m.role_code, m.open_id, m.user_id, m.display_name, m.bot_name, m.is_human, m.sort
FROM chats c
JOIN (
  SELECT 'mapped' kind, 'editor'     role_code, 'ou_e8b7e1e4076dbfbaf88c55078794b5eb' open_id, NULL user_id, '主编'       display_name, '主编'           bot_name, 0 is_human, 1 sort
  UNION ALL SELECT 'mapped','collector', 'ou_4df246d7bdd235a7337bfb453b38da5d', NULL, '情报收集员', '情报收集员',     0, 2
  UNION ALL SELECT 'mapped','researcher','ou_7d2e525dff82f92ab89741f92613353c', NULL, '选题研究员', '选题研究员',     0, 3
  UNION ALL SELECT 'mapped','writer',    'ou_98ef567d4e26d4d2ef9279da5304a758', NULL, '文案',       '深度内容创作者', 0, 4
  UNION ALL SELECT 'mapped','designer',  'ou_0d21381c313e0fa02be7e68d45c6f96e', NULL, '设计师',     '视觉设计师',     0, 5
  UNION ALL SELECT 'mapped','publisher', 'ou_2a362b7337a9a96717ef3c1f4ecd51fe', NULL, '发布员',     '发布运营员',     0, 6
  UNION ALL SELECT 'mapped','van',       'ou_1fca25427af132224142dd3abdda906d', 'ag83acf1', 'Van', 'Van',          1, 7
  UNION ALL SELECT 'reserved', NULL, 'ou_003deac0bece81514fca9cd9ba081139', NULL, '小红书图文作者', '小红书图文作者', 0, 8
  UNION ALL SELECT 'reserved', NULL, 'ou_5e4a9f33f31701a5dd674db4c9daa2dc', NULL, '合规版权审查员', '合规版权审查员', 0, 9
  UNION ALL SELECT 'reserved', NULL, 'ou_7b9ba1d4d09cfbee3ba3f89005db434f', NULL, '数据复盘师',     '数据复盘师',     0, 10
) m
WHERE c.chat_key = 'oc_c7484358fbf366ba1c806e9d65db3103';

-- +goose Down
DROP TABLE IF EXISTS chat_members;
DROP TABLE IF EXISTS chats;
