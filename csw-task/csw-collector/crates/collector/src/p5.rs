//! P5 · 历史反馈回收：把 Van 说过的话从 Hermes 的会话库里捞出来。
//!
//! # 为什么分两步，而且这里只做第一步
//!
//! 引擎侧 `selection_cases` 的建表注释写着一条硬约束：
//!
//! > 案例库只存**她说过的原话**与当时的决定，不存我们的归纳。
//! > 归纳放准则卡，且必须标明是否经她确认过。
//!
//! 所以回收天然是两步：
//!
//! | 步 | 做什么 | 出网 | 产物 |
//! |---|---|---|---|
//! | **一（这里）** | 只读扫会话库，按人与时间筛出原话 + 上下文 | **不出网** | `quotes.jsonl` + `coverage.md` |
//! | 二（另一个子命令） | 送模型分 track 与身份，产出准则卡**草稿** | 出网 | 待 Van 校准的卡片 |
//!
//! 第一步一个字都不改写、不归纳、不调模型——它产出的就是原话本身，
//! 而原话正是这条约束要保住的东西。第二步要把 Van 的原话送去第三方，
//! **那是另一次授权**。
//!
//! # 只读，而且是真的只读
//!
//! 会话库是 Hermes 正在用的，九个 agent 随时在写。这里用
//! `?immutable=1` 打开：SQLite 连 WAL 都不碰，不建 `-shm`、不建 `-wal`、
//! 不取任何锁。代价是读不到还在 WAL 里没落盘的最新几条——对回收历史来说无所谓，
//! 而「绝不干扰正在跑的 agent」是硬要求。
//!
//! # open_id 按应用隔离
//!
//! 同一个人在九个 profile 里的 `user_id` 不一样（总方案 §6.4 记的缺口之一）。
//! 所以第一次跑不给 `--who`，它只列出「每个库里有哪些人、各说了多少条」，
//! 由人对着认；认出来之后再用 `--who` 指定，可以给多个（每个库一个 id）。
//!
//! **认不出来就不抽**：宁可产出一张空清单，也不能把别人的话当成她的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

/// 一个人在某个库里的发言量。探查模式列的就是这个。
#[derive(Debug, Clone)]
pub struct Speaker {
    pub profile: String,
    pub user_id: String,
    pub display_name: String,
    /// 私聊里说了几条
    pub direct: usize,
    /// 群里说了几条
    pub group: usize,
    pub first_at: String,
    pub last_at: String,
}

/// 捞出来的一条原话，连着它的上下文。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Quote {
    pub profile: String,
    pub session_id: String,
    /// direct | group
    pub chat_type: String,
    pub message_id: String,
    pub said_at: String,
    /// **原话，一个字都没动。**
    pub text: String,
    /// 她这句话之前的几条（多半是 agent 的提案），按时间正序
    pub context_before: Vec<ContextLine>,
    /// 之后的几条（多半是 agent 的回应）
    pub context_after: Vec<ContextLine>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextLine {
    pub role: String,
    pub at: String,
    /// 上下文按长度截断——它是用来认场景的，不是证据本身
    pub text: String,
}

pub struct Opts {
    /// 扫哪个目录下的 profile（默认 `/root/.hermes/profiles`）
    pub profiles_dir: PathBuf,
    /// 只要这几个人的话。空 = 探查模式，只列人不抽话
    pub who: Vec<String>,
    /// 这个时刻之后的（RFC3339 或 `YYYY-MM-DD`）
    pub since: String,
    /// 一条原话前后各带几条上下文
    pub context: usize,
    /// 产物写到哪
    pub out: PathBuf,
}

/// 上下文每条最多留这么多字。认场景够用，不喧宾夺主。
const CONTEXT_CHARS: usize = 300;

pub fn run(o: &Opts) -> Result<()> {
    let dbs = find_dbs(&o.profiles_dir)?;
    anyhow::ensure!(
        !dbs.is_empty(),
        "{} 下面一个 state.db 都没有",
        o.profiles_dir.display()
    );
    std::fs::create_dir_all(&o.out).with_context(|| format!("建 {}", o.out.display()))?;

    if o.who.is_empty() {
        return probe(&dbs, &o.since, &o.out);
    }
    collect(&dbs, o)
}

/// 探查模式：只回答「每个库里有哪些人、各说了多少条」。
///
/// 一条消息内容都不打印——认人用的是 id、昵称与条数。
fn probe(dbs: &[(String, PathBuf)], since: &str, out: &Path) -> Result<()> {
    let mut all: Vec<Speaker> = Vec::new();
    for (profile, path) in dbs {
        match speakers_in(path, profile, since) {
            Ok(mut v) => all.append(&mut v),
            Err(e) => eprintln!("  {profile}：读不了（{e:#}）"),
        }
    }
    // 说得多的排前面，好认人
    all.sort_by_key(|s| std::cmp::Reverse(s.direct + s.group));

    let mut md = String::from("# P5 探查：会话库里有哪些人\n\n");
    md.push_str(&format!(
        "窗口：{since} 起。**这一步没有读任何消息内容**，只数了条数。\n\n"
    ));
    md.push_str("| profile | user_id | 昵称 | 私聊 | 群 | 最早 | 最晚 |\n");
    md.push_str("|---|---|---|---|---|---|---|\n");
    for s in &all {
        md.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} |\n",
            s.profile,
            s.user_id,
            if s.display_name.is_empty() {
                "—"
            } else {
                &s.display_name
            },
            s.direct,
            s.group,
            s.first_at.chars().take(10).collect::<String>(),
            s.last_at.chars().take(10).collect::<String>(),
        ));
    }
    md.push_str(
        "\n## 接下来\n\n\
         对着这张表认出哪几个 id 是 Van（**同一个人在不同 profile 里 id 不一样**，\
         这是 open_id 按应用隔离造成的），然后：\n\n\
         ```\n\
         csw-collector p5 --who <id1> --who <id2> … --out <目录>\n\
         ```\n\n\
         认不出来就别抽——宁可产出一张空清单，也不能把别人的话当成她的。\n",
    );
    let p = out.join("probe.md");
    std::fs::write(&p, &md).with_context(|| format!("写 {}", p.display()))?;
    println!("{md}");
    println!("也写到了 {}", p.display());
    Ok(())
}

/// 抽取模式：把指定那几个人的原话连同上下文捞出来。
fn collect(dbs: &[(String, PathBuf)], o: &Opts) -> Result<()> {
    let mut quotes: Vec<Quote> = Vec::new();
    let mut per_profile: BTreeMap<String, usize> = BTreeMap::new();
    let mut failed: Vec<String> = Vec::new();

    for (profile, path) in dbs {
        match quotes_in(path, profile, &o.who, &o.since, o.context) {
            Ok(v) => {
                per_profile.insert(profile.clone(), v.len());
                quotes.extend(v);
            }
            Err(e) => {
                failed.push(format!("{profile}：{e:#}"));
                per_profile.insert(profile.clone(), 0);
            }
        }
    }
    quotes.sort_by(|a, b| a.said_at.cmp(&b.said_at));

    let jsonl = o.out.join("quotes.jsonl");
    let mut buf = String::new();
    for q in &quotes {
        buf.push_str(&serde_json::to_string(q)?);
        buf.push('\n');
    }
    std::fs::write(&jsonl, &buf).with_context(|| format!("写 {}", jsonl.display()))?;

    let cov = coverage_md(o, &per_profile, &quotes, &failed);
    let covp = o.out.join("coverage.md");
    std::fs::write(&covp, &cov).with_context(|| format!("写 {}", covp.display()))?;

    println!("捞到 {} 条原话 → {}", quotes.len(), jsonl.display());
    println!("覆盖清单 → {}", covp.display());
    println!("\n{cov}");
    Ok(())
}

/// 覆盖清单。**缺口要写明**，不能让人以为这就是全部。
fn coverage_md(
    o: &Opts,
    per_profile: &BTreeMap<String, usize>,
    quotes: &[Quote],
    failed: &[String],
) -> String {
    let mut s = String::from("# P5 覆盖清单\n\n");
    s.push_str(&format!(
        "窗口：{} 起。认的人：{}。\n\n",
        o.since,
        o.who
            .iter()
            .map(|w| format!("`{w}`"))
            .collect::<Vec<_>>()
            .join("、")
    ));

    s.push_str("| profile | 捞到几条 |\n|---|---|\n");
    for (p, n) in per_profile {
        s.push_str(&format!("| {p} | {n} |\n"));
    }
    s.push_str(&format!("| **合计** | **{}** |\n\n", quotes.len()));

    let (d, g) = quotes.iter().fold((0, 0), |(d, g), q| {
        if q.chat_type == "group" {
            (d, g + 1)
        } else {
            (d + 1, g)
        }
    });
    s.push_str(&format!("私聊 {d} 条、群里 {g} 条。\n\n"));

    if !failed.is_empty() {
        s.push_str("## 读不了的库\n\n");
        for f in failed {
            s.push_str(&format!("- {f}\n"));
        }
        s.push('\n');
    }

    s.push_str(
        "## 已知缺口（总方案 §6.4 记过，这里再说一遍）\n\n\
         1. **各库只存了路由给该 agent 的群消息。** 完整的群记录不在这里，\
         要经飞书接口另取——那要另外的权限。所以上面「群」那一列是**偏少**的，\
         不能当成她在群里说话的全部。\n\
         2. **open_id 按应用隔离。** 同一个人在九个 profile 里 id 不同，\
         认漏一个就少一个库的话。上面「认的人」那一行要对着 `probe.md` 逐个核。\n\
         3. **只读打开（`immutable=1`）读不到还在 WAL 里的最新几条。** \
         对回收历史无所谓，但今天刚说的话可能不在里面。\n\n\
         ## 接下来\n\n\
         `quotes.jsonl` 里是**原话本身，一个字都没动**。下一步（分类归纳、\
         产出准则卡草稿）要把这些话送去模型，**那是另一次授权**——\
         引擎侧 `selection_cases` 的建表注释写着：案例只存原话，归纳放准则卡，\
         且必须标明是否经她确认过。\n",
    );
    s
}

/// `<profiles_dir>/<名字>/state.db`，按名字排序。
fn find_dbs(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => anyhow::bail!("读 {} 失败：{e}", dir.display()),
    };
    for e in rd.flatten() {
        let db = e.path().join("state.db");
        if db.is_file() {
            let name = e.file_name().to_string_lossy().to_string();
            out.push((name, db));
        }
    }
    out.sort();
    Ok(out)
}

/// **只读打开。** 见模块文档：不碰 WAL、不取锁、不干扰正在跑的 agent。
fn open_ro(path: &Path) -> Result<Connection> {
    let uri = format!("file:{}?immutable=1", path.display());
    Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("只读打开 {}", path.display()))
}

/// 把 `YYYY-MM-DD` 补成一天的开头；已经是完整时间戳就原样用。
fn since_ts(since: &str) -> String {
    if since.len() == 10 {
        format!("{since}T00:00:00Z")
    } else {
        since.to_string()
    }
}

/// 把一列时间归一成 ISO 文本（`2026-09-22T05:30:00Z`）的 SQL 表达式。
///
/// # 为什么要它
///
/// Hermes 的会话库把 `messages.timestamp` 存成 **`REAL`（Unix 秒）**。第一版直接拿它
/// 和 `'2026-06-01T00:00:00Z'` 比——**SQLite 里数字永远小于文本**，于是条件对每一行
/// 都是假，九个库一个人都数不出来，而且**没有任何报错**。单测没抓到，是因为夹具建的是
/// `timestamp TEXT`：夹具和真库不是一个类型。
///
/// 归一之后比较与输出都用文本：ISO 串按字典序比就是按时间比，两种存法都认。
fn iso(col: &str) -> String {
    format!(
        "(CASE WHEN typeof({col}) IN ('real','integer') \
               THEN strftime('%Y-%m-%dT%H:%M:%SZ', {col}, 'unixepoch') ELSE {col} END)"
    )
}

fn speakers_in(path: &Path, profile: &str, since: &str) -> Result<Vec<Speaker>> {
    let conn = open_ro(path)?;
    let since = since_ts(since);
    let t = iso("m.timestamp");
    let mut st = conn.prepare(&format!(
        "SELECT s.user_id,
                COALESCE(MAX(s.display_name), ''),
                SUM(CASE WHEN COALESCE(s.chat_type,'') = 'group' THEN 0 ELSE 1 END),
                SUM(CASE WHEN COALESCE(s.chat_type,'') = 'group' THEN 1 ELSE 0 END),
                MIN({t}), MAX({t})
           FROM messages m JOIN sessions s ON s.id = m.session_id
          WHERE m.role = 'user' AND {t} >= ?1
            AND COALESCE(s.user_id,'') <> ''
          GROUP BY s.user_id"
    ))?;
    let rows = st.query_map([&since], |r| {
        Ok(Speaker {
            profile: profile.to_string(),
            user_id: r.get::<_, String>(0)?,
            display_name: r.get::<_, String>(1)?,
            direct: r.get::<_, i64>(2)? as usize,
            group: r.get::<_, i64>(3)? as usize,
            first_at: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            last_at: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        })
    })?;
    // **不用 `.flatten()`**：它会把读取失败的行悄悄丢掉。类型对不上时第一版就是
    // 这么「成功地」返回了一张空表——错要报出来，不能吞。
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn quotes_in(
    path: &Path,
    profile: &str,
    who: &[String],
    since: &str,
    ctx: usize,
) -> Result<Vec<Quote>> {
    let conn = open_ro(path)?;
    let since = since_ts(since);
    // user_id 的数量不定，占位符按需拼——值仍然走参数绑定，不拼进 SQL
    let holes = std::iter::repeat_n("?", who.len())
        .collect::<Vec<_>>()
        .join(",");
    let t = iso("m.timestamp");
    let sql = format!(
        "SELECT m.id, m.session_id, COALESCE(s.chat_type,''), {t}, m.content
           FROM messages m JOIN sessions s ON s.id = m.session_id
          WHERE m.role = 'user' AND {t} >= ?1
            AND s.user_id IN ({holes})
            AND COALESCE(m.content,'') <> ''
          ORDER BY m.timestamp, m.id"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&since];
    for w in who {
        params.push(w);
    }
    let mut st = conn.prepare(&sql)?;
    let rows: Vec<(i64, String, String, String, String)> = st
        .query_map(params.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, String>(4)?,
            ))
        })?
        // 同上：不 flatten，读取出错要报出来
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (id, session_id, chat_type, at, text) in rows {
        let (before, after) = context_of(&conn, &session_id, id, ctx)?;
        let id = id.to_string();
        out.push(Quote {
            profile: profile.to_string(),
            session_id,
            chat_type: if chat_type == "group" {
                "group".into()
            } else {
                "direct".into()
            },
            message_id: id,
            said_at: at,
            // **原话原样**：不 trim、不截断、不清洗
            text,
            context_before: before,
            context_after: after,
        });
    }
    Ok(out)
}

/// 取这句话前后各 `n` 条。上下文是用来认场景的，所以按长度截断。
///
/// **按消息 id 前后取，不按时间。** 同一会话里 id 随时间递增；按时间比就又要面对
/// 「REAL 还是 TEXT」那个坑，而且同一秒里的两条会分不出先后。
fn context_of(
    conn: &Connection,
    session_id: &str,
    msg_id: i64,
    n: usize,
) -> Result<(Vec<ContextLine>, Vec<ContextLine>)> {
    if n == 0 {
        return Ok((vec![], vec![]));
    }
    let t = iso("timestamp");
    let mut before = fetch_ctx(
        conn,
        &format!(
            "SELECT role, {t}, content FROM messages
              WHERE session_id = ?1 AND id < ?2 AND COALESCE(content,'') <> ''
              ORDER BY id DESC LIMIT ?3"
        ),
        session_id,
        msg_id,
        n,
    )?;
    before.reverse(); // 查出来是倒序，还原成时间正序
    let after = fetch_ctx(
        conn,
        &format!(
            "SELECT role, {t}, content FROM messages
              WHERE session_id = ?1 AND id > ?2 AND COALESCE(content,'') <> ''
              ORDER BY id LIMIT ?3"
        ),
        session_id,
        msg_id,
        n,
    )?;
    Ok((before, after))
}

fn fetch_ctx(
    conn: &Connection,
    sql: &str,
    session_id: &str,
    msg_id: i64,
    n: usize,
) -> Result<Vec<ContextLine>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map(rusqlite::params![session_id, msg_id, n as i64], |r| {
        Ok(ContextLine {
            role: r.get::<_, String>(0)?,
            at: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            text: r.get::<_, String>(2)?,
        })
    })?;
    // 不 flatten：读取出错要报出来
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|mut c| {
            if c.text.chars().count() > CONTEXT_CHARS {
                c.text = c.text.chars().take(CONTEXT_CHARS).collect::<String>() + "…（截断）";
            }
            c
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个长得像 Hermes 会话库的临时库。
    /// **照真实 Hermes 会话库建的夹具**：`timestamp REAL`（Unix 秒）。
    ///
    /// 旧夹具用的是 `TEXT`，于是第一版「时间拿 REAL 和文本比」的 bug 测不出来——
    /// 上线扫九个真库，一个人都没数到，而且没有任何报错。两种存法都留着测。
    ///
    /// 2026-06-10T09:00:00Z = 1781082000；2026-05-01T09:00:00Z = 1777626000
    fn fake_db_real(dir: &Path, profile: &str) -> PathBuf {
        let d = dir.join(profile);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("state.db");
        let c = Connection::open(&p).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, user_id TEXT, chat_type TEXT, display_name TEXT, started_at REAL);
             CREATE TABLE messages(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, content TEXT, timestamp REAL);
             INSERT INTO sessions VALUES ('s1','ou_van','p2p','Van',1781082000.0),('s2','ou_other','group','别人',1781168400.0);
             INSERT INTO messages(session_id,role,content,timestamp) VALUES
               ('s1','assistant','今天这条推荐给你看',1781082000.0),
               ('s1','user','这个不要，不是新品',1781082060.5),
               ('s1','assistant','好的，我换一条',1781082120.0),
               ('s1','user','五月那条也别发了',1777626000.0),
               ('s2','user','别人说的话',1781168400.0);",
        )
        .unwrap();
        p
    }

    #[test]
    fn 时间存成unix秒的真实会话库也数得出人() {
        let dir = tempdir::TempDir::new("p5real").unwrap();
        fake_db_real(dir.path(), "chief");
        let dbs = vec![("chief".to_string(), dir.path().join("chief/state.db"))];
        let sp = speakers_in(&dbs[0].1, "chief", "2026-06-01").unwrap();
        // 两个人都在窗口里说过话；五月那句被窗口卡掉，但 Van 六月还有一句
        assert_eq!(sp.len(), 2, "REAL 时间也要数得出人：{sp:?}");
        let van = sp.iter().find(|s| s.user_id == "ou_van").unwrap();
        assert_eq!(van.direct, 1, "五月那句不在窗口里");
        // 输出的时间要是可读的 ISO，不是一串浮点数
        assert_eq!(van.first_at, "2026-06-10T09:01:00Z");
    }

    #[test]
    fn 时间存成unix秒时抽话与上下文也对() {
        let dir = tempdir::TempDir::new("p5real2").unwrap();
        fake_db_real(dir.path(), "chief");
        let q = quotes_in(
            &dir.path().join("chief/state.db"),
            "chief",
            &["ou_van".to_string()],
            "2026-06-01",
            1,
        )
        .unwrap();
        assert_eq!(q.len(), 1, "只有六月那一句：{q:?}");
        assert_eq!(q[0].text, "这个不要，不是新品");
        assert_eq!(q[0].said_at, "2026-06-10T09:01:00Z");
        // 前后各一条，按消息 id 取
        assert_eq!(q[0].context_before.len(), 1);
        assert_eq!(q[0].context_before[0].text, "今天这条推荐给你看");
        assert_eq!(q[0].context_after[0].text, "好的，我换一条");
    }

    fn fake_db(dir: &Path, profile: &str) -> PathBuf {
        let d = dir.join(profile);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("state.db");
        let c = Connection::open(&p).unwrap();
        c.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, user_id TEXT, chat_type TEXT, display_name TEXT);
             CREATE TABLE messages(id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, content TEXT, timestamp TEXT);
             INSERT INTO sessions VALUES ('s1','ou_van','p2p','Van'),('s2','ou_other','group','别人');
             INSERT INTO messages(session_id,role,content,timestamp) VALUES
               ('s1','assistant','今天这条推荐给你看','2026-06-10T09:00:00Z'),
               ('s1','user','这个不要，不是新品','2026-06-10T09:01:00Z'),
               ('s1','assistant','好的，我换一条','2026-06-10T09:02:00Z'),
               ('s1','user','五月那条也别发了','2026-05-01T09:00:00Z'),
               ('s2','user','别人说的话','2026-06-11T09:00:00Z');",
        )
        .unwrap();
        p
    }

    #[test]
    fn 只抽认出来的那个人且按时间卡住() {
        let dir = tempdir::TempDir::new("p5").unwrap();
        fake_db(dir.path(), "chief");
        let out = dir.path().join("out");
        run(&Opts {
            profiles_dir: dir.path().to_path_buf(),
            who: vec!["ou_van".into()],
            since: "2026-06-01".into(),
            context: 1,
            out: out.clone(),
        })
        .unwrap();

        let lines = std::fs::read_to_string(out.join("quotes.jsonl")).unwrap();
        let qs: Vec<serde_json::Value> = lines
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // 只有 6/10 那条：5/1 在窗口外，别人那条不是她说的
        assert_eq!(qs.len(), 1, "{lines}");
        assert_eq!(qs[0]["text"], "这个不要，不是新品");
        // **原话一个字都没动**
        assert_eq!(qs[0]["chat_type"], "direct");
        // 上下文前后各一条，前面那条是 agent 的提案
        assert_eq!(qs[0]["context_before"][0]["text"], "今天这条推荐给你看");
        assert_eq!(qs[0]["context_after"][0]["text"], "好的，我换一条");
    }

    /// 不给 `--who` 就只列人，**一条消息内容都不读出来**。
    #[test]
    fn 探查模式只列人不抽话() {
        let dir = tempdir::TempDir::new("p5probe").unwrap();
        fake_db(dir.path(), "chief");
        let out = dir.path().join("out");
        run(&Opts {
            profiles_dir: dir.path().to_path_buf(),
            who: vec![],
            since: "2026-06-01".into(),
            context: 2,
            out: out.clone(),
        })
        .unwrap();

        let md = std::fs::read_to_string(out.join("probe.md")).unwrap();
        assert!(md.contains("ou_van") && md.contains("ou_other"), "{md}");
        // 探查产物里不该出现任何一句原话
        assert!(!md.contains("不是新品"), "探查模式泄露了消息内容：{md}");
        // 也不该产出 quotes.jsonl
        assert!(!out.join("quotes.jsonl").exists());
    }

    #[test]
    fn 认不出人就产出空清单而不是抽错人() {
        let dir = tempdir::TempDir::new("p5none").unwrap();
        fake_db(dir.path(), "chief");
        let out = dir.path().join("out");
        run(&Opts {
            profiles_dir: dir.path().to_path_buf(),
            who: vec!["ou_不存在".into()],
            since: "2026-06-01".into(),
            context: 1,
            out: out.clone(),
        })
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("quotes.jsonl")).unwrap(),
            ""
        );
        let cov = std::fs::read_to_string(out.join("coverage.md")).unwrap();
        assert!(cov.contains("**0**"), "{cov}");
        // 缺口必须写明，不能让人以为这就是全部
        assert!(cov.contains("open_id 按应用隔离"));
    }
}
