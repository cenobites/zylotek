use std::time::Instant;

use axum::{extract::Request, middleware::Next, response::Response};

use crate::errors::ApiError;

pub async fn logging_middleware(req: Request, next: Next) -> Result<Response, ApiError> {
    let time = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let response = next.run(req).await;
    log::info!(
        "{} {} -> {} ({:?} ms)",
        method,
        uri,
        response.status(),
        time.elapsed().as_millis()
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use axum::{Router, http::StatusCode, routing::get};

    use super::*;
    use crate::test_support::send;

    struct CaptureLogger(Mutex<Vec<String>>);

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            self.0.lock().unwrap().push(record.args().to_string());
        }

        fn flush(&self) {}
    }

    fn logger() -> &'static CaptureLogger {
        static LOGGER: OnceLock<&'static CaptureLogger> = OnceLock::new();
        LOGGER.get_or_init(|| {
            let logger: &'static CaptureLogger =
                Box::leak(Box::new(CaptureLogger(Mutex::default())));
            log::set_logger(logger).unwrap();
            log::set_max_level(log::LevelFilter::Info);
            logger
        })
    }

    #[tokio::test]
    async fn logs_method_uri_and_status_and_passes_the_response_through() {
        let logger = logger();
        let router = Router::new()
            .route(
                "/teapot",
                get(|| async { (StatusCode::IM_A_TEAPOT, "short and stout") }),
            )
            .layer(axum::middleware::from_fn(logging_middleware));

        let (status, body) = send(&router, axum::http::Method::GET, "/teapot?x=1", None).await;

        assert_eq!(status, StatusCode::IM_A_TEAPOT);
        assert_eq!(body, "short and stout");
        assert!(
            logger
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.starts_with("GET /teapot?x=1 -> 418 I'm a teapot")),
            "captured log lines"
        );

        log::Log::flush(logger);
        assert!(log::Log::enabled(logger, &log::Metadata::builder().build()));
    }
}
