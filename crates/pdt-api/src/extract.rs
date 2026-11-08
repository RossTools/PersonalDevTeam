//! Extractors that refuse a request as an [`ApiError`] (JSON `{code, message}`), where axum's own answer in plain text.
//!
//! Handlers take these, or `Result<Json<T>, JsonRejection>` when they treat a missing body themselves; the test below
//! keeps bare `Json`, `Query` and typed `Path` out of handler signatures.

use crate::error::ApiError;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

/// A JSON body: 400/415/422 `invalid_input` saying what is wrong with it.
pub struct ApiJson<T>(pub T);

/// A query string: 400 `invalid_input` naming the field that did not parse.
pub struct ApiQuery<T>(pub T);

/// Path segments parsed as more than a string (a version number): 400 `invalid_input` when one does not parse.
pub struct ApiPath<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let axum::Json(v) = axum::Json::<T>::from_request(req, state).await?;
        Ok(Self(v))
    }
}

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let axum::extract::Query(v) = axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|e: QueryRejection| ApiError::invalid(e.body_text()))?;
        Ok(Self(v))
    }
}

impl<T: DeserializeOwned + Send, S: Send + Sync> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        let axum::extract::Path(v) =
            axum::extract::Path::<T>::from_request_parts(parts, state).await.map_err(|e: PathRejection| {
                // A route whose segments don't fit its handler is PDT's mistake, not the caller's.
                if e.status().is_client_error() {
                    ApiError::invalid(e.body_text())
                } else {
                    ApiError::internal(e.body_text())
                }
            })?;
        Ok(Self(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::{get, post};
    use tower::ServiceExt;

    #[derive(serde::Deserialize)]
    struct Q {
        #[allow(dead_code)]
        n: u32,
    }

    async fn refusal(app: Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).expect("a JSON body"))
    }

    #[tokio::test]
    async fn rejections_answer_code_and_message() {
        let app = Router::new()
            .route("/q", get(|ApiQuery(_): ApiQuery<Q>| async {}))
            .route("/p/{v}", get(|ApiPath(_): ApiPath<u32>| async {}))
            .route("/j", post(|ApiJson(_): ApiJson<Q>| async {}));
        let (status, body) = refusal(app.clone(), Request::get("/q?n=x").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
        assert!(body["message"].as_str().unwrap().contains("n"), "{body}");

        let (status, body) = refusal(app.clone(), Request::get("/p/x").body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");

        let json = |b: &'static str| {
            Request::post("/j").header(header::CONTENT_TYPE, "application/json").body(Body::from(b)).unwrap()
        };
        let (status, body) = refusal(app.clone(), json(r#"{"n":"x"}"#)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["code"], "invalid_input");
        let (status, body) = refusal(app.clone(), json("{")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_input");
        let (status, body) = refusal(app, Request::post("/j").body(Body::from("{}")).unwrap()).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body["code"], "invalid_input");
    }

    /// Handlers take the extractors above: axum's own `Json`, `Query` and typed `Path` refuse in plain text.
    #[test]
    fn handlers_refuse_in_json() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = vec![];
        crate::error::tests::sources(&root.join("src"), &mut files);
        for p in std::fs::read_dir(root.join("..")).unwrap().flatten().map(|e| e.path()) {
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("pdt-int-")) {
                crate::error::tests::sources(&p.join("src"), &mut files);
            }
        }
        // Split, so this list doesn't match itself.
        let bare = [concat!("): Json", "<"), concat!("): Query", "<")];
        let path = concat!("): Path", "<");
        let mut found = vec![];
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, line) in text.lines().enumerate() {
                let line = line.replace("axum::extract::", "").replace("axum::", "");
                let path_typed = line.match_indices(path).any(|(at, _)| {
                    let ty = &line[at + path.len()..];
                    !["String>", "(String, String)>"].iter().any(|ok| ty.starts_with(ok))
                });
                if bare.iter().any(|b| line.contains(b)) || path_typed {
                    found.push(format!("{}:{}: {}", f.display(), n + 1, line.trim()));
                }
            }
        }
        assert!(found.is_empty(), "use ApiJson / ApiQuery / ApiPath (crate::extract):\n{}", found.join("\n"));
    }
}
