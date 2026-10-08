//! 知识库页的杂志背景接口（方案 §6「HTTP / MCP」、步骤 6）：
//!
//! - `GET  /api/kb/magazine/search?q=&book=&limit=`：文字检索（viewer）
//! - `GET  /api/kb/magazine/{id}`：条目详情（viewer）
//! - `POST /api/kb/image-search?limit=`：以图搜图（operator；请求体就是图片字节，≤20MB）
//!
//! 都不重排；以图搜图占一次向量服务（GPU 全进程串行），所以只给 operator。
//! 连接锁不跨 `await` 持有，理由同 `kb_search`。

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use csw_collector_core::vector::EmbedInput;
use csw_collector_kb::magazine_search::{self as ms, MagazineHit, TextQuery};

use super::http::{ApiError, AppState, need_operator};
use crate::bff::Session;

/// 以图搜图的请求体上限（路由上用 `DefaultBodyLimit` 放到这么大）
pub const MAX_IMAGE_BYTES: usize = 20 << 20;

#[derive(Deserialize)]
pub struct MagQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    book: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

fn hits_json(hits: &[MagazineHit]) -> Value {
    json!({ "count": hits.len(), "items": hits })
}

fn svc(st: &AppState) -> Result<&super::services::Services, ApiError> {
    st.svc.as_deref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "知识库还没起来".into(),
    ))
}

pub async fn search(
    State(st): State<Arc<AppState>>,
    Query(q): Query<MagQuery>,
) -> Result<Json<Value>, ApiError> {
    let text = q.q.trim();
    if text.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "要搜什么".into()));
    }
    let svc = svc(&st)?;
    let vector = match svc
        .vector
        .embed(&[EmbedInput::Text(text.chars().take(2000).collect())])
        .await
    {
        Ok(mut vs) if !vs.is_empty() => Some(vs.remove(0)),
        Ok(_) => None,
        Err(e) => {
            // 算不出向量不算错：全文那路照样能召回
            tracing::warn!(原因 = %format!("{e:#}"), "杂志检索算查询向量失败，只走全文");
            None
        }
    };
    let tq = TextQuery {
        text,
        vector: vector.as_deref(),
        book_key: q.book.as_deref().filter(|b| !b.is_empty()),
        limit: q.limit.clamp(1, 100),
    };
    let ids = ms::vector_ids(&svc.store, &tq)
        .await
        .map_err(ApiError::any)?;
    let conn = st.conn.lock().await;
    let hits =
        ms::fuse(&conn, &svc.tok, &tq, &ids, &st.cfg.vector.embed_model).map_err(ApiError::any)?;
    Ok(Json(hits_json(&hits)))
}

pub async fn item(
    State(st): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    let conn = st.conn.lock().await;
    let it = ms::get(&conn, id, &st.cfg.vector.embed_model)
        .map_err(ApiError::any)?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "没有这条杂志条目".into()))?;
    Ok(Json(json!(it)))
}

#[derive(Deserialize)]
pub struct ImgQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

pub async fn image_search(
    State(st): State<Arc<AppState>>,
    axum::Extension(s): axum::Extension<Session>,
    Query(q): Query<ImgQuery>,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    need_operator(&s, "以图搜图")?;
    if body.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "请求体要是一张图片".into(),
        ));
    }
    let svc = svc(&st)?;
    let side = st.cfg.magazine.image_side;
    let started = std::time::Instant::now();
    let b64 = tokio::task::spawn_blocking(move || {
        csw_collector_kb::magazine_embed::encode_b64(&body, side)
    })
    .await
    .map_err(|e| ApiError::any(e.into()))?
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, format!("图片读不出来：{e:#}")))?;
    let qv = svc
        .vector
        .embed(&[EmbedInput::Image(b64)])
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("向量服务没算出来：{e:#}"),
            )
        })?
        .into_iter()
        .next()
        .ok_or(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "向量服务没返回向量".into(),
        ))?;
    let embed_ms = started.elapsed().as_millis() as u64;
    let limit = q.limit.clamp(1, 100);
    let raw = ms::search_image(&svc.store, &qv, limit)
        .await
        .map_err(ApiError::any)?;
    let conn = st.conn.lock().await;
    let hits =
        ms::image_hits(&conn, &raw, limit, &st.cfg.vector.embed_model).map_err(ApiError::any)?;
    let mut v = hits_json(&hits);
    v["embed_ms"] = json!(embed_ms);
    v["total_ms"] = json!(started.elapsed().as_millis() as u64);
    tracing::info!(条数 = hits.len(), 算向量毫秒 = embed_ms, 操作人 = %s.engine.user.username, "以图搜图");
    Ok(Json(v))
}
