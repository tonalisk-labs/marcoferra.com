//! Google Calendar as the calendar owner: an access token refreshed from the stored refresh
//! token and cached until a minute before it expires, then freeBusy and events.

use std::time::{Duration, Instant};

use jiff::Timestamp;
use reqwest::{Method, RequestBuilder, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::{Google as Settings, Secrets};
use crate::error::ApiError;

/// Each call's budget; a slower answer is an outage to the visitor, who may retry.
const CALL_TIMEOUT: Duration = Duration::from_secs(8);

/// Google event ids are base32hex: lowercase `a`–`v` and digits (RFC 4648 §7).
const BASE32HEX: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";

/// The event id for a slot's `generation`: the same slot always names the same event, so two
/// instances inserting it at once collide in Google rather than both succeeding.
pub(crate) fn event_id(start: Timestamp, generation: u32) -> String {
    let mut digest = Sha256::new();
    digest.update(start.as_second().to_be_bytes());
    digest.update(generation.to_be_bytes());
    let digit = |value: u32| {
        let index = usize::try_from(value & 31).unwrap_or_default();
        BASE32HEX.get(index).map_or('0', |byte| char::from(*byte))
    };
    let (mut id, mut buffer, mut bits) = (String::from("mf"), 0_u32, 0_u32);
    for byte in digest.finalize() {
        buffer = (buffer & 0xff).wrapping_shl(8) | u32::from(byte);
        bits = bits.saturating_add(8);
        while bits >= 5 {
            bits = bits.saturating_sub(5);
            id.push(digit(buffer.wrapping_shr(bits)));
        }
    }
    if bits > 0 {
        id.push(digit(buffer.wrapping_shl(5_u32.saturating_sub(bits))));
    }
    id
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Event {
    pub id: String,
    #[serde(rename = "iCalUID")]
    pub ical_uid: String,
    pub status: String,
    #[serde(default)]
    pub sequence: u32,
    pub created: Timestamp,
    pub updated: Timestamp,
    #[serde(default)]
    pub description: String,
    pub start: When,
    pub end: When,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    pub hangout_link: Option<String>,
    pub extended_properties: Option<Extended>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct When {
    pub date_time: Timestamp,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attendee {
    pub email: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Deserialize, Clone)]
pub(crate) struct Extended {
    #[serde(default)]
    pub private: serde_json::Map<String, Value>,
}

impl Event {
    pub(crate) fn private(&self, key: &str) -> Option<&str> {
        self.extended_properties
            .as_ref()?
            .private
            .get(key)?
            .as_str()
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.status == "cancelled"
    }
}

#[derive(Deserialize)]
struct Granted {
    access_token: String,
    expires_in: u64,
}

/// What an insert found: the new event, or an event that already has its id.
pub(crate) enum Inserted {
    Created(Box<Event>),
    Exists,
}

pub(crate) struct Calendar {
    http: reqwest::Client,
    events: String,
    free_busy: String,
    id: String,
    token_url: String,
    /// The OAuth grant: client id, secret and refresh token.
    grant: [(&'static str, SecretString); 3],
    /// Held across a refresh, so callers arriving meanwhile wait for its token.
    token: tokio::sync::Mutex<Option<(String, Instant)>>,
}

fn unavailable(what: &str, detail: impl std::fmt::Display) -> ApiError {
    tracing::error!(%detail, "{what} failed");
    ApiError::upstream(
        "The calendar is not answering. Nothing was booked. Please try again shortly.",
    )
}

impl Calendar {
    pub(crate) fn new(settings: &Settings, secrets: &Secrets, http: reqwest::Client) -> Self {
        let calendar = url_segment(&settings.calendar_id);
        let api = settings.api_base.trim_end_matches('/');
        Self {
            http,
            events: format!("{api}/calendar/v3/calendars/{calendar}/events"),
            free_busy: format!("{api}/calendar/v3/freeBusy"),
            id: settings.calendar_id.clone(),
            token_url: settings.token_url.clone(),
            grant: [
                ("client_id", secrets.google_client_id.clone()),
                ("client_secret", secrets.google_client_secret.clone()),
                ("refresh_token", secrets.google_refresh_token.clone()),
            ],
            token: tokio::sync::Mutex::new(None),
        }
    }

    pub(crate) async fn access_token(&self) -> Result<String, ApiError> {
        let mut cached = self.token.lock().await;
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let grant = self
            .grant
            .iter()
            .map(|(name, value)| (*name, value.expose_secret()));
        let form: Vec<_> = grant.chain([("grant_type", "refresh_token")]).collect();
        let started = Instant::now();
        let reply = self
            .http
            .post(&self.token_url)
            .timeout(CALL_TIMEOUT)
            .form(&form)
            .send()
            .await;
        timed("token", started, reply.as_ref().ok());
        let reply = reply
            .and_then(reqwest::Response::error_for_status)
            .map_err(|err| unavailable("token refresh", err))?;
        let granted: Granted = reply
            .json()
            .await
            .map_err(|err| unavailable("token refresh", err))?;
        let lifetime = Duration::from_secs(granted.expires_in.saturating_sub(60));
        *cached = Instant::now()
            .checked_add(lifetime)
            .map(|until| (granted.access_token.clone(), until));
        drop(cached);
        Ok(granted.access_token)
    }

    /// The status and body, whatever the status.
    async fn call(
        &self,
        what: &str,
        request: RequestBuilder,
    ) -> Result<(StatusCode, Vec<u8>), ApiError> {
        let token = self.access_token().await?;
        let started = Instant::now();
        let reply = request
            .timeout(CALL_TIMEOUT)
            .bearer_auth(token)
            .send()
            .await;
        timed(what, started, reply.as_ref().ok());
        let reply = reply.map_err(|err| unavailable(what, err))?;
        let status = reply.status();
        let body = reply.bytes().await.map_err(|err| unavailable(what, err))?;
        Ok((status, body.to_vec()))
    }

    fn parse<T: DeserializeOwned>(
        what: &str,
        (status, body): (StatusCode, Vec<u8>),
    ) -> Result<T, ApiError> {
        if !status.is_success() {
            return Err(unavailable(
                what,
                format!("{status} {}", String::from_utf8_lossy(&body)),
            ));
        }
        serde_json::from_slice(&body).map_err(|err| unavailable(what, err))
    }

    pub(crate) async fn busy(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<(Timestamp, Timestamp)>, ApiError> {
        #[derive(Deserialize)]
        struct Interval {
            start: Timestamp,
            end: Timestamp,
        }
        let body = json!({ "timeMin": from, "timeMax": to, "items": [{ "id": self.id }] });
        let request = self.http.post(&self.free_busy).json(&body);
        let reply: Value = Self::parse("freeBusy", self.call("freeBusy", request).await?)?;
        // A calendar Google could not read comes back without `busy`: an outage, not a free day.
        let pointer = format!(
            "/calendars/{}/busy",
            self.id.replace('~', "~0").replace('/', "~1")
        );
        let busy = reply
            .pointer(&pointer)
            .cloned()
            .ok_or_else(|| unavailable("freeBusy", &reply))?;
        let intervals: Vec<Interval> =
            serde_json::from_value(busy).map_err(|err| unavailable("freeBusy", err))?;
        Ok(intervals
            .into_iter()
            .map(|busy| (busy.start, busy.end))
            .collect())
    }

    /// Inserts without Google's own invitation: the booking sends its own.
    pub(crate) async fn insert(&self, event: &Value) -> Result<Inserted, ApiError> {
        let url = format!("{}?conferenceDataVersion=1&sendUpdates=none", self.events);
        let request = self.http.post(url).json(event);
        let reply = self.call("events.insert", request).await?;
        if reply.0 == StatusCode::CONFLICT {
            return Ok(Inserted::Exists);
        }
        Self::parse("events.insert", reply).map(|event| Inserted::Created(Box::new(event)))
    }

    pub(crate) async fn get(&self, id: &str) -> Result<Option<Event>, ApiError> {
        let request = self
            .http
            .get(format!("{}/{}", self.events, url_segment(id)));
        let reply = self.call("events.get", request).await?;
        if matches!(reply.0, StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(None);
        }
        Self::parse("events.get", reply).map(Some)
    }

    /// Sets an event's status, telling no guest: the booking sends its own mail.
    pub(crate) async fn set_status(&self, id: &str, status: &str) -> Result<Event, ApiError> {
        self.patch(id, &json!({ "status": status })).await
    }

    /// Changes the members `fields` names, telling no guest; a conference it requests is made.
    pub(crate) async fn patch(&self, id: &str, fields: &Value) -> Result<Event, ApiError> {
        let url = format!(
            "{}/{}?conferenceDataVersion=1&sendUpdates=none",
            self.events,
            url_segment(id)
        );
        let request = self.http.request(Method::PATCH, url).json(fields);
        Self::parse("events.patch", self.call("events.patch", request).await?)
    }

    /// Deletes an event, telling no guest; Google keeps its id, as a cancelled event.
    pub(crate) async fn delete(&self, id: &str) -> Result<(), ApiError> {
        let url = format!("{}/{}?sendUpdates=none", self.events, url_segment(id));
        let (status, body) = self.call("events.delete", self.http.delete(url)).await?;
        if status.is_success() || status == StatusCode::GONE {
            return Ok(());
        }
        Err(unavailable(
            "events.delete",
            format!("{status} {}", String::from_utf8_lossy(&body)),
        ))
    }

    /// The events not yet ended whose private property `key` is `value`.
    pub(crate) async fn upcoming(
        &self,
        key: &str,
        value: &str,
        now: Timestamp,
    ) -> Result<Vec<Event>, ApiError> {
        self.listed(key, value, now, None).await
    }

    /// The events whose private property `key` is `value` and which overlap `from` to `to`.
    pub(crate) async fn overlapping(
        &self,
        (key, value): (&str, &str),
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Event>, ApiError> {
        self.listed(key, value, from, Some(to)).await
    }

    /// Events by a private property, ending after `since` and, if given, starting before `until`;
    /// cancelled ones are left out.
    async fn listed(
        &self,
        key: &str,
        value: &str,
        since: Timestamp,
        until: Option<Timestamp>,
    ) -> Result<Vec<Event>, ApiError> {
        #[derive(Deserialize)]
        struct Page {
            #[serde(default)]
            items: Vec<Event>,
        }
        let (property, since) = (
            url_segment(&format!("{key}={value}")),
            url_segment(&since.to_string()),
        );
        let until = until
            .map(|until| format!("&timeMax={}", url_segment(&until.to_string())))
            .unwrap_or_default();
        let url = format!(
            "{}?privateExtendedProperty={property}&timeMin={since}{until}&singleEvents=true",
            self.events
        );
        let request = self.http.get(url);
        let page: Page = Self::parse("events.list", self.call("events.list", request).await?)?;
        Ok(page
            .items
            .into_iter()
            .filter(|event| !event.cancelled())
            .collect())
    }
}

/// Logs one Google call's latency and protocol, so production shows what a request costs.
fn timed(what: &str, started: Instant, reply: Option<&reqwest::Response>) {
    let ms = started.elapsed().as_millis();
    let status = reply.map(|reply| reply.status().as_u16());
    let version = reply.map(|reply| format!("{:?}", reply.version()));
    tracing::info!(call = what, ms, ?status, ?version, "google");
}

/// A path segment or query value, percent-encoded: a calendar id may be an address.
pub(crate) fn url_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::event_id;
    use crate::TestResult;

    #[test]
    fn a_slot_names_one_event_per_generation_in_googles_alphabet() -> TestResult {
        let start = "2026-01-06T10:00:00Z".parse()?;
        let first = event_id(start, 0);
        assert_eq!(first, event_id(start, 0));
        assert_ne!(first, event_id(start, 1));
        assert_ne!(first, event_id("2026-01-06T11:00:00Z".parse()?, 0));
        assert_eq!(first.len(), 2 + 52);
        assert!(first.starts_with("mf"));
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
        );
        Ok(())
    }
}
