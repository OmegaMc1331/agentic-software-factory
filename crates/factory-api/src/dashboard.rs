use std::path::Path;

use axum::Router;

#[cfg_attr(feature = "embedded-dashboard", allow(unused_variables))]
pub fn router(root: &Path) -> Router {
    #[cfg(feature = "embedded-dashboard")]
    {
        embedded::router()
    }
    #[cfg(not(feature = "embedded-dashboard"))]
    {
        disk::router(root)
    }
}

#[cfg(feature = "embedded-dashboard")]
mod embedded {
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::any;
    use axum::Router;
    use rust_embed::RustEmbed;

    // Relative to this crate's manifest directory (crates/factory-api).
    #[derive(RustEmbed)]
    #[folder = "../../apps/dashboard/dist/"]
    struct Assets;

    async fn serve(request: Request<Body>) -> Response<Body> {
        let path = request.uri().path().trim_start_matches('/');
        let file = Assets::get(path).or_else(|| Assets::get("index.html"));
        match file {
            Some(file) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", file.metadata.mimetype())
                .body(Body::from(file.data.into_owned()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }

    pub fn router() -> Router {
        Router::new().fallback_service(any(serve))
    }
}

#[cfg(not(feature = "embedded-dashboard"))]
mod disk {
    use std::path::{Path, PathBuf};

    use axum::Router;
    use tower_http::services::{ServeDir, ServeFile};

    pub fn router(root: &Path) -> Router {
        let dir = find_dashboard_dir(root).unwrap_or_else(dashboard_stub_dir);
        Router::new().fallback_service(
            ServeDir::new(&dir).not_found_service(ServeFile::new(dir.join("index.html"))),
        )
    }

    fn dashboard_stub_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("factory-dashboard-stub");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><meta charset=\"utf-8\"><title>Agentic Software Factory</title>\
             <style>body{font-family:system-ui,sans-serif;background:#0f1115;color:#d7dce3;margin:48px auto;max-width:560px;line-height:1.6}</style>\
             <h1>Dashboard not built</h1>\
             <p>The dashboard has not been built yet. From the project root run:</p>\
             <pre>cd apps/dashboard\nnpm install\nnpm run build</pre>\
             <p>Then restart <code>factory start</code>.</p>",
        )
        .ok();
        dir
    }

    fn find_dashboard_dir(start: &Path) -> Option<PathBuf> {
        let mut cursor = Some(start.to_path_buf());
        while let Some(dir) = cursor {
            let candidate = dir.join("apps").join("dashboard").join("dist");
            if candidate.join("index.html").is_file() {
                return Some(candidate);
            }
            cursor = dir.parent().map(Path::to_path_buf);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let mut cursor = Some(parent.to_path_buf());
                while let Some(dir) = cursor {
                    let candidate = dir.join("apps").join("dashboard").join("dist");
                    if candidate.join("index.html").is_file() {
                        return Some(candidate);
                    }
                    cursor = dir.parent().map(Path::to_path_buf);
                }
            }
        }
        None
    }
}
