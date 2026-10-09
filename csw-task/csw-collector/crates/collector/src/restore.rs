//! 按某一版交付包恢复本地轮次里几条判断。
//!
//! 工作台返工时误改了判断（10-09 r67 v4：一句否定的「不按低价值停止」被当成停止，
//! 同账号四条被移出；点名的那条被交模型重判、六维全变），主编要求恢复到 v3 的原判断。
//! 交付包的 `trace/judgements.jsonl` 是当时逐条判断的全文，就是主编复核用的基准，
//! 从它恢复，不从某个时间点的库备份猜。
//!
//! 只写本地一轮，**不写引擎、不提交**：恢复之后由主编重开派工，工作台返工时
//! 按恢复后的判断重新登记、导出。不给 `--apply` 只打印要改什么。

use std::io::Read;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use csw_collector_core::types::Judgement;
use csw_collector_core::{Config, ledger, rounds};

pub struct Opts {
    pub round: i64,
    /// 作为基准的交付包（本地 zip）
    pub from: PathBuf,
    /// 恢复哪几条（条目键）
    pub keys: Vec<String>,
    /// 为什么恢复：写进这几条的留痕
    pub reason: String,
    pub apply: bool,
}

/// 恢复时去掉的留痕：误改那一步留下的
const DROP_FLAGS: [&str; 2] = ["【返工】主编退回意见要求移出", "【返工移出】"];

pub fn run(cfg: &Config, o: Opts) -> Result<()> {
    if o.keys.is_empty() {
        bail!("要给 --keys：恢复哪几条");
    }
    if o.reason.trim().is_empty() {
        bail!("要给 --reason：为什么恢复，会写进留痕");
    }
    let conn = csw_collector_core::store::open(&cfg.db_path())
        .with_context(|| format!("打开本地库 {}", cfg.db_path().display()))?;
    let r = rounds::get(&conn, o.round)?.ok_or_else(|| anyhow::anyhow!("没有第 {} 轮", o.round))?;
    let base = judgements_in_zip(&o.from)?;
    let name = o
        .from
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for key in &o.keys {
        let Some(want) = base.iter().find(|j| &j.candidate_key == key) else {
            bail!("{name} 里没有 {key} 的判断");
        };
        let (now_tier, flags_json, model, rubric): (String, String, String, String) = conn
            .query_row(
                "SELECT tier, check_flags_json, model, rubric_version FROM judgements
                 WHERE round_id = ?1 AND candidate_key = ?2",
                rusqlite::params![r.id, key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .with_context(|| format!("第 {} 轮里没有 {key}", r.id))?;
        let mut flags: Vec<String> = serde_json::from_str::<Vec<String>>(&flags_json)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| !DROP_FLAGS.iter().any(|p| f.starts_with(p)))
            .collect();
        flags.push(format!(
            "【维护恢复】{}（按 {name} 的 trace/judgements.jsonl 恢复档位与判断全文，{}）",
            o.reason.trim(),
            jiff::Timestamp::now()
        ));
        println!(
            "{key}：{now_tier} → {}{}",
            serde_json::to_value(want.tier)?.as_str().unwrap_or("?"),
            if o.apply { "" } else { "（干跑，未写）" }
        );
        if o.apply {
            ledger::put_judgement(&conn, r.id, want, &flags, &model, &rubric)?;
        }
    }
    if !o.apply {
        println!("加 --apply 才写入第 {} 轮", r.id);
    }
    Ok(())
}

/// 交付包里的逐条判断全文。
fn judgements_in_zip(path: &PathBuf) -> Result<Vec<Judgement>> {
    let f = std::fs::File::open(path).with_context(|| format!("打开 {}", path.display()))?;
    let mut z = zip::ZipArchive::new(f).with_context(|| format!("读 zip {}", path.display()))?;
    let idx = (0..z.len())
        .find(|i| {
            z.by_index(*i)
                .is_ok_and(|e| e.name().ends_with("/trace/judgements.jsonl"))
        })
        .ok_or_else(|| anyhow::anyhow!("{} 里没有 trace/judgements.jsonl", path.display()))?;
    let mut s = String::new();
    z.by_index(idx)?.read_to_string(&mut s)?;
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Judgement>(l).context("判断行读不回来"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::types::Tier;

    #[test]
    fn 从交付包恢复判断并换掉误改留痕() {
        let dir = tempdir::TempDir::new("restore").unwrap();
        // 交付包：v3 时这条是推荐
        let base = Judgement::fixture("lesyndrome-29913d", Tier::Recommend);
        let zip_path = dir.path().join("v3.zip");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            w.start_file(
                "pkg/trace/judgements.jsonl",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            std::io::Write::write_all(&mut w, serde_json::to_string(&base).unwrap().as_bytes())
                .unwrap();
            w.finish().unwrap();
        }
        let cfg = Config {
            data_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        let conn = csw_collector_core::store::open(&cfg.db_path()).unwrap();
        let (r, _) = rounds::open_round(
            &conn,
            &rounds::NewRound {
                kind: csw_collector_core::types::RoundKind::Task,
                trigger: csw_collector_core::types::RoundTrigger::Dispatch,
                run_id: Some(67),
                task_id: Some(763),
                stage_code: Some("intake".into()),
                target_version: 4,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v".into(),
                kb_snapshot: "k".into(),
                instructions_hash: String::new(),
            },
        )
        .unwrap();
        // 现在：被误移出成不推荐
        let wrong = Judgement::fixture("lesyndrome-29913d", Tier::NotRecommend);
        let flags = vec![
            "【口径】查重改否".to_string(),
            "【返工】主编退回意见要求移出本期主备选（不是品牌永久排除）".to_string(),
        ];
        ledger::put_judgement(&conn, r.id, &wrong, &flags, "m", "v").unwrap();
        drop(conn);

        let opts = |apply| Opts {
            round: r.id,
            from: zip_path.clone(),
            keys: vec!["lesyndrome-29913d".into()],
            reason: "v4 误移出，按主编要求恢复 v3".into(),
            apply,
        };
        run(&cfg, opts(false)).unwrap();
        let conn = csw_collector_core::store::open(&cfg.db_path()).unwrap();
        let tier = |c: &rusqlite::Connection| -> String {
            c.query_row(
                "SELECT tier FROM judgements WHERE candidate_key='lesyndrome-29913d'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(tier(&conn), "not_recommend", "干跑不写");
        drop(conn);

        run(&cfg, opts(true)).unwrap();
        let conn = csw_collector_core::store::open(&cfg.db_path()).unwrap();
        assert_eq!(tier(&conn), "recommend");
        let flags: Vec<String> = serde_json::from_str(
            &conn
                .query_row(
                    "SELECT check_flags_json FROM judgements WHERE candidate_key='lesyndrome-29913d'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap();
        assert!(flags.contains(&"【口径】查重改否".to_string()), "{flags:?}");
        assert!(
            !flags.iter().any(|f| f.starts_with("【返工】")),
            "{flags:?}"
        );
        assert!(
            flags.iter().any(|f| f.starts_with("【维护恢复】v4 误移出")),
            "{flags:?}"
        );
    }
}
