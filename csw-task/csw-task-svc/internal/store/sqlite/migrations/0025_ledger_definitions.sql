-- +goose Up
-- 指标快照保存指标定义与口径来源（平台对各字段的说明，或人工导出文件名），与原值、异常标记并存；
-- 账号级趋势与单篇快照分表保存，各自保留时间区间与定义，不把单篇相加冒充账号数据。
ALTER TABLE ledger_metrics_snapshots ADD COLUMN definitions_json TEXT;
ALTER TABLE ledger_account_snapshots ADD COLUMN definitions_json TEXT;

-- +goose Down
ALTER TABLE ledger_account_snapshots DROP COLUMN definitions_json;
ALTER TABLE ledger_metrics_snapshots DROP COLUMN definitions_json;
