package sqlite

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/azure1489/csw-agent-team/csw-task-svc/internal/domain"
)

func memStore(t *testing.T) *Store {
	t.Helper()
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { db.Close() })
	if err := Migrate(db); err != nil {
		t.Fatalf("migrate: %v", err)
	}
	return New(db)
}

// 导入准则卡**永远不会**把它标成她确认过的，重新导入也不会把已确认的冲掉。
//
// 这条钉的是迁移 0039 建表注释里那句硬约束：未经她确认的归纳只能展示，
// 不能当判断依据。自动置 1 会让我们的猜测直接变成 02 / 03 的依据；
// 重新导入冲掉已确认的，则会让一条她点过头的准则悄悄降级。
func TestRuleConfirmNeverAutomatic(t *testing.T) {
	st := memStore(t)
	ctx := context.Background()
	q := st.Q()

	r := domain.SelectionRule{
		RuleKey: "lower-plain-restock", Category: "lower",
		Text: "普通上新降低优先级", DerivedFrom: "p5-a-1,p5-a-2",
	}
	if err := q.UpsertSelectionRule(ctx, r); err != nil {
		t.Fatalf("upsert: %v", err)
	}
	got, err := q.ListSelectionRules(ctx, "", false, 10)
	if err != nil || len(got) != 1 {
		t.Fatalf("list: %v %d", err, len(got))
	}
	if got[0].ConfirmedByVan {
		t.Fatal("导入不该自动标成她确认过")
	}
	// 只给确认过的那条路，此时应当为空
	if only, _ := q.ListSelectionRules(ctx, "", true, 10); len(only) != 0 {
		t.Fatalf("未确认的不该出现在 confirmed=1 里，拿到 %d 条", len(only))
	}

	// 人工确认
	if err := q.SetRuleConfirmed(ctx, r.RuleKey, true); err != nil {
		t.Fatalf("confirm: %v", err)
	}
	only, _ := q.ListSelectionRules(ctx, "", true, 10)
	if len(only) != 1 || only[0].ConfirmedAt == "" {
		t.Fatalf("确认后该有一条且带时间，拿到 %+v", only)
	}

	// 再导一次：正文更新，确认标记不动
	r.Text = "改过的说法"
	if err := q.UpsertSelectionRule(ctx, r); err != nil {
		t.Fatalf("re-upsert: %v", err)
	}
	after, _ := q.ListSelectionRules(ctx, "", false, 10)
	if after[0].Text != "改过的说法" {
		t.Fatalf("正文该更新，拿到 %q", after[0].Text)
	}
	if !after[0].ConfirmedByVan {
		t.Fatal("重新导入不该把她确认过的冲掉")
	}
}

// 案例按 case_key 幂等，原话一字不动地存。
func TestSelectionCaseUpsert(t *testing.T) {
	st := memStore(t)
	ctx := context.Background()
	q := st.Q()

	quote := "这个不要，普通上新没什么好说的"
	c := domain.SelectionCase{
		CaseKey: "p5-editor-m1", Brand: "Snow Peak", Title: "秋季新色帐篷",
		Decision: "rejected", Quote: quote, DecidedAt: "2026-09-01T10:00:00Z",
	}
	for i := 0; i < 2; i++ {
		if err := q.UpsertSelectionCase(ctx, c); err != nil {
			t.Fatalf("upsert %d: %v", i, err)
		}
	}
	got, err := q.ListSelectionCases(ctx, "", "", 10)
	if err != nil || len(got) != 1 {
		t.Fatalf("导两次该只有一条：%v %d", err, len(got))
	}
	if got[0].Quote != quote {
		t.Fatalf("原话要一字不动，拿到 %q", got[0].Quote)
	}
	// 按决定与品牌筛
	if byD, _ := q.ListSelectionCases(ctx, "rejected", "", 10); len(byD) != 1 {
		t.Fatal("按 decision 筛不到")
	}
	if byB, _ := q.ListSelectionCases(ctx, "", "snow peak", 10); len(byB) != 1 {
		t.Fatal("品牌筛该不分大小写")
	}
	if none, _ := q.ListSelectionCases(ctx, "adopted", "", 10); len(none) != 0 {
		t.Fatal("不该筛出别的决定")
	}
}
