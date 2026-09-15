-- +goose Up
-- 审核记录绑定业务决定：decision_type（选题批准 / 全文批准 / 继续研究…；edit_pass 为主编定点编辑自动留痕）、
-- source_quote（Van 原话，人工闸必填）、items_json（条目范围，JSON 字符串数组）、expected_version（审核人以为的当前版本）。
ALTER TABLE reviews ADD COLUMN decision_type TEXT;
ALTER TABLE reviews ADD COLUMN source_quote TEXT;
ALTER TABLE reviews ADD COLUMN items_json TEXT;
ALTER TABLE reviews ADD COLUMN expected_version INTEGER;

-- +goose Down
ALTER TABLE reviews DROP COLUMN expected_version;
ALTER TABLE reviews DROP COLUMN items_json;
ALTER TABLE reviews DROP COLUMN source_quote;
ALTER TABLE reviews DROP COLUMN decision_type;
