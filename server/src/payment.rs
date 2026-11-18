//! A paid booking's life as its event's `mfState` records it, what each word from Stripe does to
//! it as a pure decision, the refund window, and the receipt the invitation carries.

use jiff::{SignedDuration, Timestamp};

use crate::google::Event;

/// Where a paid booking stands, as its event says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hold {
    /// The time is held for a payment not yet seen.
    Held,
    /// Paid, and the invitation not yet accepted by the relay.
    Paid,
    /// Paid, and invited.
    Confirmed,
    /// No hold for this payment: never made, released, cancelled, or now another's.
    Gone,
}

impl Hold {
    /// The state of `event` for the Checkout session `session`; an event a later session holds
    /// is gone for this one.
    pub(crate) fn of(event: Option<&Event>, session: &str) -> Self {
        let Some(event) = event.filter(|event| !event.cancelled()) else {
            return Self::Gone;
        };
        if event
            .private("mfSession")
            .is_some_and(|held| held != session)
        {
            return Self::Gone;
        }
        Self::named(event)
    }

    /// The state an event names, whichever session it holds.
    pub(crate) fn named(event: &Event) -> Self {
        if event.cancelled() {
            return Self::Gone;
        }
        match event.private("mfState") {
            Some("held") => Self::Held,
            Some("paid") => Self::Paid,
            Some("confirmed") => Self::Confirmed,
            _ => Self::Gone,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Signal {
    /// The session is paid.
    Paid,
    /// The session expired, was abandoned, or its delayed payment failed.
    Released,
    /// The payment was refunded in full.
    Refunded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    /// Add the guest and the video call, send the invitation, then mark it confirmed.
    Confirm,
    /// Send the invitation an earlier attempt could not, then mark it confirmed.
    Resend,
    /// Delete the held event, freeing the time.
    Release,
    /// Cancel the meeting and send the cancellation.
    Cancel,
    /// The time is no longer held for this payment: refund it and apologise.
    RefundLate,
    /// Already done, or nothing to do.
    Nothing,
}

/// The transition table. A step done once is never done again: each moves the state on.
pub(crate) const fn step(hold: Hold, signal: Signal) -> Step {
    match (hold, signal) {
        (Hold::Held, Signal::Paid) => Step::Confirm,
        (Hold::Paid, Signal::Paid) => Step::Resend,
        (Hold::Gone, Signal::Paid) => Step::RefundLate,
        (Hold::Held, Signal::Released) => Step::Release,
        (Hold::Paid | Hold::Confirmed, Signal::Refunded) => Step::Cancel,
        (Hold::Confirmed, Signal::Paid)
        | (Hold::Paid | Hold::Confirmed | Hold::Gone, Signal::Released)
        | (Hold::Held | Hold::Gone, Signal::Refunded) => Step::Nothing,
    }
}

/// A paid booking's refund terms: the consumer's withdrawal period after booking, and the
/// notice before the start that a later cancellation still keeps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Terms {
    pub withdrawal_days: u32,
    pub notice_hours: u32,
}

impl Terms {
    fn withdrawal_ends(self, booked: Timestamp) -> Timestamp {
        let days = SignedDuration::from_hours(i64::from(self.withdrawal_days).saturating_mul(24));
        booked.checked_add(days).unwrap_or(booked)
    }

    fn notice_ends(self, start: Timestamp) -> Timestamp {
        start
            .checked_sub(SignedDuration::from_hours(self.notice_hours.into()))
            .unwrap_or(start)
    }
}

/// Whether a meeting booked at `booked` and cancelled at `now` is refunded in full: before it
/// starts, within the withdrawal period (EU Directive 2011/83, art. 9) or the notice before it.
pub(crate) fn refundable(
    booked: Timestamp,
    start: Timestamp,
    now: Timestamp,
    terms: Terms,
) -> bool {
    now < start && (now < terms.withdrawal_ends(booked) || now <= terms.notice_ends(start))
}

/// The last moment a cancellation is refunded, as the cancellation page states it.
pub(crate) fn refund_deadline(booked: Timestamp, start: Timestamp, terms: Terms) -> Timestamp {
    terms
        .withdrawal_ends(booked)
        .max(terms.notice_ends(start))
        .min(start)
}

/// An amount as the email writes it: `€40.00`, or `40.00 USD` in another currency.
pub(crate) fn money(minor: i64, currency: &str) -> String {
    let (units, cents) = (minor.div_euclid(100), minor.rem_euclid(100));
    if currency.eq_ignore_ascii_case("eur") {
        format!("€{units}.{cents:02}")
    } else {
        format!("{units}.{cents:02} {}", currency.to_ascii_uppercase())
    }
}

/// A payment method as a person names it.
fn method_name(method: &str) -> String {
    match method {
        "card" => "card".to_owned(),
        "mb_way" => "MB WAY".to_owned(),
        "paypal" => "PayPal".to_owned(),
        "link" => "Link".to_owned(),
        other => other.replace('_', " "),
    }
}

/// The VAT statement every receipt and the terms carry: the seller is exempt (Art. 53 CIVA).
pub(crate) const VAT_NOTE: &str = "IVA, regime de isenção, art. 53.º do CIVA.";

pub(crate) struct Receipt<'a> {
    pub amount_minor: i64,
    pub currency: &'a str,
    pub method: Option<&'a str>,
    pub link: Option<&'a str>,
    pub terms: Terms,
    /// Whether the visitor asked for the session within the withdrawal period, waiving it.
    pub waiver: bool,
}

/// The receipt's paragraphs, in the order the email gives them.
pub(crate) fn receipt_lines(receipt: &Receipt<'_>) -> Vec<String> {
    let method = receipt
        .method
        .map(|method| format!(" by {}", method_name(method)))
        .unwrap_or_default();
    let mut lines = vec![format!(
        "Paid: {}{method}. No VAT is charged: {VAT_NOTE}",
        money(receipt.amount_minor, receipt.currency)
    )];
    if let Some(url) = receipt.link {
        lines.push(format!("Your receipt from Stripe: {url}"));
    }
    let (days, hours) = (receipt.terms.withdrawal_days, receipt.terms.notice_hours);
    lines.push(format!(
        "Cancelling before the start and within {days} days of booking, or at least {hours} hours before the start, refunds the payment in full. Any other cancellation is not refunded."
    ));
    if receipt.waiver {
        lines.push(
            "You asked for the session to take place within the 14-day withdrawal period and acknowledged that you lose the right of withdrawal once it has been given in full."
                .to_owned(),
        );
    }
    lines
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        Hold, Receipt, Signal, Step, Terms, money, receipt_lines, refund_deadline, refundable, step,
    };
    use crate::TestResult;
    use crate::google::Event;

    fn event(
        status: &str,
        state: Option<&str>,
        session: Option<&str>,
    ) -> Result<Event, serde_json::Error> {
        let mut private = serde_json::Map::new();
        if let Some(state) = state {
            private.insert("mfState".to_owned(), json!(state));
        }
        if let Some(session) = session {
            private.insert("mfSession".to_owned(), json!(session));
        }
        serde_json::from_value(json!({
            "id": "mfabc", "iCalUID": "mfabc@google.com", "status": status,
            "created": "2026-01-05T09:00:00Z", "updated": "2026-01-05T09:00:00Z",
            "start": {"dateTime": "2026-01-06T10:00:00Z"}, "end": {"dateTime": "2026-01-06T10:30:00Z"},
            "extendedProperties": {"private": private},
        }))
    }

    #[test]
    fn an_event_names_its_state_for_its_own_session_only() -> TestResult {
        let held = event("confirmed", Some("held"), Some("cs_1"))?;
        assert_eq!(Hold::of(Some(&held), "cs_1"), Hold::Held);
        assert_eq!(Hold::of(Some(&held), "cs_2"), Hold::Gone);
        let unsaved = event("confirmed", Some("held"), None)?;
        assert_eq!(Hold::of(Some(&unsaved), "cs_1"), Hold::Held);
        let released = event("cancelled", Some("held"), Some("cs_1"))?;
        assert_eq!(Hold::of(Some(&released), "cs_1"), Hold::Gone);
        let free = event("confirmed", None, None)?;
        assert_eq!(Hold::of(Some(&free), "cs_1"), Hold::Gone);
        assert_eq!(Hold::of(None, "cs_1"), Hold::Gone);
        let paid = event("confirmed", Some("paid"), Some("cs_1"))?;
        assert_eq!(Hold::named(&paid), Hold::Paid);
        let confirmed = event("confirmed", Some("confirmed"), Some("cs_1"))?;
        assert_eq!(Hold::of(Some(&confirmed), "cs_1"), Hold::Confirmed);
        Ok(())
    }

    /// A repeated signal finds the state it left and does nothing. Only a payment with no hold
    /// left to take it is refunded.
    #[test]
    fn each_signal_moves_each_state_once() {
        let table = [
            (Hold::Held, Signal::Paid, Step::Confirm),
            (Hold::Paid, Signal::Paid, Step::Resend),
            (Hold::Confirmed, Signal::Paid, Step::Nothing),
            (Hold::Gone, Signal::Paid, Step::RefundLate),
            (Hold::Held, Signal::Released, Step::Release),
            (Hold::Paid, Signal::Released, Step::Nothing),
            (Hold::Confirmed, Signal::Released, Step::Nothing),
            (Hold::Gone, Signal::Released, Step::Nothing),
            (Hold::Held, Signal::Refunded, Step::Nothing),
            (Hold::Paid, Signal::Refunded, Step::Cancel),
            (Hold::Confirmed, Signal::Refunded, Step::Cancel),
            (Hold::Gone, Signal::Refunded, Step::Nothing),
        ];
        for (hold, signal, expected) in table {
            assert_eq!(step(hold, signal), expected, "{hold:?} + {signal:?}");
        }
    }

    const TERMS: Terms = Terms {
        withdrawal_days: 14,
        notice_hours: 24,
    };

    /// Booked 13 to 15 days before, cancelled 23 or 25 hours before the start: refunded within
    /// the withdrawal period or with enough notice, never after the start.
    #[test]
    fn a_cancellation_is_refunded_within_the_withdrawal_period_or_with_notice() -> TestResult {
        let now: jiff::Timestamp = "2026-02-01T10:00:00Z".parse()?;
        let ago = |days: i64| now.checked_sub(jiff::SignedDuration::from_hours(days * 24));
        let ahead = |hours: i64| now.checked_add(jiff::SignedDuration::from_hours(hours));
        for (booked_days, start_hours, refunded) in [
            (13, 23, true),
            (13, 25, true),
            (14, 23, false),
            (14, 25, true),
            (15, 23, false),
            (15, 25, true),
            (1, -1, false),
            (15, 24, true),
        ] {
            let (booked, start) = (ago(booked_days)?, ahead(start_hours)?);
            assert_eq!(
                refundable(booked, start, now, TERMS),
                refunded,
                "booked {booked_days} d before, {start_hours} h ahead"
            );
        }
        let (booked, start) = (ago(13)?, ahead(30)?);
        assert_eq!(
            refund_deadline(booked, start, TERMS),
            ago(-1)?,
            "the withdrawal period ends first"
        );
        let (booked, start) = (ago(15)?, ahead(30)?);
        assert_eq!(
            refund_deadline(booked, start, TERMS),
            ahead(6)?,
            "the notice ends then"
        );
        let (booked, start) = (ago(1)?, ahead(5)?);
        assert_eq!(
            refund_deadline(booked, start, TERMS),
            start,
            "never past the start"
        );
        Ok(())
    }

    #[test]
    fn the_receipt_states_the_amount_method_link_terms_waiver_and_vat() {
        assert_eq!(money(4000, "eur"), "€40.00");
        assert_eq!(money(12_050, "usd"), "120.50 USD");
        let lines = receipt_lines(&Receipt {
            amount_minor: 4000,
            currency: "eur",
            method: Some("mb_way"),
            link: Some("https://pay.stripe.com/receipts/r1"),
            terms: TERMS,
            waiver: true,
        });
        assert_eq!(
            lines.first().map(String::as_str),
            Some(
                "Paid: €40.00 by MB WAY. No VAT is charged: IVA, regime de isenção, art. 53.º do CIVA."
            )
        );
        let text = lines.join("\n");
        for expected in [
            "https://pay.stripe.com/receipts/r1",
            "within 14 days of booking, or at least 24 hours before the start, refunds the payment in full",
            "lose the right of withdrawal",
        ] {
            assert!(text.contains(expected), "{expected}");
        }
        let plain = receipt_lines(&Receipt {
            amount_minor: 4000,
            currency: "eur",
            method: None,
            link: None,
            terms: TERMS,
            waiver: false,
        });
        assert_eq!(plain.len(), 2);
    }
}
