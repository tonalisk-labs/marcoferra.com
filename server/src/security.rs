use std::sync::{Mutex, MutexGuard};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use jiff::{SignedDuration, Timestamp};
use kit_server::sync::lock;
use sha2::Sha256;

use crate::error::ApiError;

/// What a booking token lets its holder do; each purpose signs apart.
#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Cancel,
    Ics,
    /// Gives up an unpaid hold, from Checkout's way back.
    Release,
}

/// HMAC-SHA256 under `SERVER_SECRET`: booking capabilities and the form's fill-time stamp.
pub(crate) struct Keys(Hmac<Sha256>);

impl Keys {
    pub(crate) fn new(secret: &str) -> Result<Self, String> {
        Hmac::new_from_slice(secret.as_bytes())
            .map(Self)
            .map_err(|_| "SERVER_SECRET cannot key an HMAC".to_owned())
    }

    fn mac(&self, label: &str, subject: &str) -> Hmac<Sha256> {
        let mut mac = self.0.clone();
        mac.update(label.as_bytes());
        mac.update(b"\0");
        mac.update(subject.as_bytes());
        mac
    }

    const fn label(purpose: Purpose) -> &'static str {
        match purpose {
            Purpose::Cancel => "cancel",
            Purpose::Ics => "ics",
            Purpose::Release => "release",
        }
    }

    pub(crate) fn booking(&self, purpose: Purpose, id: &str) -> String {
        URL_SAFE_NO_PAD.encode(self.mac(Self::label(purpose), id).finalize().into_bytes())
    }

    /// Checked in constant time.
    pub(crate) fn holds(&self, purpose: Purpose, id: &str, token: &str) -> bool {
        let Ok(tag) = URL_SAFE_NO_PAD.decode(token) else {
            return false;
        };
        self.mac(Self::label(purpose), id)
            .verify_slice(&tag)
            .is_ok()
    }

    /// `<unix seconds>.<mac>`: when the form was handed out, which the client cannot move.
    pub(crate) fn form(&self, issued: Timestamp) -> String {
        let seconds = issued.as_second().to_string();
        let tag = URL_SAFE_NO_PAD.encode(self.mac("form", &seconds).finalize().into_bytes());
        format!("{seconds}.{tag}")
    }

    /// How long ago a genuine form token was issued; `None` for any other string.
    pub(crate) fn form_age(&self, token: &str, now: Timestamp) -> Option<SignedDuration> {
        let (seconds, tag) = token.split_once('.')?;
        self.mac("form", seconds)
            .verify_slice(&URL_SAFE_NO_PAD.decode(tag).ok()?)
            .ok()?;
        let issued = Timestamp::from_second(seconds.parse().ok()?).ok()?;
        Some(now.duration_since(issued))
    }
}

/// Invitations sent this UTC day, capped so that abuse cannot spend the relay's own limit.
pub(crate) struct DailyCap {
    max: u32,
    sent: Mutex<(jiff::civil::Date, u32)>,
}

impl DailyCap {
    pub(crate) fn new(max: u32) -> Self {
        Self {
            max,
            sent: Mutex::new((jiff::civil::Date::default(), 0)),
        }
    }

    fn today(&self, now: Timestamp) -> MutexGuard<'_, (jiff::civil::Date, u32)> {
        let date = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        let mut sent = lock(&self.sent);
        if sent.0 != date {
            *sent = (date, 0);
        }
        sent
    }

    /// Refuses once the cap is spent, saying when the next UTC day starts.
    pub(crate) fn check(&self, now: Timestamp) -> Result<(), ApiError> {
        if self.today(now).1 < self.max {
            return Ok(());
        }
        let midnight = now
            .to_zoned(jiff::tz::TimeZone::UTC)
            .tomorrow()
            .and_then(|day| day.start_of_day());
        let wait = midnight.map_or(SignedDuration::ZERO, |day| {
            day.timestamp().duration_since(now)
        });
        let detail =
            "Bookings are paused until tomorrow. Please email hello@marcoferra.com instead.";
        Err(ApiError::upstream(detail).after(wait.unsigned_abs()))
    }

    pub(crate) fn record(&self, now: Timestamp) {
        let mut sent = self.today(now);
        sent.1 = sent.1.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use super::{DailyCap, Keys, Purpose};
    use crate::TestResult;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_booking_token_opens_its_own_booking_for_its_own_purpose_only() -> TestResult {
        let keys = Keys::new(SECRET)?;
        let token = keys.booking(Purpose::Cancel, "mfabc");
        assert!(keys.holds(Purpose::Cancel, "mfabc", &token));
        assert!(!keys.holds(Purpose::Ics, "mfabc", &token));
        assert!(!keys.holds(Purpose::Release, "mfabc", &token));
        assert!(!keys.holds(Purpose::Cancel, "mfabd", &token));
        assert!(!keys.holds(Purpose::Cancel, "mfabc", "not-base64!"));
        assert!(!Keys::new("another secret, just as long as it")?.holds(
            Purpose::Cancel,
            "mfabc",
            &token
        ));
        Ok(())
    }

    #[test]
    fn a_form_token_tells_its_age_and_refuses_an_edited_time() -> TestResult {
        let keys = Keys::new(SECRET)?;
        let issued: Timestamp = "2026-01-05T10:00:00Z".parse()?;
        let token = keys.form(issued);
        let later = issued.checked_add(SignedDuration::from_secs(4))?;
        assert_eq!(
            keys.form_age(&token, later),
            Some(SignedDuration::from_secs(4))
        );
        let (_, tag) = token.split_once('.').ok_or("no tag")?;
        let earlier = format!("{}.{tag}", issued.as_second().saturating_sub(60));
        assert_eq!(keys.form_age(&earlier, later), None);
        Ok(())
    }

    #[test]
    fn the_daily_cap_refuses_once_spent_and_resets_each_utc_day() -> TestResult {
        let cap = DailyCap::new(1);
        let now: Timestamp = "2026-01-05T23:59:00Z".parse()?;
        cap.check(now)?;
        cap.record(now);
        assert!(cap.check(now).is_err());
        cap.check(now.checked_add(SignedDuration::from_mins(2))?)?;
        Ok(())
    }
}
