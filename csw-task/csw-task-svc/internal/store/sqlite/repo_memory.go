package sqlite

import (
	"context"
	"strings"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

// 选题记忆：准则卡与案例库。
//
// 两张表的分工是硬约束（迁移 0039 的建表注释）：案例只存原话，准则才是归纳，
// 且归纳必须标明是否经她确认。代码里对应的是 `ConfirmedByVan` —— 写入路径
// **从不自动置 1**，只有人在后台确认才变。

// ListSelectionRules 列出准则卡。
//
// `onlyConfirmed` 为真时只给她确认过的——02 / 03 取判断依据走这条，
// 未确认的归纳不能当依据用。展示用的地方传 false，但要把这一列一起显示出来。
func (q *Queries) ListSelectionRules(ctx context.Context, category string, onlyConfirmed bool, limit int) ([]domain.SelectionRule, error) {
	if limit <= 0 || limit > 200 {
		limit = 50
	}
	where := []string{"enabled = 1"}
	args := []any{}
	if c := strings.TrimSpace(category); c != "" {
		where = append(where, "category = ?")
		args = append(args, c)
	}
	if onlyConfirmed {
		where = append(where, "confirmed_by_van = 1")
	}
	args = append(args, limit)
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, rule_key, category, text, COALESCE(derived_from,''), version,
		       confirmed_by_van, COALESCE(confirmed_at,''), enabled, created_at, updated_at
		FROM selection_rules WHERE `+strings.Join(where, " AND ")+`
		ORDER BY confirmed_by_van DESC, category, rule_key LIMIT ?`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.SelectionRule
	for rows.Next() {
		var r domain.SelectionRule
		if err := rows.Scan(&r.ID, &r.RuleKey, &r.Category, &r.Text, &r.DerivedFrom, &r.Version,
			&r.ConfirmedByVan, &r.ConfirmedAt, &r.Enabled, &r.CreatedAt, &r.UpdatedAt); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// ListSelectionCases 列出案例。`decision` 空则全给。
func (q *Queries) ListSelectionCases(ctx context.Context, decision, brand string, limit int) ([]domain.SelectionCase, error) {
	if limit <= 0 || limit > 500 {
		limit = 50
	}
	where := []string{"1=1"}
	args := []any{}
	if d := strings.TrimSpace(decision); d != "" {
		where = append(where, "decision = ?")
		args = append(args, d)
	}
	if b := strings.TrimSpace(brand); b != "" {
		where = append(where, "LOWER(COALESCE(brand,'')) = LOWER(?)")
		args = append(args, b)
	}
	args = append(args, limit)
	rows, err := q.ex.QueryContext(ctx, `
		SELECT id, case_key, run_id, COALESCE(item_key,''), COALESCE(brand,''), title,
		       COALESCE(source_url,''), decision, quote, COALESCE(quote_ref,''),
		       COALESCE(decided_at,''), COALESCE(judged_tier,''), created_at, updated_at
		FROM selection_cases WHERE `+strings.Join(where, " AND ")+`
		ORDER BY COALESCE(decided_at,'') DESC, id DESC LIMIT ?`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []domain.SelectionCase
	for rows.Next() {
		var c domain.SelectionCase
		if err := rows.Scan(&c.ID, &c.CaseKey, &c.RunID, &c.ItemKey, &c.Brand, &c.Title,
			&c.SourceURL, &c.Decision, &c.Quote, &c.QuoteRef, &c.DecidedAt, &c.JudgedTier,
			&c.CreatedAt, &c.UpdatedAt); err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// UpsertSelectionRule 按 rule_key 写入准则卡。
//
// **不碰 confirmed_by_van。** 导入是机器的动作，确认是人的动作——
// 一次重新导入把她确认过的标记冲掉，那条准则就悄悄降级成了猜测；
// 反过来自动置 1 更糟，未经确认的归纳会直接变成判断依据。
func (q *Queries) UpsertSelectionRule(ctx context.Context, r domain.SelectionRule) error {
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO selection_rules (rule_key, category, text, derived_from, version, updated_at)
		VALUES (?,?,?,?,?, strftime('%Y-%m-%dT%H:%M:%SZ','now'))
		ON CONFLICT(rule_key) DO UPDATE SET
		  category=excluded.category, text=excluded.text,
		  derived_from=excluded.derived_from, version=excluded.version,
		  updated_at=excluded.updated_at`,
		r.RuleKey, r.Category, r.Text, r.DerivedFrom, r.Version)
	return err
}

// SetRuleConfirmed 人工确认或撤销确认一条准则。
func (q *Queries) SetRuleConfirmed(ctx context.Context, ruleKey string, confirmed bool) error {
	_, err := q.ex.ExecContext(ctx, `
		UPDATE selection_rules
		SET confirmed_by_van=?,
		    confirmed_at=CASE WHEN ? THEN strftime('%Y-%m-%dT%H:%M:%SZ','now') ELSE NULL END,
		    updated_at=strftime('%Y-%m-%dT%H:%M:%SZ','now')
		WHERE rule_key=?`, confirmed, confirmed, ruleKey)
	return err
}

// UpsertSelectionCase 按 case_key 写入案例。
//
// 原话**原样存**，不做任何清洗——这张表的全部价值就在于它存的是她说过的话。
func (q *Queries) UpsertSelectionCase(ctx context.Context, c domain.SelectionCase) error {
	_, err := q.ex.ExecContext(ctx, `
		INSERT INTO selection_cases
		  (case_key, run_id, item_key, brand, title, source_url, decision, quote,
		   quote_ref, decided_at, judged_tier, updated_at)
		VALUES (?,?,?,?,?,?,?,?,?,?,?, strftime('%Y-%m-%dT%H:%M:%SZ','now'))
		ON CONFLICT(case_key) DO UPDATE SET
		  run_id=excluded.run_id, item_key=excluded.item_key, brand=excluded.brand,
		  title=excluded.title, source_url=excluded.source_url, decision=excluded.decision,
		  quote=excluded.quote, quote_ref=excluded.quote_ref, decided_at=excluded.decided_at,
		  judged_tier=excluded.judged_tier, updated_at=excluded.updated_at`,
		c.CaseKey, c.RunID, c.ItemKey, c.Brand, c.Title, c.SourceURL, c.Decision,
		c.Quote, c.QuoteRef, c.DecidedAt, c.JudgedTier)
	return err
}
