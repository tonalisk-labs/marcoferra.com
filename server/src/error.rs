//! Refusals as RFC 9457 problem+json. A problem a client acts on is typed by a `tag:` URI;
//! any other is `about:blank`, titled by its status.

use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
pub(crate) use kit_server::problem::{InvalidMember, Problem};

#[derive(Debug)]
pub(crate) struct ProblemType {
    pub slug: &'static str,
    pub status: StatusCode,
    pub title: &'static str,
}

macro_rules! problem_types {
    ($($name:ident = ($slug:literal, $status:ident, $title:literal);)*) => {$(
        pub(crate) const $name: ProblemType =
            ProblemType { slug: $slug, status: StatusCode::$status, title: $title };
    )*};
}

problem_types! {
    VALIDATION = ("validation", BAD_REQUEST, "The request is not valid");
    SLOT_TAKEN = ("slot-taken", CONFLICT, "That time is no longer free");
    ALREADY_BOOKED = ("already-booked", CONFLICT, "A meeting is already booked");
    KEY_REUSED = ("idempotency-key-reused", CONFLICT, "Idempotency key reused");
    CANCELLED = ("booking-cancelled", GONE, "The meeting is already cancelled");
    ALREADY_PAID = ("already-paid", CONFLICT, "The time is already paid for");
    REFUNDED = ("payment-refunded", CONFLICT, "The payment was refunded");
    NOT_PAID = ("not-paid", CONFLICT, "The payment was not made");
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    problem: Option<&'static ProblemType>,
    detail: Option<String>,
    errors: Vec<InvalidMember>,
    wait: Option<Duration>,
}

impl ApiError {
    pub(crate) fn status(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            status,
            problem: None,
            detail: Some(detail.into()),
            errors: Vec::new(),
            wait: None,
        }
    }

    pub(crate) fn refused(problem: &'static ProblemType, detail: impl Into<String>) -> Self {
        Self {
            problem: Some(problem),
            ..Self::status(problem.status, detail)
        }
    }

    pub(crate) fn invalid(errors: Vec<InvalidMember>) -> Self {
        Self {
            errors,
            ..Self::refused(&VALIDATION, "The request did not pass validation.")
        }
    }

    pub(crate) fn member(pointer: &str, detail: &str) -> InvalidMember {
        InvalidMember::new(pointer, detail)
    }

    /// The calendar or the mail relay failed; the client may try again.
    pub(crate) fn upstream(detail: impl Into<String>) -> Self {
        Self::status(StatusCode::SERVICE_UNAVAILABLE, detail)
    }

    /// Adds `Retry-After`: how long the client waits before trying again.
    pub(crate) fn after(self, wait: Duration) -> Self {
        Self {
            wait: Some(wait),
            ..self
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}",
            self.status,
            self.detail.as_deref().unwrap_or_default()
        )
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(status = self.status.as_u16(), detail = ?self.detail, "request failed");
        }
        let problem = self.problem.map_or_else(
            || Problem::untyped(self.status),
            |known| {
                let uri = format!("tag:marcoferra.com,2026:{}", known.slug);
                Problem::typed(uri, self.status, known.title)
            },
        );
        problem
            .detailed(self.detail)
            .with_errors(self.errors)
            .respond(self.wait)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::{HeaderValue, StatusCode};
    use axum::response::IntoResponse;

    use super::{ApiError, SLOT_TAKEN};
    use crate::TestResult;

    #[tokio::test]
    async fn a_typed_refusal_is_problem_json_with_its_type_and_wait() -> TestResult {
        let response = ApiError::refused(&SLOT_TAKEN, "Taken.")
            .after(Duration::from_millis(1200))
            .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let headers = response.headers();
        assert_eq!(
            headers.get("content-type").map(HeaderValue::as_bytes),
            Some(&b"application/problem+json"[..])
        );
        assert_eq!(
            headers.get("retry-after").map(HeaderValue::as_bytes),
            Some(&b"2"[..])
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
        let problem: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(
            problem.pointer("/type"),
            Some(&"tag:marcoferra.com,2026:slot-taken".into())
        );
        Ok(())
    }
}
