-- 杂志背景库（docs/情报收集员工作台_杂志背景库方案_20261007.md §6、清单契约 v1）。
--
-- kb_docs.kind 新增 magazine_item：刊译台清单里一张有译文的裁图一条。它**不是参考库四类**，
-- 判断路径（品牌路、全文路、向量路、补位）一律只看四类；杂志有自己的检索入口。
-- kb_cursors.source 新增 magazine（最近一次扫清单目录的时间）。
--
-- kb_doc_images：杂志条目 ↔ 裁图。LanceDB images 表刻意只按 blake3 存向量、不记归属，
-- 以图搜图命中要靠这张表回到条目；整本对账 / 清理按 book_key 查。
CREATE TABLE kb_doc_images (
  doc_id       INTEGER PRIMARY KEY REFERENCES kb_docs(id) ON DELETE CASCADE,
  book_key     TEXT NOT NULL,
  image_id     TEXT NOT NULL,             -- 刊译台 images.id（task_id:pdf_index:seq）
  blake3       TEXT NOT NULL,             -- 裁图内容哈希 = OSS 对象名 = LanceDB images 主键
  url          TEXT NOT NULL,
  page_blake3  TEXT NOT NULL DEFAULT '',
  page_url     TEXT NOT NULL DEFAULT '',
  pdf_index    INTEGER NOT NULL,
  printed_page INTEGER,
  bbox         TEXT NOT NULL DEFAULT '',  -- JSON [x0,y0,x1,y1]（PDF 点）
  page_size    TEXT NOT NULL DEFAULT '',  -- JSON [w,h]
  local_path   TEXT NOT NULL DEFAULT '',
  page_local_path TEXT NOT NULL DEFAULT '',
  category     TEXT NOT NULL DEFAULT '',
  -- 决策 8：has_content && !skipped && category != decor 才算向量
  vector_eligible INTEGER NOT NULL DEFAULT 0,
  -- 纯图向量（images 表）用哪个模型算过；空 = 没算。融合向量记在 kb_docs.embed_model
  image_embed_model TEXT NOT NULL DEFAULT '',
  -- 进全文索引时的词元串：contentless FTS5 删行要原样给回旧词元
  fts_tokens   TEXT NOT NULL DEFAULT ''
);
CREATE INDEX kb_doc_images_book ON kb_doc_images(book_key);
CREATE INDEX kb_doc_images_blake3 ON kb_doc_images(blake3);
