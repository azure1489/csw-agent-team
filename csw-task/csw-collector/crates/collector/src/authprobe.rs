//! 阶段 0.6 的登录转发自检：起一个真的 BFF，对一台真引擎把整条链路走一遍。
//!
//! 验的是七件事：登录成功、cookie 属性对、引擎 token 不外泄、`me` 能用、
//! access 到期能自动续、CSRF 拦得住、登出后会话真的没了。

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use csw_collector_engineapi::admin::AdminClient;

use crate::bff::{AuthState, CSRF_HEADER, SESSION_COOKIE, SessionStore};

pub async fn run(
    engine_url: &str,
    username: &str,
    password: &str,
    van: Option<&str>,
) -> Result<()> {
    let state = Arc::new(AuthState {
        engine: AdminClient::new(engine_url)?,
        store: SessionStore::default(),
        van_usernames: van.map(|v| vec![v.to_string()]).unwrap_or_default(),
        // 自检跑在 http 回环上，生产必须是 true
        secure_cookie: false,
    });
    let app = crate::bff::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });
    println!("BFF 监听 {addr}，引擎 {engine_url}\n");

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = format!("http://{addr}");
    let mut failures = Vec::new();

    // 1 登录
    let resp = http
        .post(format!("{base}/api/auth/login"))
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .context("调 /api/auth/login")?;
    let set_cookie = resp
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    check(
        "登录返回 200",
        status.is_success(),
        &format!("{status} {body}"),
        &mut failures,
    );

    let sid = set_cookie
        .split(';')
        .next()
        .and_then(|kv| kv.split_once('='))
        .filter(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    check(
        "下发了会话 cookie",
        !sid.is_empty(),
        &set_cookie,
        &mut failures,
    );
    check(
        "cookie 带 HttpOnly",
        set_cookie.contains("HttpOnly"),
        &set_cookie,
        &mut failures,
    );
    check(
        "cookie 带 SameSite=Lax",
        set_cookie.contains("SameSite=Lax"),
        &set_cookie,
        &mut failures,
    );
    check(
        "cookie 是不透明 id（不是 JWT）",
        !sid.contains('.'),
        &sid,
        &mut failures,
    );

    // 2 引擎 token 绝不能出现在给浏览器的任何东西里
    let leaked = {
        let s = state.store.get(&sid);
        let at = s
            .as_ref()
            .map(|s| s.engine.access_token.clone())
            .unwrap_or_default();
        let rc = s
            .as_ref()
            .map(|s| s.engine.refresh_cookie.clone())
            .unwrap_or_default();
        let blob = format!("{set_cookie}{body}");
        (!at.is_empty() && blob.contains(&at)) || (!rc.is_empty() && blob.contains(&rc))
    };
    check(
        "引擎 access / refresh 未外泄给浏览器",
        !leaked,
        "响应里出现了引擎 token",
        &mut failures,
    );

    let csrf = body
        .get("csrf_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    check(
        "返回了 csrf_token",
        !csrf.is_empty(),
        "缺 csrf_token",
        &mut failures,
    );
    let role = body.get("role").and_then(|v| v.as_str()).unwrap_or("");
    let engine_role = body
        .get("engine_role")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    println!("  用户 {username}  工作台角色 {role}  引擎角色 {engine_role}");
    if van == Some(username) {
        check(
            "van 映射生效",
            role == "van" || engine_role != "viewer",
            &format!("role={role} engine_role={engine_role}"),
            &mut failures,
        );
    }

    // 3 me
    let me = http
        .get(format!("{base}/api/auth/me"))
        .header(reqwest::header::COOKIE, format!("{SESSION_COOKIE}={sid}"))
        .send()
        .await?;
    check(
        "带 cookie 的 /me 返回 200",
        me.status().is_success(),
        &me.status().to_string(),
        &mut failures,
    );

    // 4 无 cookie 必须 401
    let anon = http.get(format!("{base}/api/auth/me")).send().await?;
    check(
        "不带 cookie 的 /me 返回 401",
        anon.status() == 401,
        &anon.status().to_string(),
        &mut failures,
    );

    // 5 access 过期能自动续：把会话里的到期时间拨到过去，再调一次 me
    if let Some(mut s) = state.store.get(&sid) {
        let old_refresh = s.engine.refresh_cookie.clone();
        s.engine.access_expires_at = 0;
        state.store.put(sid.clone(), s);
        let r = http
            .get(format!("{base}/api/auth/me"))
            .header(reqwest::header::COOKIE, format!("{SESSION_COOKIE}={sid}"))
            .send()
            .await?;
        check(
            "access 过期后自动续期",
            r.status().is_success(),
            &r.status().to_string(),
            &mut failures,
        );
        let rotated = state
            .store
            .get(&sid)
            .map(|s| s.engine.refresh_cookie != old_refresh)
            .unwrap_or(false);
        check(
            "续期后 refresh 已轮换",
            rotated,
            "引擎没有轮换 refresh",
            &mut failures,
        );
    }

    // 6 CSRF：会话有效但头不对，写接口要被挡
    {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::COOKIE,
            format!("{SESSION_COOKIE}={sid}").parse()?,
        );
        let good = crate::bff::check_write(&state, &to_axum(&h)).is_ok();
        check("缺 CSRF 头的写请求被挡", !good, "居然放行了", &mut failures);
        h.insert(CSRF_HEADER, csrf.parse()?);
        let ok = crate::bff::check_write(&state, &to_axum(&h)).is_ok();
        check("带正确 CSRF 头的写请求放行", ok, "被误挡", &mut failures);
        h.insert(CSRF_HEADER, "0".repeat(csrf.len()).parse()?);
        let bad = crate::bff::check_write(&state, &to_axum(&h)).is_ok();
        check("CSRF 头不对的写请求被挡", !bad, "居然放行了", &mut failures);
    }

    // 7 登出
    let out = http
        .post(format!("{base}/api/auth/logout"))
        .header(reqwest::header::COOKIE, format!("{SESSION_COOKIE}={sid}"))
        .send()
        .await?;
    check(
        "登出返回 204",
        out.status() == 204,
        &out.status().to_string(),
        &mut failures,
    );
    let after = http
        .get(format!("{base}/api/auth/me"))
        .header(reqwest::header::COOKIE, format!("{SESSION_COOKIE}={sid}"))
        .send()
        .await?;
    check(
        "登出后 /me 返回 401",
        after.status() == 401,
        &after.status().to_string(),
        &mut failures,
    );
    check(
        "会话已从存放处删除",
        state.store.is_empty(),
        "会话还在",
        &mut failures,
    );

    let _ = tx.send(());
    let _ = server.await;

    if failures.is_empty() {
        println!("\n全部通过");
        Ok(())
    } else {
        bail!("{} 项未通过：{}", failures.len(), failures.join("、"))
    }
}

fn to_axum(h: &reqwest::header::HeaderMap) -> axum::http::HeaderMap {
    let mut out = axum::http::HeaderMap::new();
    for (k, v) in h {
        if let (Ok(k), Ok(v)) = (
            axum::http::HeaderName::from_bytes(k.as_str().as_bytes()),
            axum::http::HeaderValue::from_bytes(v.as_bytes()),
        ) {
            out.insert(k, v);
        }
    }
    out
}

fn check(name: &str, ok: bool, detail: &str, failures: &mut Vec<String>) {
    if ok {
        println!("  通过  {name}");
    } else {
        println!("  失败  {name} — {detail}");
        failures.push(name.to_string());
    }
}
