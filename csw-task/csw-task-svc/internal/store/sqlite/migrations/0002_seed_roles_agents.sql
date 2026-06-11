-- +goose Up
-- 角色（不写死、纯数据；管理/非管理两类）。Van = 管理·人工，无 token、结论由中枢代录。
INSERT INTO roles (code, name, is_management, is_human) VALUES
  ('editor',     '主编',       1, 0),
  ('van',        'Van',        1, 1),
  ('collector',  '情报收集员', 0, 0),
  ('researcher', '选题研究员', 0, 0),
  ('writer',     '文案',       0, 0),
  ('designer',   '设计师',     0, 0),
  ('publisher',  '发布员',     0, 0),
  ('scheduler',  '调度器',     0, 0);

-- 各角色一个 agent（一角色一 agent 假设）。Van 不建 agent（人工闸代录）。
-- token 不在迁移里发，用 adminctl token issue 签发（明文仅一次）。
INSERT INTO agents (role_code, name) VALUES
  ('editor',     '主编'),
  ('collector',  '情报收集员'),
  ('researcher', '选题研究员'),
  ('writer',     '文案'),
  ('designer',   '设计师'),
  ('publisher',  '发布员'),
  ('scheduler',  '调度器');

-- +goose Down
DELETE FROM agents WHERE role_code IN
  ('editor','collector','researcher','writer','designer','publisher','scheduler');
DELETE FROM roles WHERE code IN
  ('editor','van','collector','researcher','writer','designer','publisher','scheduler');
