-- +goose Up
-- 发布记录加两列，供「五类对照材料」里的前两类分得开。
--
-- publish_evidence：凭什么说它发布了——公开页地址、抓取时间、抓到的标题。
--   现在 state='published' 只是同步器的一个判断，回头核不了。
-- is_reference：这条是不是「范例」。范例是编辑部认可的写法样板，
--   与「正式已发布」是两类对照材料，不能混成一类。
ALTER TABLE ledger_published_posts ADD COLUMN publish_evidence TEXT;
ALTER TABLE ledger_published_posts ADD COLUMN is_reference INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_ledger_posts_reference ON ledger_published_posts(is_reference, published_at);

-- +goose Down
DROP INDEX IF EXISTS idx_ledger_posts_reference;
ALTER TABLE ledger_published_posts DROP COLUMN is_reference;
ALTER TABLE ledger_published_posts DROP COLUMN publish_evidence;
