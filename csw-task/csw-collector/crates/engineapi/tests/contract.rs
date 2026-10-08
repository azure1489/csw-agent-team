//! 引擎客户端的契约测试。
//!
//! 两层：
//! - **wiremock**：造出真引擎很难造的情形——429 连发、幂等结果未知、错误体缺字段。
//! - **真引擎二进制**（`contract_live.rs`）：证明字段名与状态枚举真的对得上。
//!   mock 只能证明「我按我以为的样子解析」，对不上是发现不了的。

use std::time::Duration;

use csw_collector_engineapi::{EngineClient, EngineError, submit_idem_key};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(uri: &str) -> EngineClient {
    EngineClient::new(uri, "csw_live_test", Duration::from_secs(5)).unwrap()
}

#[tokio::test]
async fn 接单带bearer并解析任务状态() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/me/tasks"))
        .and(header("authorization", "Bearer csw_live_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tasks": [
                {"task": {"id": 7, "run_id": 3, "stage_code": "intake", "status": "dispatched",
                          "item_key": "", "cur_version": 1}, "run_subject": "2026-09-22"},
                {"task": {"id": 8, "run_id": 3, "stage_code": "material", "status": "returned",
                          "item_key": "hxo-3fa91c", "cur_version": 2}, "run_subject": "2026-09-22"}
            ],
            "skill_min_version": "3.3.0"
        })))
        .mount(&srv)
        .await;

    let got = client(&srv.uri()).my_tasks().await.unwrap();
    assert_eq!(got.tasks.len(), 2);
    assert!(got.tasks[0].task.status.is_actionable());
    assert!(
        got.tasks[1].task.status.is_actionable(),
        "returned 也要干活"
    );
    assert_eq!(got.tasks[1].task.item_key, "hxo-3fa91c");
}

#[tokio::test]
async fn 没见过的状态不会让整次接单失败() {
    // 引擎以后加了新状态时，收集员不该因为解析不了就整轮停摆
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/me/tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "tasks": [{"task": {"id": 1, "run_id": 1, "stage_code": "intake", "status": "某个新状态"}}]
        })))
        .mount(&srv)
        .await;
    let got = client(&srv.uri()).my_tasks().await.unwrap();
    assert!(!got.tasks[0].task.status.is_actionable());
    assert!(!got.tasks[0].task.status.is_terminal());
}

#[tokio::test]
async fn 遇到429会退避重试() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/me/tasks"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(serde_json::json!({"code":"rate_limited"})),
        )
        .up_to_n_times(1)
        .mount(&srv)
        .await;
    Mock::given(method("GET"))
        .and(path("/me/tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"tasks": []})))
        .mount(&srv)
        .await;
    let got = client(&srv.uri()).my_tasks().await.unwrap();
    assert!(got.tasks.is_empty());
}

#[tokio::test]
async fn 四百零九不重试直接报上去() {
    // 业务冲突重试多少次都是同一个结果，只会白白拖住这一轮
    let srv = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/runs/3/items"))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            serde_json::json!({"code":"run_not_active","message":"仅进行中的 run 可登记"}),
        ))
        .expect(1)
        .mount(&srv)
        .await;
    let err = client(&srv.uri()).put_items(3, &[]).await.unwrap_err();
    assert_eq!(err.code(), "run_not_active");
}

#[tokio::test]
async fn 幂等结果未知要抛成专门的错而不是重试() {
    // 这是最危险的一种：换键再发就会重复提交同一份交付物
    let srv = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks/7/fail"))
        .respond_with(ResponseTemplate::new(409).set_body_json(
            serde_json::json!({"code":"idempotency_in_progress_or_uncertain","message":"结果未知"}),
        ))
        .expect(1)
        .mount(&srv)
        .await;
    let err = client(&srv.uri())
        .fail(7, "取数失败", "fail-7-abc")
        .await
        .unwrap_err();
    match err {
        EngineError::IdempotencyUncertain { idem_key } => assert_eq!(idem_key, "fail-7-abc"),
        other => panic!("应当是幂等未知错，实为 {other:?}"),
    }
}

#[tokio::test]
async fn 心跳不带幂等键() {
    // ack 本来就该能重复调（引擎没有独立心跳接口），带了键第二次就会被幂等层挡回来
    let srv = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tasks/7/ack"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"task": {}})))
        .expect(3)
        .mount(&srv)
        .await;
    let c = client(&srv.uri());
    for _ in 0..3 {
        c.ack(7).await.unwrap();
    }
    // 请求里不该出现 Idempotency-Key
    for req in srv.received_requests().await.unwrap() {
        assert!(
            req.headers.get("idempotency-key").is_none(),
            "心跳带了幂等键"
        );
    }
}

#[tokio::test]
async fn 判断台账自动分批到两百() {
    let srv = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/runs/3/intake-judgements"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"judgements": []})),
        )
        .expect(3) // 450 条 → 200 + 200 + 50
        .mount(&srv)
        .await;
    let js: Vec<_> = (0..450).map(|i| judgement(&format!("k-{i:06}"))).collect();
    let n = client(&srv.uri()).put_judgements(3, &js).await.unwrap();
    assert_eq!(n, 450);
}

#[tokio::test]
async fn 幂等键从内容派生所以重试必然同键() {
    let a = submit_idem_key(7, "abcdef0123456789aaaa");
    let b = submit_idem_key(7, "abcdef0123456789aaaa");
    let c = submit_idem_key(7, "ffffff0123456789aaaa");
    assert_eq!(a, b, "同一份内容必须算出同一个键");
    assert_ne!(a, c, "内容变了必须是新键");
    assert_eq!(a, "submit-7-abcdef0123456789");
}

#[tokio::test]
async fn 条目清单只认批准可写的那几条() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/runs/48/items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {"item_key": "and-wander-a1b2c3", "title": "40L 背包", "status": "approved_write",
                 "source_url": "https://www.instagram.com/p/ABC123/"},
                {"item_key": "yamatomichi-d4e5f6", "title": "THREE", "status": "shortlisted",
                 "rank": "alt"},
                {"item_key": "x-999999", "title": "被否的", "status": "rejected"}
            ],
            "target_count": 8, "written": 1, "gap": 7
        })))
        .mount(&srv)
        .await;

    let got = client(&srv.uri()).run_items(48).await.unwrap();
    assert_eq!(got.items.len(), 3);
    assert_eq!(got.target_count, 8);
    // 11 选图包只给「Van 批准可写」的配图。把「研究员建议」当成批准，
    // 会多做一批没人要的图
    let approved: Vec<&str> = got
        .items
        .iter()
        .filter(|i| i.approved())
        .map(|i| i.item_key.as_str())
        .collect();
    assert_eq!(approved, ["and-wander-a1b2c3"]);
    assert_eq!(
        got.items[0].source_url,
        "https://www.instagram.com/p/ABC123/"
    );
}

fn judgement(key: &str) -> csw_collector_engineapi::JudgementInput {
    csw_collector_engineapi::JudgementInput {
        candidate_key: key.into(),
        item_key: String::new(),
        platform: "instagram".into(),
        post_ref: String::new(),
        source_url: String::new(),
        tier: "not_recommend".into(),
        dims: serde_json::json!({}),
        three_sentences: serde_json::json!({}),
        comparison: serde_json::json!({}),
        heat_note: String::new(),
        gaps: serde_json::json!([]),
        hits: serde_json::json!({}),
        jev: serde_json::json!({}),
        rubric_version: "v9".into(),
        image_seen: true,
        carried: false,
    }
}

#[test]
fn 没归属的判断也明确发空item_key让引擎清掉旧归属() {
    // r66 v4：空串被 skip 掉，引擎当「没带」保留了旧归属，两键一直对不上
    let v = serde_json::to_value(judgement("asimocrafts-e1a771")).unwrap();
    assert_eq!(v["item_key"], serde_json::json!(""));
}

#[tokio::test]
async fn 提交的kind用引擎认的英文值() {
    use csw_collector_engineapi::{DeliverableKind, SubmitInput};
    use wiremock::matchers::body_string_contains;

    let srv = MockServer::start().await;
    // 引擎只认 output / supplement / edit。中文「产出」会被 400 bad_kind 挡回——
    // 09-23 切换后第一次真提交就是这么失败的，mock 当时不看这个字段
    Mock::given(method("POST"))
        .and(path("/tasks/9/deliverables"))
        .and(body_string_contains("name=\"kind\"\r\n\r\noutput"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"id": 1})))
        .mount(&srv)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"code": "bad_kind", "message": "kind 须为 output / supplement / edit"}),
        ))
        .mount(&srv)
        .await;

    let zip = std::env::temp_dir().join(format!("csw-submit-{}.zip", std::process::id()));
    std::fs::write(&zip, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    let r = client(&srv.uri())
        .submit(&SubmitInput {
            task_id: 9,
            kind: DeliverableKind::Output,
            zip_path: zip.clone(),
            file_name: "x.zip".into(),
            idem_key: "submit-9-x".into(),
            note: String::new(),
            affects_deliverable_id: None,
            item_key: String::new(),
        })
        .await;
    let _ = std::fs::remove_file(&zip);
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(DeliverableKind::Supplement.as_str(), "supplement");
}
