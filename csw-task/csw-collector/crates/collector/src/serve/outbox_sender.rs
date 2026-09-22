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
    let body: serde_json::Value = serde_json::from_str(&e.body_json)
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
    Ok(engine.put_raw(&path, &body).await?.to_string())
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
