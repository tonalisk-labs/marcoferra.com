//! The operations: availability, booking (at once, or held for a payment), the payment's
//! confirmation and release, cancelling, the booking as its invitation, and Stripe's webhook.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, RawQuery, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use jiff::Timestamp;
use kit_server::stripe::verify_webhook_signature;
use lettre::Address;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{OpenApi, ToSchema};

use crate::App;
use crate::config::EventType;
use crate::error::{
    ALREADY_BOOKED, ALREADY_PAID, ApiError, CANCELLED, InvalidMember, KEY_REUSED, NOT_PAID,
    Problem, REFUNDED,
};
use crate::google::url_segment;
use crate::grid::Rules;
use crate::ics::Method;
use crate::paid::Confirmation;
use crate::payment::Hold;
use crate::security::Purpose;
use crate::stripe::Session;

/// A meeting type's free starts and terms, and the token that dates this visit's form.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Availability {
    /// Every bookable start, RFC 3339 in UTC, earliest first.
    slots: Vec<String>,
    duration_minutes: u32,
    /// The price in the currency's minor unit. 0 is free.
    price_minor: i64,
    /// ISO 4217, in lower case.
    currency: String,
    /// Sent back with the booking: a form sent within seconds of this answer is refused.
    form_token: String,
}

#[derive(Deserialize)]
pub(crate) struct AvailabilityQuery {
    #[serde(default)]
    r#type: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BookingRequest {
    /// The meeting type's slug.
    r#type: String,
    /// One of the offered `slots`.
    start: String,
    name: String,
    email: String,
    #[serde(default)]
    message: String,
    /// The visitor's IANA time zone, in which the email states the time.
    #[serde(default)]
    time_zone: Option<String>,
    /// A field people never see: anything in it refuses the request.
    #[serde(default)]
    website: String,
    form_token: String,
    /// For a paid type, required: the visitor asks for the session within the 14-day withdrawal
    /// period and accepts losing the right of withdrawal once it is given in full.
    #[serde(default)]
    withdrawal_waiver: bool,
    /// Where the link the visitor followed was shared (its `utm_source`), such as `reddit`: 1 to
    /// 32 lowercase letters, digits, dots or hyphens. Kept with the booking.
    #[serde(default)]
    source: Option<String>,
}

/// A time held for its payment, and the Stripe Checkout page that pays for it.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Held {
    id: String,
    /// Stripe's hosted Checkout page: the browser goes there.
    checkout_url: String,
    /// When the hold ends unpaid, RFC 3339 in UTC.
    expires_at: String,
}

/// The Checkout session the success page returns with.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfirmRequest {
    session_id: String,
}

/// A payment by a method that settles later: the invitation follows once it does.
#[derive(Serialize, ToSchema)]
pub(crate) struct PaymentPending {
    detail: String,
}

/// What a cancellation refunded: 0 for a free meeting, or one cancelled too late.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Cancelled {
    refunded_minor: i64,
    /// ISO 4217, in lower case.
    currency: String,
}

/// A booked meeting and the two links its holder uses.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Booking {
    id: String,
    start: String,
    end: String,
    /// The invitation exactly as emailed.
    ics_url: String,
    /// The page that cancels the meeting.
    cancel_url: String,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CancelRequest {
    token: String,
    /// The meeting type's slug, as the cancellation link names it. A link from before types
    /// were named has none. The token covers it.
    #[serde(default)]
    r#type: Option<String>,
    /// For a paid type, the last moment the cancellation is refunded, in Unix seconds, as the
    /// link names it. The token covers it.
    #[serde(default)]
    refund: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct IcsQuery {
    token: String,
}

/// The token from Checkout's way back.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseRequest {
    token: String,
}

/// The visitor's address as the edge saw it: `CloudFront`'s viewer address in production.
pub(crate) struct ClientIp(IpAddr);

impl FromRequestParts<Arc<App>> for ClientIp {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        app: &Arc<App>,
    ) -> impl Future<Output = Result<Self, Infallible>> {
        let name = app.config.client_ip_header.as_str();
        let raw = parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok());
        let ip = raw.and_then(|raw| parse_ip(raw, name == "cloudfront-viewer-address"));
        std::future::ready(Ok(Self(ip.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)))))
    }
}

/// An address, or `CloudFront`'s `address:port` (`2001:db8::7:443` for IPv6, unbracketed); of a
/// forwarded list, the entry the edge itself added, which is the last.
fn parse_ip(raw: &str, with_port: bool) -> Option<IpAddr> {
    let last = raw.rsplit(',').next()?.trim();
    let address = if with_port {
        last.rsplit_once(':')?.0
    } else {
        last
    };
    address
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

fn json_body<T: DeserializeOwned>(headers: &HeaderMap, body: &[u8]) -> Result<T, ApiError> {
    let media = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let media = media
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !media.is_some_and(|media| media.eq_ignore_ascii_case("application/json")) {
        return Err(ApiError::status(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Send the body as application/json.",
        ));
    }
    serde_json::from_slice(body)
        .map_err(|err| ApiError::invalid(vec![ApiError::member("", &err.to_string())]))
}

fn idempotency_key(headers: &HeaderMap) -> Result<String, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if (16..=128).contains(&key.len()) && key.chars().all(allowed) {
        return Ok(key.to_owned());
    }
    let detail = "Send an Idempotency-Key of 16 to 128 letters, digits, hyphens or underscores.";
    Err(ApiError::invalid(vec![ApiError::member(
        "header:Idempotency-Key",
        detail,
    )]))
}

pub(crate) struct Valid {
    pub kind: EventType,
    pub start: Timestamp,
    pub name: String,
    pub email: Address,
    pub message: String,
    pub zone: String,
    pub waiver: bool,
    pub source: Option<String>,
}

fn validate(app: &App, request: &BookingRequest, now: Timestamp) -> Result<Valid, ApiError> {
    let limits = &app.config.limits;
    let mut errors: Vec<InvalidMember> = Vec::new();
    let mut require = |ok: bool, pointer: &str, detail: String| {
        if !ok {
            errors.push(ApiError::member(pointer, &detail));
        }
    };
    let name = request.name.trim();
    let named = !name.is_empty() && name.chars().count() <= limits.name_max_chars;
    require(
        named && !name.chars().any(char::is_control),
        "/name",
        format!("Give a name of up to {} characters.", limits.name_max_chars),
    );
    let email = request.email.trim();
    let address = (email.chars().count() <= limits.email_max_chars)
        .then(|| email.parse::<Address>().ok())
        .flatten();
    require(
        address.is_some(),
        "/email",
        "Give an email address the invitation can reach.".to_owned(),
    );
    let message = request.message.trim();
    let written = message.chars().count() <= limits.message_max_chars;
    let printable = !message
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'));
    require(
        written && printable,
        "/message",
        format!(
            "Keep the message within {} characters.",
            limits.message_max_chars
        ),
    );
    let kind = app.kind(&request.r#type);
    require(kind.is_some(), "/type", UNKNOWN_TYPE.to_owned());
    let start =
        request.start.parse::<Timestamp>().ok().filter(|&start| {
            kind.is_none_or(|kind| app.grid.is_start(now, start, Rules::of(kind)))
        });
    require(
        start.is_some(),
        "/start",
        "Choose one of the offered times.".to_owned(),
    );
    let zone = request
        .time_zone
        .as_deref()
        .filter(|zone| !zone.is_empty())
        .unwrap_or("UTC");
    require(
        zone.len() <= 64 && jiff::tz::db().get(zone).is_ok(),
        "/timeZone",
        "Name an IANA time zone.".to_owned(),
    );
    require(
        kind.is_none_or(|kind| kind.price_minor == 0 || request.withdrawal_waiver),
        "/withdrawalWaiver",
        "Confirm that the session may take place within the withdrawal period.".to_owned(),
    );
    let source = request
        .source
        .as_deref()
        .filter(|source| !source.is_empty());
    require(
        source.is_none_or(source_is_valid),
        "/source",
        "Name the source in 1 to 32 lowercase letters, digits, dots or hyphens.".to_owned(),
    );
    require(
        request.website.is_empty(),
        "/website",
        "Leave this field empty.".to_owned(),
    );
    let age = app.keys.form_age(&request.form_token, now);
    let (fastest, oldest) = (
        limits.min_fill_seconds,
        limits.form_token_max_age_minutes.saturating_mul(60),
    );
    let fresh = age.is_some_and(|age| (fastest..=oldest).contains(&age.as_secs()));
    require(
        fresh,
        "/formToken",
        "The form was sent too soon or too late. Reload the page and try again.".to_owned(),
    );
    match (kind, start, address) {
        (Some(kind), Some(start), Some(email)) if errors.is_empty() => Ok(Valid {
            kind: kind.clone(),
            start,
            name: name.to_owned(),
            email,
            message: message.to_owned(),
            zone: zone.to_owned(),
            waiver: request.withdrawal_waiver,
            source: source.map(str::to_owned),
        }),
        _ => Err(ApiError::invalid(errors)),
    }
}

fn source_is_valid(source: &str) -> bool {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-';
    (1..=32).contains(&source.len()) && source.chars().all(allowed)
}

const UNKNOWN_TYPE: &str = "Choose one of the offered meeting types.";

/// Lists the times a visitor may book a meeting type at: the host's grid within the type's
/// rules, less what the calendar has busy.
#[utoipa::path(
    get, path = "/v1/availability", operation_id = "get_availability",
    params(("type" = String, Query, description = "The meeting type's slug")),
    responses(
        (status = 200, description = "Bookable starts and a form token", body = Availability),
        (status = 400, description = "No such meeting type", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The calendar is not answering", body = Problem, content_type = "application/problem+json"),
    )
)]
pub(crate) async fn availability(
    State(app): State<Arc<App>>,
    Query(query): Query<AvailabilityQuery>,
) -> Result<Response, ApiError> {
    let kind = app
        .kind(&query.r#type)
        .ok_or_else(|| ApiError::invalid(vec![ApiError::member("query:type", UNKNOWN_TYPE)]))?;
    let now = Timestamp::now();
    let slots = app
        .free_starts(now, kind)
        .await?
        .iter()
        .map(Timestamp::to_string)
        .collect();
    let answer = Availability {
        slots,
        duration_minutes: kind.duration_minutes,
        price_minor: kind.price_minor,
        currency: kind.currency.clone(),
        form_token: app.keys.form(now),
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(answer)).into_response())
}

/// Books a start: 201 once the relay has a free type's invitation, or 202 with the Checkout that
/// pays for a paid type's held time. A repeated `Idempotency-Key` answers what it made.
#[utoipa::path(
    post, path = "/v1/bookings", operation_id = "create_booking",
    params(("Idempotency-Key" = String, Header, description = "One per booking attempt, reused on retry")),
    request_body = BookingRequest,
    responses(
        (status = 201, description = "Booked, and the invitation is on its way", body = Booking),
        (status = 202, description = "Held for payment at Stripe Checkout", body = Held),
        (status = 400, description = "Invalid. `errors` lists each member", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The time was taken, the address holds a booking, or the key was reused", body = Problem, content_type = "application/problem+json"),
        (status = 415, description = "The body is not JSON", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests from this address", body = Problem, content_type = "application/problem+json",
            headers(("Retry-After" = u64, description = "Seconds to wait"))),
        (status = 503, description = "The calendar or the mail relay failed, so nothing was booked", body = Problem, content_type = "application/problem+json"),
    )
)]
pub(crate) async fn create(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    app.admit(ip)?;
    let key = idempotency_key(&headers)?;
    let request: BookingRequest = json_body(&headers, &body)?;
    let now = Timestamp::now();
    let valid = validate(&app, &request, now)?;
    let _turn = app.booking.lock().await;
    let holder = hex(&Sha256::digest(valid.email.to_string().to_lowercase()));
    let upcoming = app.calendar.upcoming("mfEmail", &holder, now).await?;
    if let Some(earlier) = upcoming
        .iter()
        .find(|event| event.private("idem") == Some(key.as_str()))
    {
        if earlier.start.date_time != valid.start {
            return Err(ApiError::refused(
                &KEY_REUSED,
                "This Idempotency-Key was sent with another booking.",
            ));
        }
        if Hold::named(earlier) == Hold::Held {
            return Ok(held(app.checkout(earlier).await?));
        }
        return Ok((StatusCode::CREATED, Json(app.booking(earlier))).into_response());
    }
    if valid.kind.price_minor > 0 {
        // One hold per visitor at a time: a new one ends the last, which was left unpaid.
        for earlier in upcoming
            .iter()
            .filter(|event| Hold::named(event) == Hold::Held)
        {
            app.supersede(earlier).await?;
        }
        app.cap.check(now)?;
        return Ok(held(app.hold(&valid, (&key, &holder), now).await?));
    }
    // A paid booking never stands in the way of the free one.
    if let Some(booked) = upcoming
        .iter()
        .find(|event| event.private("mfState").is_none())
    {
        let detail = format!(
            "This address already has a meeting on {}. Cancel it from its email to choose another time.",
            crate::when(booked)
        );
        return Err(ApiError::refused(&ALREADY_BOOKED, detail));
    }
    app.cap.check(now)?;
    let event = app.book(&valid, &key, &holder).await?;
    Ok((StatusCode::CREATED, Json(app.booking(&event))).into_response())
}

fn held(pending: crate::paid::Pending) -> Response {
    let answer = Held {
        id: pending.id,
        checkout_url: pending.url,
        expires_at: pending.expires_at.to_string(),
    };
    (StatusCode::ACCEPTED, Json(answer)).into_response()
}

/// Confirms a paid booking from Stripe's success page, as the webhook would: the payment is
/// read back from Stripe, and the invitation sent once.
#[utoipa::path(
    post, path = "/v1/bookings:confirm", operation_id = "confirm_booking",
    request_body = ConfirmRequest,
    responses(
        (status = 200, description = "Booked, and the invitation sent", body = Booking),
        (status = 202, description = "Paid by a method that settles later. The invitation follows", body = PaymentPending),
        (status = 404, description = "No such payment", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Not paid, or paid too late and refunded", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests from this address", body = Problem, content_type = "application/problem+json",
            headers(("Retry-After" = u64, description = "Seconds to wait"))),
        (status = 503, description = "Stripe, the calendar or the mail relay failed. Try again", body = Problem, content_type = "application/problem+json"),
    )
)]
pub(crate) async fn confirm(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    app.admit(ip)?;
    let request: ConfirmRequest = json_body(&headers, &body)?;
    let session = &request.session_id;
    let well_formed = session.len() <= 255
        && session.starts_with("cs_")
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !well_formed {
        return Err(missing_payment());
    }
    let _turn = app.booking.lock().await;
    match app.confirm(session).await? {
        Confirmation::Booked(event) => Ok(Json(app.booking(&event)).into_response()),
        Confirmation::Pending => {
            let detail =
                "The payment is pending. The invitation follows by email once it is confirmed.";
            let answer = PaymentPending {
                detail: detail.to_owned(),
            };
            Ok((StatusCode::ACCEPTED, Json(answer)).into_response())
        }
        Confirmation::Unpaid => Err(ApiError::refused(
            &NOT_PAID,
            "This payment was not made, so nothing is booked.",
        )),
        Confirmation::Refunded => Err(ApiError::refused(
            &REFUNDED,
            "The payment arrived after the time was released, so it is refunded in full. Please choose another time.",
        )),
    }
}

/// A booking's custom methods (AIP-136), `{id}:cancel` and `{id}:release`, told apart by the
/// verb after the colon.
pub(crate) async fn act(
    State(app): State<Arc<App>>,
    ClientIp(ip): ClientIp,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    app.admit(ip)?;
    match name.split_once(':') {
        Some((id, "cancel")) => {
            let cancelled = cancel(&app, id, json_body(&headers, &body)?).await?;
            Ok(Json(cancelled).into_response())
        }
        Some((id, "release")) => {
            let request: ReleaseRequest = json_body(&headers, &body)?;
            Ok(release(&app, id, &request.token).await?.into_response())
        }
        _ => Err(ApiError::status(
            StatusCode::NOT_FOUND,
            "There is no such resource.",
        )),
    }
}

/// Cancels a meeting and sends the cancellation, which removes it from the visitor's calendar.
/// A paid meeting cancelled early enough is refunded in full first. Later, it is not refunded.
#[utoipa::path(
    post, path = "/v1/bookings/{id}:cancel", operation_id = "cancel_booking",
    params(("id" = String, Path, description = "The booking's id")),
    request_body = CancelRequest,
    responses(
        (status = 200, description = "Cancelled, and the cancellation sent. Says what was refunded", body = Cancelled),
        (status = 400, description = "Invalid. `errors` lists each member", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not open this booking", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "No such booking", body = Problem, content_type = "application/problem+json"),
        (status = 410, description = "Already cancelled", body = Problem, content_type = "application/problem+json"),
        (status = 415, description = "The body is not JSON", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests from this address", body = Problem, content_type = "application/problem+json",
            headers(("Retry-After" = u64, description = "Seconds to wait"))),
        (status = 503, description = "Stripe, the calendar or the mail relay failed, so the meeting stands", body = Problem, content_type = "application/problem+json"),
    )
)]
async fn cancel(app: &App, id: &str, request: CancelRequest) -> Result<Cancelled, ApiError> {
    let subject = cancel_subject(id, request.r#type.as_deref(), request.refund);
    if !app.keys.holds(Purpose::Cancel, &subject, &request.token) {
        return Err(ApiError::status(
            StatusCode::FORBIDDEN,
            "This cancellation link is not valid.",
        ));
    }
    let _turn = app.booking.lock().await;
    let event = app.calendar.get(id).await?.ok_or_else(missing)?;
    if event.cancelled() {
        return Err(ApiError::refused(
            &CANCELLED,
            "This meeting was already cancelled.",
        ));
    }
    if Hold::named(&event) == Hold::Held {
        return Err(missing());
    }
    let currency = event
        .private("type")
        .and_then(|slug| app.kind(slug))
        .map_or_else(|| "eur".to_owned(), |kind| kind.currency.clone());
    let (refunded_minor, said) = app.refund_on_cancel(&event, Timestamp::now()).await?;
    app.cancel(&event, &said.into_iter().collect::<Vec<_>>())
        .await?;
    Ok(Cancelled {
        refunded_minor,
        currency,
    })
}

/// Gives up a time held for payment, from Checkout's way back: the session expires at Stripe
/// and the time is free again at once.
#[utoipa::path(
    post, path = "/v1/bookings/{id}:release", operation_id = "release_booking",
    params(("id" = String, Path, description = "The held booking's id")),
    request_body = ReleaseRequest,
    responses(
        (status = 204, description = "Released, or already free"),
        (status = 400, description = "Invalid. `errors` lists each member", body = Problem, content_type = "application/problem+json"),
        (status = 403, description = "The token does not open this hold", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Already paid for, or its payment is under way", body = Problem, content_type = "application/problem+json"),
        (status = 415, description = "The body is not JSON", body = Problem, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests from this address", body = Problem, content_type = "application/problem+json",
            headers(("Retry-After" = u64, description = "Seconds to wait"))),
        (status = 503, description = "Stripe or the calendar failed. Try again", body = Problem, content_type = "application/problem+json"),
    )
)]
async fn release(app: &App, id: &str, token: &str) -> Result<StatusCode, ApiError> {
    if !app.keys.holds(Purpose::Release, id, token) {
        return Err(ApiError::status(
            StatusCode::FORBIDDEN,
            "This link does not release a held time.",
        ));
    }
    let Some(event) = app.calendar.get(id).await? else {
        return Ok(StatusCode::NO_CONTENT);
    };
    match Hold::named(&event) {
        Hold::Held => app.give_up(&event).await?,
        Hold::Paid | Hold::Confirmed => {
            return Err(ApiError::refused(
                &ALREADY_PAID,
                "This time is already paid for and booked.",
            ));
        }
        Hold::Gone => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Stripe's signed events, each acted on under the booking lock by its booking's state, so a
/// repeat does nothing twice; an event not used here is acknowledged and ignored.
pub(crate) async fn webhook(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let secret = &app.config.secrets.stripe_webhook_secret;
    if !verify_webhook_signature(&body, signature, secret) {
        return Err(ApiError::status(
            StatusCode::BAD_REQUEST,
            "The signature does not match.",
        ));
    }
    let event: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::status(StatusCode::BAD_REQUEST, "The event is not JSON."))?;
    let kind = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let object = event.pointer("/data/object").cloned().unwrap_or_default();
    let session = || {
        Session::read(&object)
            .ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST, "The event names no session."))
    };
    let _turn = app.booking.lock().await;
    match kind {
        "checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
            let session = session()?;
            if session.paid() {
                app.confirm(&session.id).await?;
            }
        }
        "checkout.session.async_payment_failed" => drop(app.release(&session()?, true).await?),
        "checkout.session.expired" => drop(app.release(&session()?, false).await?),
        "charge.refunded" => app.refunded(&object).await?,
        _ => {}
    }
    Ok(StatusCode::OK)
}

/// A booking in its one representation (RFC 9110 §12.5.1): its invitation, byte for byte the
/// one its email carried.
#[utoipa::path(
    get, path = "/v1/bookings/{id}", operation_id = "get_booking",
    params(
        ("id" = String, Path, description = "The booking's id"),
        ("token" = String, Query, description = "The token from the booking's `icsUrl`"),
    ),
    responses(
        (status = 200, description = "The iCalendar invitation", body = String, content_type = "text/calendar"),
        (status = 403, description = "The token does not open this booking", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "No such booking", body = Problem, content_type = "application/problem+json"),
        (status = 406, description = "`Accept` admits no `text/calendar`", body = Problem, content_type = "application/problem+json"),
        (status = 410, description = "Cancelled", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The calendar is not answering", body = Problem, content_type = "application/problem+json"),
    )
)]
pub(crate) async fn get(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Query(query): Query<IcsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !admits_calendar(&headers) {
        return Err(ApiError::status(
            StatusCode::NOT_ACCEPTABLE,
            "A booking is served as text/calendar only.",
        ));
    }
    if !app.keys.holds(Purpose::Ics, &id, &query.token) {
        return Err(ApiError::status(
            StatusCode::FORBIDDEN,
            "This invitation link is not valid.",
        ));
    }
    let event = app.calendar.get(&id).await?.ok_or_else(missing)?;
    if event.cancelled() {
        return Err(ApiError::refused(&CANCELLED, "This meeting was cancelled."));
    }
    let invite = app.ics(Method::Request, &event).ok_or_else(missing)?;
    let headers = [
        (
            header::CONTENT_TYPE,
            "text/calendar; charset=UTF-8; method=REQUEST",
        ),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"invite.ics\"",
        ),
        (header::CACHE_CONTROL, "no-store"),
        (header::VARY, "Accept"),
    ];
    Ok((headers, invite).into_response())
}

/// Whether `Accept` admits `text/calendar` by its most specific matching range, weighed above
/// zero (RFC 9110 §12.5.1); with no range matching, only when no `Accept` was sent.
fn admits_calendar(headers: &HeaderMap) -> bool {
    let fields = headers.get_all(header::ACCEPT).iter();
    let ranges = fields.flat_map(|field| field.to_str().unwrap_or_default().split(','));
    let weighed = ranges.filter_map(|range| {
        let mut parts = range.split(';');
        let specificity = match parts.next()?.trim().to_ascii_lowercase().as_str() {
            "text/calendar" => 2,
            "text/*" => 1,
            "*/*" => 0,
            _ => return None,
        };
        let mut params = parts.filter_map(|param| param.split_once('='));
        let weight = params.find(|(name, _)| name.trim().eq_ignore_ascii_case("q"));
        let above_zero =
            |(_, weight): (&str, &str)| weight.contains(|digit| ('1'..='9').contains(&digit));
        Some((specificity, weight.is_none_or(above_zero)))
    });
    let sent = headers.contains_key(header::ACCEPT);
    let best = weighed.max_by_key(|&(specificity, _)| specificity);
    best.map_or(!sent, |(_, admitted)| admitted)
}

/// The calendar's unversioned address, which links in invitations already sent name: kept until
/// the last of those meetings has passed. Relative, so it holds behind the edge's `/api`.
pub(crate) async fn moved(Path(id): Path<String>, RawQuery(query): RawQuery) -> Response {
    let query = query.map(|query| format!("?{query}")).unwrap_or_default();
    let location = format!("../../v1/bookings/{}{query}", url_segment(&id));
    (
        StatusCode::PERMANENT_REDIRECT,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// What a cancellation token signs: the id, and the type and refund deadline when the link
/// names them. Neither a slug nor an event id holds `/`, so no subject reads as another.
fn cancel_subject(id: &str, kind: Option<&str>, refund: Option<i64>) -> String {
    let parts = [
        kind.map(str::to_owned),
        refund.map(|seconds| seconds.to_string()),
    ];
    parts
        .into_iter()
        .flatten()
        .fold(id.to_owned(), |subject, part| format!("{subject}/{part}"))
}

fn missing() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND, "There is no such booking.")
}

fn missing_payment() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND, "There is no such payment.")
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(nibble.into(), 16))
        .collect()
}

impl App {
    pub(crate) fn booking(&self, event: &crate::google::Event) -> Booking {
        let id = &event.id;
        Booking {
            id: id.clone(),
            start: event.start.date_time.to_string(),
            end: event.end.date_time.to_string(),
            ics_url: format!(
                "/api/v1/bookings/{id}?token={}",
                self.keys.booking(Purpose::Ics, id)
            ),
            cancel_url: self.cancel_path(event),
        }
    }

    /// The page that cancels a booking, naming its type so the page can say what a
    /// cancellation refunds; the token signs the id and the type together.
    pub(crate) fn cancel_path(&self, event: &crate::google::Event) -> String {
        let id = &event.id;
        let kind = event.private("type");
        let refund = self.refund_deadline(event).map(Timestamp::as_second);
        let token = self
            .keys
            .booking(Purpose::Cancel, &cancel_subject(id, kind, refund));
        let named = kind.map(|slug| format!("&type={slug}")).unwrap_or_default();
        let until = refund
            .map(|seconds| format!("&refund={seconds}"))
            .unwrap_or_default();
        format!("/book/cancel/?id={id}{named}{until}&token={token}")
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "marcoferra.com booking API",
        description = "Books a meeting in the owner's Google Calendar and sends the invitation by email. \
                       Every refusal is RFC 9457 problem+json, and a typed one carries a `tag:marcoferra.com,2026:` type.",
    ),
    servers((url = "/api", description = "The site's own origin")),
    // Public: a booking's own token, in its query or body, opens it.
    security(()),
    paths(availability, create, confirm, get, cancel, release),
)]
pub(crate) struct ApiDoc;

#[cfg(test)]
mod tests {
    use axum::extract::{Path, RawQuery};
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use reqwest::Url;

    use super::{admits_calendar, cancel_subject, moved, parse_ip, source_is_valid};
    use crate::TestResult;
    use crate::security::{Keys, Purpose};

    /// A browser's link, a client asking for the calendar, and no `Accept` at all are served;
    /// a client refusing every calendar, or naming only other types, is not.
    #[test]
    fn a_booking_is_served_to_whoever_accepts_a_calendar() -> TestResult {
        let cases = [
            (None, true),
            (Some("text/calendar"), true),
            (Some("TEXT/Calendar; method=REQUEST"), true),
            (Some("text/html,application/xhtml+xml,*/*;q=0.8"), true),
            (Some("text/*;q=0.5"), true),
            (Some("application/json"), false),
            (Some("*/*;q=0"), false),
            (Some("text/calendar;q=0, */*"), false),
        ];
        for (accept, admitted) in cases {
            let mut headers = HeaderMap::new();
            if let Some(accept) = accept {
                headers.insert(header::ACCEPT, HeaderValue::from_str(accept)?);
            }
            assert_eq!(admits_calendar(&headers), admitted, "{accept:?}");
        }
        Ok(())
    }

    /// For good, its token kept, with the public `/api` prefix or without it.
    #[tokio::test]
    async fn the_old_calendar_address_moves_to_the_booking() -> TestResult {
        let query = RawQuery(Some("token=t".to_owned()));
        let response = moved(Path("mfabc".to_owned()), query).await;
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
        let location = response.headers().get(header::LOCATION);
        let location = location.ok_or("no Location")?.to_str()?;
        for prefix in ["/api", ""] {
            let old = Url::parse(&format!(
                "https://x.test{prefix}/bookings/mfabc/ics?token=t"
            ))?;
            let new = format!("https://x.test{prefix}/v1/bookings/mfabc?token=t");
            assert_eq!(old.join(location)?.as_str(), new);
        }
        Ok(())
    }

    /// A link's token holds for its id, type and refund deadline together; changing or dropping
    /// either breaks it, and a link from before types were named still holds for its id alone.
    #[test]
    fn a_cancellation_token_signs_what_its_link_names() -> TestResult {
        let keys = Keys::new("0123456789abcdef0123456789abcdef")?;
        let signed = |kind, refund| cancel_subject("mfabc", kind, refund);
        let token = keys.booking(
            Purpose::Cancel,
            &signed(Some("focused-session"), Some(1_800_000_000)),
        );
        assert!(keys.holds(
            Purpose::Cancel,
            &signed(Some("focused-session"), Some(1_800_000_000)),
            &token
        ));
        assert!(!keys.holds(
            Purpose::Cancel,
            &signed(Some("focused-session"), Some(1_900_000_000)),
            &token
        ));
        assert!(!keys.holds(
            Purpose::Cancel,
            &signed(Some("focused-session"), None),
            &token
        ));
        assert!(!keys.holds(
            Purpose::Cancel,
            &signed(Some("intro"), Some(1_800_000_000)),
            &token
        ));
        assert!(!keys.holds(Purpose::Cancel, &signed(None, None), &token));
        let old = keys.booking(Purpose::Cancel, "mfabc");
        assert!(keys.holds(Purpose::Cancel, &signed(None, None), &old));
        Ok(())
    }

    #[test]
    fn a_source_is_a_short_lowercase_name() {
        for good in [
            "reddit",
            "news.ycombinator.com",
            "r-cscareerquestions",
            "a",
            &"a".repeat(32),
        ] {
            assert!(source_is_valid(good), "{good}");
        }
        for bad in [
            "",
            "Reddit",
            "red dit",
            "reddit/",
            "rédit",
            "a_b",
            &"a".repeat(33),
        ] {
            assert!(!source_is_valid(bad), "{bad}");
        }
    }

    #[test]
    fn the_client_address_is_read_from_the_viewer_header_or_the_forwarded_list() {
        for (raw, with_port, expected) in [
            ("203.0.113.7:46532", true, "203.0.113.7"),
            ("2001:db8::7:46532", true, "2001:db8::7"),
            ("198.18.0.4", false, "198.18.0.4"),
            ("10.0.0.1, 198.18.0.4", false, "198.18.0.4"),
        ] {
            assert_eq!(
                parse_ip(raw, with_port).map(|ip| ip.to_string()),
                Some(expected.to_owned()),
                "{raw}"
            );
        }
        assert_eq!(parse_ip("nonsense", false), None);
    }
}
