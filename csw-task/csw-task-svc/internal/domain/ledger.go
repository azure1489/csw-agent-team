package domain

// LedgerPost 一篇真实发布（或草稿 / 演练）记录。PublishedAt 统一存 UTC「2006-01-02T15:04:05Z」。
type LedgerPost struct {
	Platform    string // wechat | xhs
	Account     string
	PostID      string
	URL         string
	PublishedAt string
	Title       string
	BodyText    string
	TemplateVer string
	Source      string // agent | manual | sync | import
	State       string // drill | draft | published
	RawJSON     string
	FirstSeenAt string
	UpdatedAt   string
	RunID       *int64
	ID          int64
}

// LedgerPostItem 合集里的一条资讯。
type LedgerPostItem struct {
	ItemKey   string
	Brand     string
	Product   string
	Angle     string
	Title     string
	SourceURL string
	SplitBy   string // auto | manual
	CheckedBy string
	CheckedAt string
	ID        int64
	PostRef   int64
	Seq       int
}

// LedgerSyncRun 一次同步 / 导入 / 探针批次（覆盖范围与缺口）。
type LedgerSyncRun struct {
	Platform   string
	Kind       string // probe | backfill | sync | snapshot | import
	StartedAt  string
	FinishedAt string
	WindowFrom string
	WindowTo   string
	GapNote    string
	Error      string
	ID         int64
	Fetched    int
	Inserted   int
	Updated    int
	OK         bool
}

// FeedbackRecord 一条编辑反馈（原话 + 适用范围）。
type FeedbackRecord struct {
	Quote         string
	SaidAt        string
	SourceRef     string
	ObjectRef     string
	ObjectVersion string
	ImageRefsJSON string
	Kind          string // long_term | case | temporary | recent
	Stance        string // explicit | inferred
	Status        string // active | superseded | expired
	ExpiresAt     string
	CuratedBy     string
	CreatedAt     string
	Tags          []string
	SupersedesID  *int64
	ReviewID      *int64
	ID            int64
}

// MetricSnapshot 单篇指标快照：只存平台原值与异常标记。
type MetricSnapshot struct {
	Platform        string
	AgeBucket       string // 2h | 24h | 72h | 7d | 30d | adhoc
	CollectedAt     string
	WindowFrom      string
	WindowTo        string
	RawJSON         string
	FlagsJSON       string
	DefinitionsJSON string // 指标定义与口径来源（平台字段说明 / 导出文件），原样保存
	ID              int64
	PostRef         int64
}

// AccountSnapshot 账号区间趋势快照。
type AccountSnapshot struct {
	Platform        string
	Account         string
	WindowFrom      string
	WindowTo        string
	CollectedAt     string
	RawJSON         string
	FlagsJSON       string
	DefinitionsJSON string // 指标定义与口径来源，原样保存
	ID              int64
}
