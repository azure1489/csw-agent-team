//! 把 outbox 里排着的发出去。
//!
//! **一次只发一条，发完确认再发下一条。** 并发发没有意义：条目之间有依赖，
//! 而且引擎那边的幂等是按键算的，抢着发只会把顺序搞乱。
//!
//! 遇到冲突就停。`idempotency_in_progress_or_uncertain` 与 409 都意味着
//! 「引擎那边可能已经有了」——这时候换键重发就是重复提交。

use anyhow::Result;
use rusqlite::Connection;

use csw_collector_core::outbox::{self, Entry};
use csw_collector_engineapi::client::{EngineClient, EngineError};

/// 排空一次。返回（发成功几条，遇到几条冲突）。
pub async fn drain(conn: &Connection, engine: &EngineClient) -> Result<(usize, usize)> {
    let (mut sent, mut conflicts) = (0, 0);
    while let Some(e) = outbox::next_ready(conn)? {
        outbox::begin_send(conn, e.seq, &e.body_sha)?;
        match send_one(engine, &e).await {
            Ok(resp) => {
                outbox::confirm(conn, e.seq, &resp)?;
                sent += 1;
            }
            Err(err) if is_conflict(&err) => {
                outbox::conflict(conn, e.seq, &format!("{err}"))?;
                conflicts += 1;
                // 停下：后面的都依赖这一条，硬发只会把错叠加
                break;
            }
            Err(err) => {
                let next = outbox::fail(conn, e.seq, &format!("{err}"))?;
                tracing::warn!(第几条 = e.seq, 类型 = %e.kind, 之后 = ?next, 原因 = %err, "这一条没发出去");
                break;
            }
        }
    }
    Ok((sent, conflicts))
}

/// 发一条。**原样发库里那份字节**——不反序列化成结构体再序列化回去，
/// 那样字段顺序与 `skip_serializing_if` 都可能让请求变样。
async fn send_one(engine: &EngineClient, e: &Entry) -> Result<String, EngineError> {
    // 只核一下它还是合法 JSON，**发出去的是原字符串**——
    // 解析成 Value 再发会把对象的键重排（Value 用的是 BTreeMap），
    // 发出去的字节就和 body_sha 对不上了
    serde_json::from_str::<serde_json::Value>(&e.body_json)
        .map_err(|err| EngineError::Transport(format!("第 {} 条的请求体坏了：{err}", e.seq)))?;
    let run_id = run_id_of(e, 0);
    let path = match e.kind.as_str() {
        "items" => format!("/runs/{run_id}/items"),
        "sweeps" => format!("/runs/{run_id}/sweeps"),
        "judgements" => format!("/runs/{run_id}/intake-judgements"),
        other => {
            return Err(EngineError::Transport(format!(
                "outbox 里有一条不认识的类型：{other}"
            )));
        }
    };
    Ok(engine.put_raw(&path, &e.body_json).await?.to_string())
}

/// 幂等键里带着 run id（`r48-items-…`），从它取回来。
///
/// 存在 body 里更直接，但 body 是要原样发给引擎的，塞一个引擎不认的字段
/// 会让请求体和引擎期望的形状对不上。
fn run_id_of(e: &Entry, fallback: i64) -> i64 {
    e.idem_key
        .strip_prefix('r')
        .and_then(|s| s.split('-').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback)
}

/// 「引擎那边可能已经有了」的两种形态。这时候换键重发就是重复提交。
fn is_conflict(e: &EngineError) -> bool {
    match e {
        EngineError::IdempotencyUncertain { .. } => true,
        EngineError::Api { status, code, .. } => *status == 409 || code.contains("idempotency"),
        EngineError::Transport(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use csw_collector_core::outbox::NewEntry;
    use csw_collector_core::types::{OutboxKind, RoundKind, RoundTrigger};

    fn entry(kind: OutboxKind, idem: &str) -> Entry {
        Entry {
            seq: 1,
            round_id: 1,
            kind: kind_str(kind).into(),
            idem_key: idem.into(),
            body_path: String::new(),
            body_json: "{}".into(),
            body_sha: "s".into(),
            depends_on: None,
            status: "pending".into(),
            attempts: 0,
            last_error: String::new(),
        }
    }

    fn kind_str(k: OutboxKind) -> &'static str {
        match k {
            OutboxKind::Items => "items",
            OutboxKind::Sweeps => "sweeps",
            OutboxKind::Judgements => "judgements",
            _ => "other",
        }
    }

    #[test]
    fn run_id从幂等键里取() {
        assert_eq!(run_id_of(&entry(OutboxKind::Items, "r48-items-abc"), 0), 48);
        assert_eq!(run_id_of(&entry(OutboxKind::Items, "乱写的"), 7), 7);
    }

    #[test]
    fn 认得出冲突() {
        assert!(is_conflict(&EngineError::IdempotencyUncertain {
            idem_key: "k".into()
        }));
        assert!(is_conflict(&EngineError::Api {
            status: 409,
            code: "conflict".into(),
            message: String::new()
        }));
        assert!(is_conflict(&EngineError::Api {
            status: 400,
            code: "idempotency_in_progress_or_uncertain".into(),
            message: String::new()
        }));
        // 网络抖动不是冲突：那个该重试
        assert!(!is_conflict(&EngineError::Transport("连不上".into())));
        assert!(!is_conflict(&EngineError::Api {
            status: 500,
            code: "internal".into(),
            message: String::new()
        }));
    }

    /// 一个只会说「收到了」的引擎。
    async fn fake_engine() -> (wiremock::MockServer, EngineClient) {
        let srv = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("PUT"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
            )
            .mount(&srv)
            .await;
        let e = EngineClient::new(
            &format!("{}/api/v1", srv.uri()),
            "t",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        (srv, e)
    }

    fn one_round(c: &Connection) -> i64 {
        csw_collector_core::rounds::open_round(
            c,
            &csw_collector_core::rounds::NewRound {
                kind: RoundKind::Task,
                trigger: RoundTrigger::Dispatch,
                run_id: Some(48),
                task_id: Some(1),
                stage_code: Some("intake".into()),
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap()
        .0
        .id
    }

    #[tokio::test]
    async fn 崩在登记之后重启会把欠的发完() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let round = one_round(&c);
        let body = r#"{"items":[{"item_key":"aw-1","title":"换了背板"}]}"#;
        outbox::enqueue(
            &c,
            &NewEntry {
                round_id: round,
                kind: OutboxKind::Items,
                idem_key: "r48-items-abc".into(),
                body_path: String::new(),
                body_json: body.into(),
                body_sha: blake3::hash(body.as_bytes()).to_hex().to_string(),
                depends_on: None,
            },
        )
        .unwrap();
        // 进程在这里被砍掉：登记排进去了，一个字节都还没发出去

        let (srv, engine) = fake_engine().await;
        let (sent, conflicts) = drain(&c, &engine).await.unwrap();
        assert_eq!((sent, conflicts), (1, 0), "重启后第一件事就是把欠的发完");
        let got = srv.received_requests().await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url.path(), "/api/v1/runs/48/items");
        assert!(outbox::outstanding(&c, round).unwrap().is_empty());
    }

    #[tokio::test]
    async fn 崩在正发着的时候重发的是同样的字节() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let round = one_round(&c);
        // 字段顺序刻意写得不「规范」：重发要原样发这一份，不是反序列化再拼回去
        let body = r#"{"items":[{"title":"换了背板","item_key":"aw-1","brand":"and wander"}]}"#;
        let sha = blake3::hash(body.as_bytes()).to_hex().to_string();
        let e = outbox::enqueue(
            &c,
            &NewEntry {
                round_id: round,
                kind: OutboxKind::Items,
                idem_key: "r48-items-abc".into(),
                body_path: String::new(),
                body_json: body.into(),
                body_sha: sha.clone(),
                depends_on: None,
            },
        )
        .unwrap();
        // 发到一半崩了：状态停在 sending，attempts 已经加过
        outbox::begin_send(&c, e.seq, &sha).unwrap();
        let mid = outbox::get(&c, e.seq).unwrap().unwrap();
        assert_eq!((mid.status.as_str(), mid.attempts), ("sending", 1));

        let (srv, engine) = fake_engine().await;
        assert_eq!(
            drain(&c, &engine).await.unwrap().0,
            1,
            "sending 的也要捡回来重发"
        );

        let got = srv.received_requests().await.unwrap();
        let sent_body = String::from_utf8(got[0].body.clone()).unwrap();
        // **这一条是重点**：引擎那边按内容算幂等，字节变了就会多出一份台账
        assert_eq!(
            sent_body, body,
            "重发的字节和库里存的不一样，引擎的幂等就挡不住重复提交了"
        );
    }

    #[tokio::test]
    async fn 发不出去就停在那一条不往下发() {
        let c = csw_collector_core::store::open_in_memory().unwrap();
        let (r, _) = csw_collector_core::rounds::open_round(
            &c,
            &csw_collector_core::rounds::NewRound {
                kind: RoundKind::Manual,
                trigger: RoundTrigger::Manual,
                run_id: None,
                task_id: None,
                stage_code: None,
                target_version: 1,
                parent_round_id: None,
                window_start: "A".into(),
                window_end: "B".into(),
                plan_version: 1,
                rubric_version: "v1".into(),
                kb_snapshot: "s".into(),
                instructions_hash: "h".into(),
            },
        )
        .unwrap();
        for (kind, idem) in [
            (OutboxKind::Items, "r48-items-a"),
            (OutboxKind::Sweeps, "r48-sweeps-a"),
        ] {
            let prev = if kind == OutboxKind::Sweeps {
                Some(1)
            } else {
                None
            };
            outbox::enqueue(
                &c,
                &NewEntry {
                    round_id: r.id,
                    kind,
                    idem_key: idem.into(),
                    body_path: String::new(),
                    body_json: r#"{"items":[]}"#.into(),
                    body_sha: blake3::hash(br#"{"items":[]}"#).to_hex().to_string(),
                    depends_on: prev,
                },
            )
            .unwrap();
        }
        // 指向一个连不上的地址：第一条就发不出去
        let e = EngineClient::new(
            "http://127.0.0.1:1",
            "t",
            std::time::Duration::from_millis(50),
        )
        .unwrap();
        let (sent, conflicts) = drain(&c, &e).await.unwrap();
        assert_eq!((sent, conflicts), (0, 0));
        // 第二条依赖第一条，不该被发
        let all = outbox::outstanding(&c, r.id).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].attempts, 1);
        assert_eq!(all[1].attempts, 0, "依赖没确认，后一条连试都不该试");
    }
}
