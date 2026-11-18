//! Runtime configuration: `marcoferra.config.json`, compiled in or replaced by the file
//! `MARCOFERRA_CONFIG` names, then overridden; secrets from the environment, or SSM on Lambda.

use kit_server::config::{Override, Settings, var};
use kit_server::stripe;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// The committed configuration, so a Lambda deployment carries it inside its one binary.
pub(crate) const COMMITTED: &str = include_str!("../../marcoferra.config.json");

const OVERRIDES: [Override; 9] = [
    ("BIND_ADDR", "/network/bind_addr"),
    ("CLIENT_IP_HEADER", "/network/client_ip_header"),
    ("PUBLIC_BASE_URL", "/domains/public_base_url"),
    ("MIN_FILL_SECONDS", "/limits/min_fill_seconds"),
    (
        "BOOKINGS_PER_IP_PER_HOUR",
        "/rate_limits/bookings_per_ip_per_hour",
    ),
    ("DAILY_SEND_CAP", "/rate_limits/daily_send_cap"),
    ("SMTP_HOST", "/email/host"),
    ("SMTP_PORT", "/email/port"),
    ("GOOGLE_CALENDAR_ID", "/google/calendar_id"),
];

/// By their environment names; SSM holds them in lower case.
pub(crate) const SECRETS: [&str; 8] = [
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "GOOGLE_REFRESH_TOKEN",
    "SMTP_USERNAME",
    "SMTP_PASSWORD",
    "SERVER_SECRET",
    "STRIPE_SECRET_KEY",
    "STRIPE_WEBHOOK_SECRET",
];

/// Shortest `SERVER_SECRET` accepted: 256 bits written as hex or base64.
const MIN_SECRET_CHARS: usize = 32;

pub(crate) struct Config {
    pub bind_addr: String,
    pub client_ip_header: String,
    pub public_base_url: String,
    pub booking: Booking,
    pub event_types: Vec<EventType>,
    pub limits: Limits,
    pub rate_limits: RateLimits,
    pub email: Email,
    pub google: Google,
    /// Stripe's API, replaced by a fake in tests only.
    pub stripe_api_base: String,
    pub secrets: Secrets,
}

#[derive(Deserialize)]
struct File {
    booking: Booking,
    event_types: Vec<EventType>,
    limits: Limits,
    rate_limits: RateLimits,
    email: Email,
    google: Google,
}

/// The host's grid: ISO weekdays, the first start and the hour every meeting ends by, in the
/// host's wall-clock time.
#[derive(Deserialize)]
pub(crate) struct Booking {
    pub host_time_zone: String,
    pub weekdays: Vec<i8>,
    pub first_start_hour: i8,
    pub last_end_hour: i8,
    pub start_every_minutes: i64,
    /// How long a paid booking's time is held for its payment; Stripe's Checkout lasts at least
    /// 30 minutes, so a shorter hold is refused.
    pub hold_minutes: u32,
    /// A paid booking cancelled at least this long before its start is refunded in full.
    pub refund_until_hours: u32,
    /// A paid booking cancelled within this many days of being booked, before it starts, is
    /// refunded in full: the consumer's right of withdrawal, 14 days at least.
    pub withdrawal_days: u32,
}

/// The withdrawal period the law sets at least, and a sane longest.
const WITHDRAWAL_RANGE: std::ops::RangeInclusive<u32> = 14..=90;

/// Stripe Checkout's shortest and longest life, in minutes.
const HOLD_RANGE: std::ops::RangeInclusive<u32> = 30..=1440;

/// A kind of meeting a visitor books, as `marcoferra.config.json` lists it under `event_types`.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventType {
    /// The page's path segment, `/book/<slug>/`: 1 to 40 lowercase letters, digits or hyphens,
    /// and not `cancel`.
    pub slug: String,
    pub title: String,
    pub description: String,
    /// The calendar event's and the invitation's title.
    pub summary: String,
    /// A positive multiple of 5.
    pub duration_minutes: u32,
    /// Kept free before and after the meeting.
    pub buffer_minutes: u32,
    /// The shortest time between booking and the meeting's start.
    pub notice_hours: u32,
    /// How far ahead a start may be booked.
    pub horizon_days: u32,
    /// The price in the currency's minor unit: 0 is free, and a paid type costs at least 50.
    pub price_minor: i64,
    /// ISO 4217, in lower case.
    pub currency: String,
    /// How a paid type is paid. Empty for a free one.
    pub payment_methods: Vec<PaymentMethod>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PaymentMethod {
    Card,
    MbWay,
    Paypal,
}

impl PaymentMethod {
    /// Stripe's name for it, as `payment_method_types` lists it.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::MbWay => "mb_way",
            Self::Paypal => "paypal",
        }
    }
}

/// The longest slug, so a page path stays short.
const SLUG_MAX_CHARS: usize = 40;
/// A slug the site's own pages under `/book/` already take.
const RESERVED_SLUGS: [&str; 1] = ["cancel"];
/// Stripe's smallest charge in euros, in cents.
const MIN_PRICE_MINOR: i64 = 50;

/// What `Config::load` refuses in `event_types`, each with the entry it names.
fn check_event_types(types: &[EventType]) -> Result<(), String> {
    if types.is_empty() {
        return Err("event_types lists no meeting type".to_owned());
    }
    let mut seen = std::collections::HashSet::new();
    for kind in types {
        let slug = &kind.slug;
        let fail = |rule: &str| Err(format!("event_types {slug:?}: {rule}"));
        let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if slug.is_empty() || slug.len() > SLUG_MAX_CHARS || !slug.chars().all(allowed) {
            return fail("the slug needs 1 to 40 lowercase letters, digits or hyphens");
        }
        if RESERVED_SLUGS.contains(&slug.as_str()) {
            return fail("the slug names one of the site's own pages");
        }
        if !seen.insert(slug) {
            return fail("the slug is used twice");
        }
        if [&kind.title, &kind.description, &kind.summary]
            .iter()
            .any(|text| text.trim().is_empty())
        {
            return fail("the title, description and summary need text");
        }
        if kind.duration_minutes == 0 || kind.duration_minutes % 5 != 0 {
            return fail("duration_minutes must be a positive multiple of 5");
        }
        if kind.price_minor < 0 {
            return fail("price_minor cannot be negative");
        }
        if kind.price_minor > 0 && kind.price_minor < MIN_PRICE_MINOR {
            return fail("a paid type costs at least 50 minor units");
        }
        if kind.price_minor > 0 && kind.payment_methods.is_empty() {
            return fail("a paid type needs payment_methods");
        }
        if kind.payment_methods.contains(&PaymentMethod::MbWay) && kind.currency != "eur" {
            return fail("MB WAY takes euros only");
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub(crate) struct Limits {
    pub name_max_chars: usize,
    pub email_max_chars: usize,
    pub message_max_chars: usize,
    pub body_max_bytes: usize,
    pub min_fill_seconds: i64,
    pub form_token_max_age_minutes: i64,
    pub availability_cache_seconds: u64,
}

#[derive(Deserialize)]
pub(crate) struct RateLimits {
    pub bookings_per_ip_per_hour: u32,
    pub daily_send_cap: u32,
}

/// The SMTP relay (STARTTLS unless port 465), the sender, and the calendar's organiser.
#[derive(Deserialize)]
pub(crate) struct Email {
    pub host: String,
    pub port: u16,
    pub from: String,
    pub organizer: String,
}

#[derive(Deserialize)]
pub(crate) struct Google {
    pub calendar_id: String,
    /// Replaced by a fake in tests only.
    #[serde(default = "google_api")]
    pub api_base: String,
    #[serde(default = "google_token")]
    pub token_url: String,
}

fn google_api() -> String {
    "https://www.googleapis.com".to_owned()
}

fn google_token() -> String {
    "https://oauth2.googleapis.com/token".to_owned()
}

pub(crate) struct Secrets {
    pub google_client_id: SecretString,
    pub google_client_secret: SecretString,
    pub google_refresh_token: SecretString,
    pub smtp_username: SecretString,
    pub smtp_password: SecretString,
    pub server_secret: SecretString,
    pub stripe_secret_key: SecretString,
    pub stripe_webhook_secret: SecretString,
}

/// Where a secret comes from: `Ok(None)` when it is absent.
pub(crate) type SecretSource<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

impl Config {
    pub(crate) fn load(secret: SecretSource<'_>) -> Result<Self, String> {
        let settings = match var("MARCOFERRA_CONFIG")? {
            Some(path) => Settings::read(&path)?,
            None => Settings::parse(COMMITTED, "marcoferra.config.json")?,
        };
        let settings = settings.overridden(&OVERRIDES, var)?;
        let origin = &settings.origin;
        let mut file: File = settings.file()?;
        check_event_types(&file.event_types).map_err(|err| format!("{origin}: {err}"))?;
        if !HOLD_RANGE.contains(&file.booking.hold_minutes) {
            return Err(format!("{origin}: booking.hold_minutes must be 30 to 1440"));
        }
        if !WITHDRAWAL_RANGE.contains(&file.booking.withdrawal_days) {
            return Err(format!(
                "{origin}: booking.withdrawal_days must be 14 to 90"
            ));
        }
        if let Some(base) = var("GOOGLE_API_BASE")? {
            file.google.api_base = base;
        }
        if let Some(url) = var("GOOGLE_TOKEN_URL")? {
            file.google.token_url = url;
        }
        let required = |name: &str| {
            let found = secret(name)?.filter(|value| !value.is_empty());
            found
                .map(SecretString::from)
                .ok_or_else(|| format!("the secret {name} is missing"))
        };
        let secrets = Secrets {
            google_client_id: required("GOOGLE_CLIENT_ID")?,
            google_client_secret: required("GOOGLE_CLIENT_SECRET")?,
            google_refresh_token: required("GOOGLE_REFRESH_TOKEN")?,
            smtp_username: required("SMTP_USERNAME")?,
            smtp_password: required("SMTP_PASSWORD")?,
            server_secret: required("SERVER_SECRET")?,
            stripe_secret_key: required("STRIPE_SECRET_KEY")?,
            stripe_webhook_secret: required("STRIPE_WEBHOOK_SECRET")?,
        };
        if secrets.server_secret.expose_secret().chars().count() < MIN_SECRET_CHARS {
            return Err(format!(
                "SERVER_SECRET needs at least {MIN_SECRET_CHARS} characters"
            ));
        }
        let client_ip_header: String = settings.get("/network/client_ip_header")?;
        let public_base_url: String = settings.get("/domains/public_base_url")?;
        Ok(Self {
            bind_addr: settings.get("/network/bind_addr")?,
            client_ip_header: client_ip_header.to_ascii_lowercase(),
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            booking: file.booking,
            event_types: file.event_types,
            limits: file.limits,
            rate_limits: file.rate_limits,
            email: file.email,
            google: file.google,
            stripe_api_base: var("STRIPE_API_BASE")?.unwrap_or_else(|| stripe::API_BASE.to_owned()),
            secrets,
        })
    }
}

#[cfg(test)]
mod tests {
    use kit_server::config::Settings;
    use serde_json::{Value, json};

    use super::{
        COMMITTED, File, HOLD_RANGE, OVERRIDES, PaymentMethod, WITHDRAWAL_RANGE, check_event_types,
    };
    use crate::TestResult;

    /// The committed file with its first meeting type changed by `change`, checked.
    fn with_type(change: impl FnOnce(&mut Value)) -> Result<(), String> {
        let mut tree: Value = serde_json::from_str(COMMITTED).map_err(|err| err.to_string())?;
        let first = tree
            .pointer_mut("/event_types/0")
            .ok_or("no meeting type")?;
        change(first);
        let file: File = serde_json::from_value(tree).map_err(|err| err.to_string())?;
        check_event_types(&file.event_types)
    }

    fn set(member: &'static str, value: Value) -> impl FnOnce(&mut Value) {
        move |kind| {
            if let Some(slot) = kind.get_mut(member) {
                *slot = value;
            }
        }
    }

    #[test]
    fn the_committed_meeting_types_pass() -> TestResult {
        with_type(|_| {})?;
        let paid = |kind: &mut Value| {
            set("price_minor", json!(5000))(kind);
            set("payment_methods", json!(["card", "mb_way", "paypal"]))(kind);
        };
        with_type(paid)?;
        Ok(())
    }

    #[test]
    fn each_meeting_type_rule_refuses_on_its_own() {
        let paid_with = |methods: Value, currency: &'static str| {
            move |kind: &mut Value| {
                set("price_minor", json!(5000))(kind);
                set("payment_methods", methods)(kind);
                set("currency", json!(currency))(kind);
            }
        };
        // Each refusal, and the words of its reason.
        let refusals = [
            (with_type(set("slug", json!("Intro"))), "the slug needs"),
            (with_type(set("slug", json!(""))), "the slug needs"),
            (
                with_type(set("slug", json!("a".repeat(41)))),
                "the slug needs",
            ),
            (with_type(set("slug", json!("cancel"))), "site's own pages"),
            (with_type(set("title", json!(" "))), "need text"),
            (
                with_type(set("duration_minutes", json!(0))),
                "multiple of 5",
            ),
            (
                with_type(set("duration_minutes", json!(32))),
                "multiple of 5",
            ),
            (
                with_type(set("price_minor", json!(-1))),
                "cannot be negative",
            ),
            (with_type(set("price_minor", json!(49))), "at least 50"),
            (
                with_type(paid_with(json!([]), "eur")),
                "needs payment_methods",
            ),
            (
                with_type(paid_with(json!(["cash"]), "eur")),
                "unknown variant",
            ),
            (with_type(paid_with(json!(["mb_way"]), "usd")), "euros only"),
            (check_event_types(&[]), "no meeting type"),
        ];
        for (refused, reason) in refusals {
            let err = refused.err().unwrap_or_default();
            assert!(err.contains(reason), "{reason}: {err}");
        }
        assert!(with_type(paid_with(json!(["card"]), "usd")).is_ok());
    }

    /// Led by the introduction the free journeys book. Every paid type takes card, MB WAY and
    /// `PayPal`, and its hold lasts as long as Checkout allows.
    #[test]
    fn the_committed_offer_is_the_free_types_then_the_paid_ones() -> TestResult {
        let file: File = serde_json::from_str(COMMITTED)?;
        let first = file.event_types.first().ok_or("no meeting type")?;
        assert_eq!(first.slug, "intro");
        let free = file
            .event_types
            .iter()
            .take_while(|kind| kind.price_minor == 0)
            .count();
        let (free, paid) = file.event_types.split_at(free);
        assert!(free.iter().all(|kind| kind.payment_methods.is_empty()));
        assert!(!paid.is_empty());
        for kind in paid {
            assert!(kind.price_minor > 0, "{}", kind.slug);
            assert_eq!(
                kind.payment_methods,
                [
                    PaymentMethod::Card,
                    PaymentMethod::MbWay,
                    PaymentMethod::Paypal
                ]
            );
            assert!(kind.summary.ends_with(" with Marco Ferra"));
        }
        assert!(HOLD_RANGE.contains(&file.booking.hold_minutes));
        assert_eq!(file.booking.refund_until_hours, 24);
        assert!(WITHDRAWAL_RANGE.contains(&file.booking.withdrawal_days));
        assert!(!WITHDRAWAL_RANGE.contains(&13));
        Ok(())
    }

    #[test]
    fn a_slug_is_used_once() -> TestResult {
        let file: File = serde_json::from_str(COMMITTED)?;
        let mut twice = file.event_types.clone();
        twice.extend(file.event_types);
        assert!(check_event_types(&twice).is_err());
        Ok(())
    }

    /// Each keeps the type of the setting it overrides.
    #[test]
    fn every_override_lands_on_a_setting_of_the_committed_file() -> TestResult {
        let committed = Settings::parse(COMMITTED, "marcoferra.config.json")?;
        let settings = committed.overridden(&OVERRIDES, |_| Ok(Some("1".to_owned())))?;
        assert_eq!(settings.get::<u16>("/email/port")?, 1);
        Ok(())
    }
}
