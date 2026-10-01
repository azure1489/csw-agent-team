-- +goose Up
-- daily_news v10 草稿：01 的窗口口径改成「按原始披露时间判」，与《生产约定》对齐。
--
-- v9 / v10 的 01 都写着「窗口以首次进入来源库的时间为准……晚几天才被抓到，它属于被抓到的那一期」，
-- 与《资讯日更 · 生产约定》「以原始披露时间判窗口，转载时间不算；晚抓到的旧内容不得改成当天新闻」相反。
-- 09-30 r58 按首次入库收口，混进 341 条披露早于窗口的旧帖，整期作废；工作台 09-30 起按披露时间判，
-- 10-01 r59 主编按派工单（v9 快照）验收时又与工作台冲突。
--
-- 只改**还是草稿的 v10**：已激活的 v9 不动（改定义只走草稿 → 校验 → 由操作员激活），
-- 已开的 run 用的是开期时的快照，也不受影响。新库里 0041 先建出 v10 草稿，这里同样会改到。

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = REPLACE(instructions,
  '**窗口以首次进入来源库的时间为准**，不是以平台的发布时间——同一条贴文晚几天才被抓到，它属于被抓到的那一期。',
  '**窗口以原始披露时间为准**（左闭右开）：平台上的发布时间决定这条内容属于哪一期；首次进入来源库的时间只作抓取证据记录。晚几天才抓到的旧内容不算当期，不得改成当天新闻；没有披露时间的才按首次入库时间判，并在条目上写明。')
WHERE code = 'intake'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose Down

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = REPLACE(instructions,
  '**窗口以原始披露时间为准**（左闭右开）：平台上的发布时间决定这条内容属于哪一期；首次进入来源库的时间只作抓取证据记录。晚几天才抓到的旧内容不算当期，不得改成当天新闻；没有披露时间的才按首次入库时间判，并在条目上写明。',
  '**窗口以首次进入来源库的时间为准**，不是以平台的发布时间——同一条贴文晚几天才被抓到，它属于被抓到的那一期。')
WHERE code = 'intake'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd
