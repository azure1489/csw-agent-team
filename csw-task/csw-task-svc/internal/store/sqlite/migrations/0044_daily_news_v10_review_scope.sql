-- +goose Up
-- daily_news v10 草稿：01 主编审核只查格式与材料，内容意见随通过意见交 02 / 03（2026-10-10 分工调整，
-- 见 docs/情报收集员_01审核分工_20261010.md）。同时把 01 的窗口说法改成每天 06:00、固定 24 小时（生产约定 v1.4）。
-- 只改还是草稿的 v10；已激活的 v9 与已开的 run（开期快照）不受影响。

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = '## 验收标准（主编格式核对）
**本闸只查格式与材料是否齐全，不改贴文的判断与去向**（2026-10-10 分工调整）。五项都符合就通过，哪怕对内容有不同看法。
1. **包结构**：index.md、images/、trace/ 下的必备文件齐全，首页链接可打开。
2. **窗口与采集**：窗口起止与派工一致；采集轮已上报；接口翻到最后一页；补采有记录。先看 `GET /runs/{run_id}/intake-check`。
3. **全部判过**：窗口内每条候选都有判断；没看到实图的标待核。
4. **材料齐全**：首批卡与推荐 / 备选附原图；原始发布时间与首次入库时间都写了；原文与译文都在。
5. **对账一致**：包与引擎逐键一致；intake-check 无红灯。

- **只有不符合这五项才退回**，退回意见写明是第几项、哪个文件或条目；工作台返工只补材料、重新导出，不改判断。
- **对内容的意见写在通过意见里**：档位、继续还是停止、哪几条是同一件事、代表图、译名。引擎会把它随 02 派工单带给选题研究员，由 02 / 03 采用，并沉淀进选题记忆；不为内容意见退回 01。'
WHERE code = 'intake'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET instructions = REPLACE(REPLACE(instructions,
  '窗口按生产约定：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00。', '窗口按生产约定：每天 06:00 由定时器开期，窗口固定为前一天 06:00 至当天 06:00（周末、节假日照跑）。'),
  '### 先交首批
凑够一批推荐选题（一般 4–8 个）就先提交，等主编校准，再把其余判完的以**补件**追加，不重新提交整包。
', '### 先交首批
凑够一批推荐选题（一般 4–8 个）就提交，交主编**核对格式与材料**（五项清单）。主编对内容的意见写在通过意见里，随 02 派工单交选题研究员采用；**工作台不按内容意见返工**，只在格式或材料不全被退回时补材料、重新导出。')
WHERE code = 'intake'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_gates SET name = '主编格式核对'
WHERE name = '主编首批校准'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = REPLACE(instructions, '01 的主编首批校准结论是你的起点，方向已被主编停掉的对象不再投入。', '**派工单里的【上游主编意见】是你的起点**：那是主编在 01 通过时写的内容意见（不纳入、同一件事、代表图、译名等）。主编说不纳入的不进候选，说是同一件事的合并成一题；主编在本派工单里另写的「这几条不纳入」同样照办。不采用的逐条写明理由。'),
  acceptance = REPLACE(acceptance, '- 每条都能对回情报记录与原文链接。',
    '- 每条都能对回情报记录与原文链接。' || char(10) || '- 已采用派工单里【上游主编意见】的内容意见，或逐条写明不采用的理由。')
WHERE code = 'shortlist'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose Down

-- +goose StatementBegin
UPDATE workflow_stages SET acceptance = '## 验收标准（主编首批校准）
- 先看 `GET /runs/{run_id}/intake-check`：每条都判、待核一致、必扫来源有轮次、窗口合规。
- **审阅覆盖是代码算出来的，不是自我声明。**
- 推荐的条目说得清「为什么值得推荐给中国户外潮流读者」，依据能在正文或图里核到。**发现编造的依据或无证据的前后对比直接退回。**
- 查重结论与证据一致：列出命中的文章与状态；正文缺失的写「查重未确认」；生成稿没被当成已发。
- 待核的写明缺什么、谁处理、下一步；没读到的没有被当成没价值。
- **推荐按选题算**：同一产品或事件不占多个推荐位；推荐贴文数与独立选题数分开给出。
- 首批条数够判断方向（一般 4–8 个选题），是按判断结论排出来的推荐，不是单一对象的深挖包。
- 主编在本闸给出方向：哪些继续、哪些停止、还缺哪类来源；退回时写明方向与位置。',
  instructions = REPLACE(REPLACE(instructions, '窗口按生产约定：每天 06:00 由定时器开期，窗口固定为前一天 06:00 至当天 06:00（周末、节假日照跑）。', '窗口按生产约定：周一覆盖最近 72 小时，周二至周五覆盖最近 24 小时，截止 07:00。'), '### 先交首批
凑够一批推荐选题（一般 4–8 个）就提交，交主编**核对格式与材料**（五项清单）。主编对内容的意见写在通过意见里，随 02 派工单交选题研究员采用；**工作台不按内容意见返工**，只在格式或材料不全被退回时补材料、重新导出。', '### 先交首批
凑够一批推荐选题（一般 4–8 个）就先提交，等主编校准，再把其余判完的以**补件**追加，不重新提交整包。
')
WHERE code = 'intake'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_gates SET name = '主编首批校准'
WHERE name = '主编格式核对'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd

-- +goose StatementBegin
UPDATE workflow_stages SET
  instructions = REPLACE(instructions, '**派工单里的【上游主编意见】是你的起点**：那是主编在 01 通过时写的内容意见（不纳入、同一件事、代表图、译名等）。主编说不纳入的不进候选，说是同一件事的合并成一题；主编在本派工单里另写的「这几条不纳入」同样照办。不采用的逐条写明理由。', '01 的主编首批校准结论是你的起点，方向已被主编停掉的对象不再投入。'),
  acceptance = REPLACE(acceptance, char(10) || '- 已采用派工单里【上游主编意见】的内容意见，或逐条写明不采用的理由。', '')
WHERE code = 'shortlist'
  AND workflow_id IN (SELECT id FROM workflows WHERE wf_key='daily_news' AND version=10 AND status='draft');
-- +goose StatementEnd
