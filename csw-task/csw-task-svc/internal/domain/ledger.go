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
	// PublishEvidence 凭什么说它发布了：公开页地址、抓取时间、抓到的标题。
	// state='published' 只是同步器的一个判断，没有这个就回头核不了。
	PublishEvidence string
	RunID           *int64
	ID              int64
	// IsReference 这条是不是「范例」。范例是编辑部认可的写法样板，
	// 与「正式已发布」是两类对照材料，判断时不能混成一类。
	IsReference bool
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

// SelectionRule 选题准则卡。**归纳，不是原话。**
//
// 与 SelectionCase 的分工是硬约束（迁移 0039 的建表注释）：案例库只存她说过的
// 原话与当时的决定，归纳放这里，且必须标明是否经她确认过。没确认过的只能展示，
// 不能当判断依据——这条区分一旦模糊，我们的猜测就会以她的名义进入判断。
type SelectionRule struct {
	RuleKey        string
	Category       string // frame | prefer | lower | dedup | other
	Text           string
	DerivedFrom    string // 逗号分隔的 case_key，回头能核
	Version        string
	ConfirmedByVan bool
	ConfirmedAt    string
	Enabled        bool
	CreatedAt      string
	UpdatedAt      string
	ID             int64
}

// SelectionCase 选题案例。**只存她说过的原话与当时的决定，不存归纳。**
type SelectionCase struct {
	CaseKey    string
	RunID      *int64
	ItemKey    string
	Brand      string
	Title      string
	SourceURL  string
	Decision   string // adopted | rejected | deferred | pending_check
	Quote      string // 她的原话。代录也要原样附，不许转述
	QuoteRef   string
	DecidedAt  string
	JudgedTier string // 当时系统给的结论档，用来算「我们判得准不准」
	CreatedAt  string
	UpdatedAt  string
	ID         int64
}
