//! Meeting booking for marcoferra.com: one axum router, served locally and by AWS Lambda behind
//! `CloudFront`. Google Calendar holds every booking; Stripe's webhook confirms a paid one.

mod busy;
mod config;
mod error;
mod google;
mod grid;
mod ics;
mod mail;
mod paid;
mod payment;
mod routes;
mod security;
mod stripe;

use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{Method as HttpMethod, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use kit_server::config::var;
use kit_server::limit::RateLimiter;
use kit_server::sync::lock;
use lettre::message::Mailbox;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use crate::busy::{Busy, BusyCache};
use crate::config::{Config, EventType, SECRETS};
use crate::error::{ApiError, SLOT_TAKEN};
use crate::google::{Calendar, Event, Inserted, event_id};
use crate::grid::{Grid, Rules};
use crate::ics::{Invite, Method, Party};
use crate::mail::{Letter, Mailer};
use crate::routes::Valid;
use crate::security::{DailyCap, Keys};
use crate::stripe::Payments;

/// Why the service could not start, or stopped.
pub type Failure = Box<dyn Error + Send + Sync>;

#[cfg(test)]
type TestResult = Result<(), Box<dyn Error>>;

/// Ids a slot may take after earlier bookings of it were cancelled; one past this is refused.
const GENERATIONS: u32 = 64;

pub(crate) struct App {
    config: Config,
    grid: Grid,
    /// How far before and after a read the busy intervals reach: every type's buffer, horizon
    /// and length, so one read serves them all.
    reach: (SignedDuration, SignedDuration),
    calendar: Calendar,
    mailer: Mailer,
    stripe: Payments,
    keys: Keys,
    limiter: RateLimiter<IpAddr>,
    cap: DailyCap,
    /// The busy intervals over every type's window, as last read.
    busy: Mutex<BusyCache>,
    /// How long a read of them is reused.
    busy_lifetime: Duration,
    /// Bookings and cancellations one at a time; with reserved concurrency 1 that is all of them.
    booking: tokio::sync::Mutex<()>,
}

impl App {
    fn new(config: Config, http: &reqwest::Client) -> Result<Self, String> {
        let hour = Duration::from_hours(1);
        let rules = config.event_types.iter().map(Rules::of);
        let reach = rules.fold(
            (SignedDuration::ZERO, SignedDuration::ZERO),
            |(back, ahead), rules| {
                let after = rules
                    .horizon
                    .saturating_add(rules.duration)
                    .saturating_add(rules.buffer);
                (back.max(rules.buffer), ahead.max(after))
            },
        );
        Ok(Self {
            grid: Grid::new(&config.booking)?,
            reach,
            calendar: Calendar::new(&config.google, &config.secrets, http.clone()),
            mailer: Mailer::new(&config.email, &config.secrets)?,
            stripe: Payments::new(
                http.clone(),
                &config.stripe_api_base,
                config.secrets.stripe_secret_key.clone(),
            ),
            keys: Keys::new(config.secrets.server_secret.expose_secret())?,
            limiter: RateLimiter::new(config.rate_limits.bookings_per_ip_per_hour, hour),
            cap: DailyCap::new(config.rate_limits.daily_send_cap),
            busy: Mutex::new(BusyCache::default()),
            busy_lifetime: Duration::from_secs(config.limits.availability_cache_seconds),
            booking: tokio::sync::Mutex::new(()),
            config,
        })
    }

    /// Spends one of the address's booking requests this hour, refused once they are spent.
    pub(crate) fn admit(&self, ip: IpAddr) -> Result<(), ApiError> {
        self.limiter.check(&ip).map_err(|wait| {
            let detail = "Too many booking requests from this address. Try again later.";
            ApiError::status(StatusCode::TOO_MANY_REQUESTS, detail).after(wait)
        })
    }

    pub(crate) fn kind(&self, slug: &str) -> Option<&EventType> {
        self.config
            .event_types
            .iter()
            .find(|kind| kind.slug == slug)
    }

    /// The title of an event's meeting type; an event booked before types were named has the
    /// first type's.
    fn summary(&self, event: &Event) -> &str {
        let named = event.private("type").and_then(|slug| self.kind(slug));
        named
            .or_else(|| self.config.event_types.first())
            .map_or("", |kind| kind.summary.as_str())
    }

    /// The starts of a meeting of `kind` no busy interval takes. One calendar read covers every
    /// type's window a cache lifetime further ahead, so it serves every read in that lifetime.
    async fn free_starts(
        &self,
        now: Timestamp,
        kind: &EventType,
    ) -> Result<Vec<Timestamp>, ApiError> {
        let out_of_range = |_| ApiError::upstream("The booking window is out of range.");
        let (back, ahead) = self.reach;
        let lifetime = SignedDuration::try_from(self.busy_lifetime).map_err(out_of_range)?;
        let from = now.checked_sub(back).map_err(out_of_range)?;
        let until = now.checked_add(ahead).map_err(out_of_range)?;
        let started = Instant::now();
        let cached = lock(&self.busy).fresh(started, self.busy_lifetime, (from, until));
        let busy = if let Some(busy) = cached {
            busy
        } else {
            let ticket = lock(&self.busy).ticket(started);
            let until = until.checked_add(lifetime).map_err(out_of_range)?;
            let busy: Busy = Arc::new(self.calendar.busy(from, until).await?);
            lock(&self.busy).store(ticket, (from, until), Arc::clone(&busy));
            busy
        };
        let rules = Rules::of(kind);
        Ok(grid::free(self.grid.starts(now, rules), &busy, rules))
    }

    /// The calendar changed here: the next availability reads it again.
    pub(crate) fn forget_busy(&self) {
        lock(&self.busy).forget();
    }

    /// Re-checks the slot, inserts its event, and sends the invitation; if the relay refuses it,
    /// the event is deleted again and nothing is booked.
    async fn book(&self, valid: &Valid, key: &str, holder: &str) -> Result<Event, ApiError> {
        self.clear(valid, Timestamp::now()).await?;
        let (event, replayed) = self.insert(valid, key, holder, None).await?;
        if replayed {
            return Ok(event);
        }
        self.forget_busy();
        if let Err(err) = self.invite(Method::Request, &event, &[]).await {
            tracing::error!(error = %err, "the relay refused the invitation; deleting the event");
            if let Err(err) = self.calendar.delete(&event.id).await {
                tracing::error!(error = ?err, id = event.id, "the unsent booking could not be deleted");
            }
            return Err(ApiError::upstream(
                "The invitation could not be sent, so nothing was booked. Please try again.",
            ));
        }
        self.cap.record(Timestamp::now());
        Ok(event)
    }

    /// Inserts under the slot's first free generation; Google refusing an id means another insert
    /// won: a cancelled event moves on, this request's own is a replay, any other holds the slot.
    async fn insert(
        &self,
        valid: &Valid,
        key: &str,
        holder: &str,
        hold: Option<Timestamp>,
    ) -> Result<(Event, bool), ApiError> {
        for generation in 0..GENERATIONS {
            let id = event_id(valid.start, generation);
            let event = hold.map_or_else(
                || Self::event(&id, generation, valid, key, holder),
                |until| Self::held(&id, valid, (key, holder), until),
            );
            match self.calendar.insert(&event).await? {
                Inserted::Created(event) if hold.is_some() => return Ok((*event, false)),
                Inserted::Created(event) => return Ok((self.with_video(*event).await?, false)),
                Inserted::Exists => match self.calendar.get(&id).await? {
                    Some(event) if event.cancelled() => {}
                    Some(event) if event.private("idem") == Some(key) => return Ok((event, true)),
                    _ => return Err(taken()),
                },
            }
        }
        Err(taken())
    }

    fn event(id: &str, generation: u32, valid: &Valid, key: &str, holder: &str) -> Value {
        let end = valid
            .start
            .checked_add(Rules::of(&valid.kind).duration)
            .unwrap_or(valid.start);
        let (name, email) = (&valid.name, valid.email.to_string());
        let note = if valid.message.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", valid.message)
        };
        // Google ignores a conference request whose id it has seen, so each generation's differs.
        let request_id = if generation == 0 {
            key.to_owned()
        } else {
            format!("{key}-{generation}")
        };
        let mut event = json!({
            "id": id,
            "summary": valid.kind.summary,
            "description": format!("Booked on marcoferra.com by {name} <{email}>.{note}"),
            "start": { "dateTime": valid.start, "timeZone": "UTC" },
            "end": { "dateTime": end, "timeZone": "UTC" },
            "attendees": [{ "email": email, "displayName": name }],
            "guestsCanInviteOthers": false,
            "guestsCanSeeOtherGuests": false,
            "conferenceData": { "createRequest": {
                "requestId": request_id,
                "conferenceSolutionKey": { "type": "hangoutsMeet" },
            } },
            "extendedProperties": { "private": {
                "idem": key, "mfEmail": holder, "tz": valid.zone, "type": valid.kind.slug,
            } },
        });
        Self::with_source(&mut event, valid);
        event
    }

    /// A time held for a payment: no guest, no video call and no invitation until it is paid;
    /// the visitor's details wait in its private properties, the message in its description.
    fn held(id: &str, valid: &Valid, (key, holder): (&str, &str), until: Timestamp) -> Value {
        let end = valid
            .start
            .checked_add(Rules::of(&valid.kind).duration)
            .unwrap_or(valid.start);
        let (name, email) = (&valid.name, valid.email.to_string());
        let note = if valid.message.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", valid.message)
        };
        let mut event = json!({
            "id": id,
            "summary": format!("Awaiting payment: {}", valid.kind.title),
            "description": format!("Held on marcoferra.com for {name} <{email}> until paid.{note}"),
            "start": { "dateTime": valid.start, "timeZone": "UTC" },
            "end": { "dateTime": end, "timeZone": "UTC" },
            "guestsCanInviteOthers": false,
            "guestsCanSeeOtherGuests": false,
            "extendedProperties": { "private": {
                "idem": key, "mfEmail": holder, "tz": valid.zone, "type": valid.kind.slug,
                "mfState": "held", "holdUntil": until.as_second().to_string(),
                "bookedAt": Timestamp::now().as_second().to_string(),
                "name": name, "email": email, "waiver": valid.waiver.to_string(),
            } },
        });
        Self::with_source(&mut event, valid);
        event
    }

    /// Notes where the visitor's link was shared, when it says; paying keeps it.
    fn with_source(event: &mut Value, valid: &Valid) {
        let private = event.pointer_mut("/extendedProperties/private");
        if let (Some(source), Some(Value::Object(private))) = (&valid.source, private) {
            private.insert("source".to_owned(), Value::String(source.clone()));
        }
    }

    /// Google may add the Meet link just after the insert answers; one read picks it up.
    async fn with_video(&self, event: Event) -> Result<Event, ApiError> {
        if event.hangout_link.is_some() {
            return Ok(event);
        }
        Ok(self.calendar.get(&event.id).await?.unwrap_or(event))
    }

    /// Cancels and sends the cancellation, with `extra` paragraphs; if the relay refuses it, the
    /// event is restored and the meeting stands.
    async fn cancel(&self, event: &Event, extra: &[String]) -> Result<(), ApiError> {
        let cancelled = self.calendar.set_status(&event.id, "cancelled").await?;
        self.forget_busy();
        if let Err(err) = self.invite(Method::Cancel, &cancelled, extra).await {
            tracing::error!(error = %err, "the relay refused the cancellation; restoring the event");
            if let Err(err) = self.calendar.set_status(&event.id, "confirmed").await {
                tracing::error!(error = ?err, id = event.id, "the cancelled event could not be restored");
            }
            return Err(ApiError::upstream(
                "The cancellation could not be sent, so the meeting stands. Please try again.",
            ));
        }
        self.cap.record(Timestamp::now());
        Ok(())
    }

    /// The invitation as iCalendar; a request is stamped with the event's creation, so the
    /// download rebuilds the very bytes the email carried.
    fn ics(&self, method: Method, event: &Event) -> Option<String> {
        let attendee = event.attendees.first()?;
        let (sequence, stamp) = match method {
            Method::Request => (event.sequence, event.created),
            Method::Cancel => (event.sequence.saturating_add(1), event.updated),
        };
        let video = event.hangout_link.as_deref();
        let joining = video
            .map(|link| format!("Video call: {link}\n"))
            .unwrap_or_default();
        let description = format!(
            "{joining}To cancel: {}{}",
            self.config.public_base_url,
            self.cancel_path(event)
        );
        Some(ics::build(&Invite {
            method,
            uid: &event.ical_uid,
            sequence,
            stamp,
            start: event.start.date_time,
            end: event.end.date_time,
            summary: self.summary(event),
            description: &description,
            video,
            organizer: Party {
                name: self.mailer.from.name.as_deref().unwrap_or_default(),
                email: &self.config.email.organizer,
            },
            attendee: Party {
                name: &attendee.display_name,
                email: &attendee.email,
            },
        }))
    }

    /// The invitation or the cancellation, with `extra` paragraphs before the way to cancel.
    async fn invite(&self, method: Method, event: &Event, extra: &[String]) -> Result<(), String> {
        let attendee = event.attendees.first().ok_or("the event has no guest")?;
        let ics = self.ics(method, event).ok_or("the event has no guest")?;
        let (when, summary) = (when(event), self.summary(event));
        let base = &self.config.public_base_url;
        let mut lines = match method {
            Method::Request => vec![
                format!("Your meeting with Marco Ferra is booked for {when}."),
                event
                    .hangout_link
                    .as_ref()
                    .map(|link| format!("Join the video call: {link}"))
                    .unwrap_or_default(),
                "The invitation is attached: open it to add the meeting to your calendar."
                    .to_owned(),
            ],
            Method::Cancel => vec![format!(
                "Your meeting with Marco Ferra on {when} is cancelled."
            )],
        };
        lines.extend_from_slice(extra);
        lines.push(match method {
            Method::Request => format!("Need to cancel? {base}{}", self.cancel_path(event)),
            Method::Cancel => format!("To choose another time: {base}/book/"),
        });
        let label = if method == Method::Request {
            "Invitation"
        } else {
            "Cancelled"
        };
        let letter = Plain {
            name: &attendee.display_name,
            email: &attendee.email,
            subject: &format!("{label}: {summary}, {when}"),
            lines: &lines,
        };
        self.send(&letter, Some((&ics, method))).await
    }

    /// A letter from Marco Ferra, as text and HTML, with the invitation when there is one.
    async fn send(
        &self,
        plain: &Plain<'_>,
        calendar: Option<(&str, Method)>,
    ) -> Result<(), String> {
        let address = plain
            .email
            .parse()
            .map_err(|err| format!("the guest's address: {err}"))?;
        let base = &self.config.public_base_url;
        let lines: Vec<&String> = plain.lines.iter().filter(|line| !line.is_empty()).collect();
        let greeting = format!("Hello {},", plain.name);
        let joined: Vec<&str> = lines.iter().map(|line| line.as_str()).collect();
        let text = format!(
            "{greeting}\n\n{}\n\nMarco Ferra\n{base}/\n",
            joined.join("\n\n")
        );
        let paragraphs = lines.iter().fold(String::new(), |mut html, line| {
            html.push_str("<p>");
            html.push_str(&escape(line));
            html.push_str("</p>");
            html
        });
        let html = format!(
            "<!doctype html><html lang=\"en\"><body><p>{}</p>{paragraphs}<p>Marco Ferra</p></body></html>",
            escape(&greeting)
        );
        let to = Mailbox::new(Some(plain.name.to_owned()), address);
        let letter = Letter {
            to,
            subject: plain.subject,
            text: &text,
            html: &html,
            calendar,
        };
        self.mailer.send(&letter).await
    }
}

pub(crate) struct Plain<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub subject: &'a str,
    pub lines: &'a [String],
}

/// The meeting's time in the visitor's own zone, as the email states it.
pub(crate) fn when(event: &Event) -> String {
    let name = event.private("tz").unwrap_or("UTC");
    let zone = jiff::tz::db().get(name).unwrap_or(TimeZone::UTC);
    let start = event
        .start
        .date_time
        .to_zoned(zone.clone())
        .strftime("%A %-d %B %Y, %H:%M");
    let end = event.end.date_time.to_zoned(zone).strftime("%H:%M");
    format!("{start} to {end} ({name})")
}

fn taken() -> ApiError {
    ApiError::refused(
        &SLOT_TAKEN,
        "That time was booked a moment ago. Please choose another.",
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The commit this binary was built from, as the build's `BUILD_SHA` gave it.
const VERSION: &str = match option_env!("BUILD_SHA") {
    Some(sha) => sha,
    None => "dev",
};

/// Liveness, and which build answers: a deploy's smoke test compares the version.
async fn healthz() -> axum::Json<Value> {
    axum::Json(json!({ "status": "ok", "version": VERSION }))
}

/// Stripe's events can outgrow a booking form's limit.
const WEBHOOK_BODY_MAX_BYTES: usize = 256 * 1024;

/// The API's routes, as the edge hands them over: without the public `/api` prefix. A booking's
/// custom methods share its one path segment, `{id}:verb`, which the router cannot split.
fn router(app: Arc<App>) -> Router {
    let body_limit = app.config.limits.body_max_bytes;
    Router::new()
        .route("/v1/availability", get(routes::availability))
        .route("/v1/bookings", post(routes::create))
        .route("/v1/bookings:confirm", post(routes::confirm))
        .route("/v1/bookings/{name}", get(routes::get).post(routes::act))
        .route("/bookings/{id}/ics", get(routes::moved))
        .route(
            "/stripe/webhook",
            post(routes::webhook).layer(DefaultBodyLimit::max(WEBHOOK_BODY_MAX_BYTES)),
        )
        .route("/healthz", get(healthz))
        .fallback(|| async {
            ApiError::status(StatusCode::NOT_FOUND, "There is no such resource.")
        })
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(app)
}

/// Logs to standard output, where `CloudWatch` or the terminal reads them; `RUST_LOG` filters.
pub fn logs() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .init();
}

/// How long requests in flight at a stop have to finish: a booking's calendar and mail calls
/// fit well within it. Past it they are cut off, so a stop always ends.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

/// Serves on `BIND_ADDR`, secrets from the environment, until SIGINT or SIGTERM, then drains.
pub async fn serve() -> Result<(), Failure> {
    let config = Config::load(&|name| var(name))?;
    // Armed before anything says ready, so a stop sent from then on drains, never kills.
    let signal = kit_server::shutdown::stop_signal();
    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    // Dropping `stop` begins the graceful shutdown.
    let (stop, stopping) = tokio::sync::oneshot::channel::<()>();
    let app = Arc::new(App::new(config, &http_client()?)?);
    let serving = axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            drop(stopping.await);
        })
        .into_future();
    tokio::pin!(serving);
    tokio::select! {
        served = &mut serving => return served.map_err(Into::into),
        () = signal => {}
    }
    tracing::info!("stopping");
    drop(stop);
    if let Ok(served) = tokio::time::timeout(SHUTDOWN_GRACE, serving).await {
        served?;
        tracing::info!("stopped: in-flight requests finished");
    } else {
        tracing::warn!("stopped: requests still in flight at the deadline were cut off");
    }
    Ok(())
}

/// The router Lambda serves, its secrets read once from SSM; `CloudFront` keeps `/api`.
pub async fn lambda_router() -> Result<Router, Failure> {
    let started = Instant::now();
    let secrets = parameters().await?;
    let secrets_ms = started.elapsed().as_millis();
    let config = Config::load(&|name| Ok(secrets.get(name).cloned()))?;
    let app = Arc::new(App::new(config, &http_client()?)?);
    let warmed = prefetch(&app).await;
    let ready_ms = started.elapsed().as_millis();
    tracing::info!(secrets_ms, ready_ms, warmed, "initialised");
    Ok(Router::new()
        .nest("/api", router(app))
        .layer(axum::middleware::from_fn(alias_gate)))
}

/// The init phase's budget for the first calendar read; past it, the first request reads.
const PREFETCH_BUDGET: Duration = Duration::from_secs(3);

/// Reads the calendar while the instance starts, when Lambda gives it more CPU than at 128 MB
/// later: a cold first request then takes about 0.8 s, not 1.5 s.
async fn prefetch(app: &App) -> bool {
    let Some(kind) = app.config.event_types.first() else {
        return false;
    };
    let read = app.free_starts(Timestamp::now(), kind);
    matches!(tokio::time::timeout(PREFETCH_BUDGET, read).await, Ok(Ok(_)))
}

/// The one HTTP client of the instance's life: its connections to Google and Stripe stay open
/// between requests, and its trusted roots are loaded once.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .build()
        .map_err(|err| err.to_string())
}

/// The alias whose public function URL takes Stripe's webhook and nothing else.
const STRIPE_ALIAS: &str = "stripe";
const WEBHOOK_PATH: &str = "/api/stripe/webhook";

/// Each alias serves its own routes: `stripe` the webhook alone, every other the rest.
async fn alias_gate(request: Request, next: Next) -> Response {
    let arn = request
        .extensions()
        .get::<lambda_http::Context>()
        .map(|context| context.invoked_function_arn.as_str());
    if admitted(qualifier(arn), request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    ApiError::status(StatusCode::NOT_FOUND, "There is no such resource.").into_response()
}

/// The alias or version of `arn:aws:lambda:<region>:<account>:function:<name>[:<qualifier>]`.
fn qualifier(arn: Option<&str>) -> Option<&str> {
    arn?.split(':').nth(7)
}

fn admitted(qualifier: Option<&str>, method: &HttpMethod, path: &str) -> bool {
    let webhook = method == HttpMethod::POST && path == WEBHOOK_PATH;
    if qualifier == Some(STRIPE_ALIAS) {
        webhook
    } else {
        path != WEBHOOK_PATH
    }
}

async fn parameters() -> Result<HashMap<String, String>, Failure> {
    let prefix = var("SSM_PREFIX")?.unwrap_or_else(|| "/marcoferra/booking".to_owned());
    let port = var("PARAMETERS_SECRETS_EXTENSION_HTTP_PORT")?.unwrap_or_else(|| "2773".to_owned());
    let token = var("AWS_SESSION_TOKEN")?.ok_or("AWS_SESSION_TOKEN is unset")?;
    fetch_parameters(&format!("http://localhost:{port}"), &prefix, &token).await
}

/// Every secret under `prefix` from the extension at `base`, all asked at once, so a cold start
/// waits for the slowest reply rather than the sum of them.
async fn fetch_parameters(
    base: &str,
    prefix: &str,
    token: &str,
) -> Result<HashMap<String, String>, Failure> {
    let http = reqwest::Client::new();
    let mut fetches = tokio::task::JoinSet::new();
    for name in SECRETS {
        let parameter = google::url_segment(&format!("{prefix}/{}", name.to_ascii_lowercase()));
        let request = http
            .get(format!(
                "{base}/systemsmanager/parameters/get?name={parameter}&withDecryption=true"
            ))
            .header("X-Aws-Parameters-Secrets-Token", token);
        let missing = format!("{prefix}: {name} has no value");
        fetches.spawn(async move {
            let reply: Value = request.send().await?.error_for_status()?.json().await?;
            let value = reply.pointer("/Parameter/Value").and_then(Value::as_str);
            Ok::<_, Failure>((name.to_owned(), value.ok_or(missing)?.to_owned()))
        });
    }
    let mut found = HashMap::new();
    while let Some(fetched) = fetches.join_next().await {
        let (name, value) = fetched??;
        found.insert(name, value);
    }
    Ok(found)
}

/// The API contract, as `server/openapi.yaml` holds it.
/// # Errors
/// A message, when the contract cannot be serialised.
pub fn openapi_yaml() -> Result<String, String> {
    kit_server::openapi::yaml(routes::ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use axum::Router;
    use axum::extract::Query;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use crate::TestResult;
    use crate::config::SECRETS;

    #[test]
    fn a_booking_notes_its_source_only_when_given() -> TestResult {
        let file: Value = serde_json::from_str(crate::config::COMMITTED)?;
        let kind: crate::config::EventType =
            serde_json::from_value(file.pointer("/event_types/1").cloned().ok_or("no type")?)?;
        let mut valid = crate::routes::Valid {
            kind,
            start: "2026-01-06T10:00:00Z".parse()?,
            name: "Ana".to_owned(),
            email: "ana@example.com".parse()?,
            message: String::new(),
            zone: "Europe/Lisbon".to_owned(),
            waiver: true,
            source: Some("reddit".to_owned()),
        };
        let until = "2026-01-05T10:31:00Z".parse()?;
        let source = "/extendedProperties/private/source";
        let held = super::App::held("mfabc", &valid, ("key", "holder"), until);
        let booked = super::App::event("mfabc", 0, &valid, "key", "holder");
        assert_eq!(held.pointer(source), Some(&json!("reddit")));
        assert_eq!(booked.pointer(source), Some(&json!("reddit")));
        valid.source = None;
        let unnamed = super::App::held("mfabc", &valid, ("key", "holder"), until);
        assert_eq!(unnamed.pointer(source), None);
        Ok(())
    }

    /// Such as a Role Conversation's brief, as its own paragraph. The invitation never repeats it.
    #[test]
    fn the_message_goes_to_the_hosts_calendar_as_written() -> TestResult {
        let file: Value = serde_json::from_str(crate::config::COMMITTED)?;
        let kind: crate::config::EventType =
            serde_json::from_value(file.pointer("/event_types/0").cloned().ok_or("no type")?)?;
        let brief = "Company: Acme\nRole title: Head of Delivery\n\nRemote first.";
        let mut valid = crate::routes::Valid {
            kind,
            start: "2026-01-06T10:00:00Z".parse()?,
            name: "Ana".to_owned(),
            email: "ana@example.com".parse()?,
            message: brief.to_owned(),
            zone: "Europe/Lisbon".to_owned(),
            waiver: false,
            source: None,
        };
        let booked = super::App::event("mfabc", 0, &valid, "key", "holder");
        let expected = format!("Booked on marcoferra.com by Ana <ana@example.com>.\n\n{brief}");
        assert_eq!(booked.pointer("/description"), Some(&json!(expected)));
        valid.message = String::new();
        let silent = super::App::event("mfabc", 0, &valid, "key", "holder");
        assert_eq!(
            silent.pointer("/description"),
            Some(&json!("Booked on marcoferra.com by Ana <ana@example.com>."))
        );
        Ok(())
    }

    #[test]
    fn the_stripe_alias_serves_the_webhook_and_nothing_else() {
        use axum::http::Method;
        let arn = "arn:aws:lambda:us-east-1:123456789012:function:marcoferra-booking";
        let (stripe, live) = (format!("{arn}:stripe"), format!("{arn}:live"));
        let cases = [
            (
                Some(stripe.as_str()),
                Method::POST,
                "/api/stripe/webhook",
                true,
            ),
            (
                Some(stripe.as_str()),
                Method::GET,
                "/api/stripe/webhook",
                false,
            ),
            (
                Some(stripe.as_str()),
                Method::POST,
                "/api/v1/bookings",
                false,
            ),
            (Some(stripe.as_str()), Method::GET, "/api/healthz", false),
            (Some(live.as_str()), Method::POST, "/api/v1/bookings", true),
            (
                Some(live.as_str()),
                Method::POST,
                "/api/stripe/webhook",
                false,
            ),
            (Some(arn), Method::GET, "/api/healthz", true),
            (None, Method::GET, "/api/v1/availability", true),
        ];
        for (arn, method, path, served) in cases {
            let named = super::qualifier(arn);
            assert_eq!(
                super::admitted(named, &method, path),
                served,
                "{arn:?} {method} {path}"
            );
        }
    }

    #[test]
    fn the_contract_is_openapi_3_2_at_versioned_paths() -> TestResult {
        let yaml = super::openapi_yaml()?;
        assert!(yaml.starts_with("openapi: 3.2.0\n"));
        assert!(yaml.contains("\n  /v1/bookings/{id}:cancel:\n"));
        Ok(())
    }

    const ANSWER_MS: u64 = 300;

    /// The Parameters and Secrets extension, answering each parameter by its name after a pause.
    async fn extension(
        query: Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> axum::Json<Value> {
        tokio::time::sleep(Duration::from_millis(ANSWER_MS)).await;
        let token = headers.get("X-Aws-Parameters-Secrets-Token");
        let name = query.get("name").cloned().unwrap_or_default();
        let value = if token.is_some_and(|token| token == "token") {
            json!(format!("value of {name}"))
        } else {
            Value::Null
        };
        axum::Json(json!({ "Parameter": { "Value": value } }))
    }

    #[tokio::test]
    async fn the_secrets_are_read_at_once_each_under_its_own_name() -> TestResult {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let app = Router::new().route("/systemsmanager/parameters/get", get(extension));
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let started = Instant::now();
        let found = super::fetch_parameters(&base, "/marcoferra/booking", "token").await;
        let took = started.elapsed();
        let refused = super::fetch_parameters(&base, "/marcoferra/booking", "wrong").await;
        server.abort();
        let found = found.map_err(|err| err.to_string())?;
        assert_eq!(found.len(), SECRETS.len());
        for name in SECRETS {
            let expected = format!("value of /marcoferra/booking/{}", name.to_ascii_lowercase());
            assert_eq!(found.get(name), Some(&expected));
        }
        let count = u64::try_from(SECRETS.len())?;
        let sequential = Duration::from_millis(ANSWER_MS.saturating_mul(count));
        assert!(took < sequential / 2, "{took:?}");
        assert!(refused.is_err());
        Ok(())
    }
}
