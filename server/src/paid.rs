//! Paid bookings: the hold and its Checkout session, the payment's confirmation, the release of
//! an unpaid hold, and refunds, each under its caller's booking lock as `payment::step` decides.

use jiff::{SignedDuration, Timestamp};
use kit_server::stripe::StripeError;
use serde_json::{Map, Value, json};

use crate::config::EventType;
use crate::error::{ALREADY_PAID, ApiError};
use crate::google::Event;
use crate::grid::{self, Rules};
use crate::ics::Method;
use crate::payment::{
    Hold, Receipt, Signal, Step, Terms, money, receipt_lines, refund_deadline, refundable, step,
};
use crate::routes::Valid;
use crate::security::Purpose;
use crate::stripe::{Checkout, Session, checkout_form};
use crate::{App, Plain, taken};

/// Added to the hold, so Stripe, whose clock and latency differ, still sees its 30 minutes.
const HOLD_MARGIN: SignedDuration = SignedDuration::from_mins(1);

pub(crate) struct Pending {
    pub id: String,
    pub url: String,
    pub expires_at: Timestamp,
}

pub(crate) enum Confirmation {
    /// Booked, and the invitation sent.
    Booked(Box<Event>),
    /// Paid by a method that settles later; Stripe says when.
    Pending,
    /// Not paid: the session is open or expired.
    Unpaid,
    /// Paid after the time stopped being held for it, so refunded.
    Refunded,
}

/// The event's private properties with `changes` made, whole: a patch replaces the map.
fn private_with(event: &Event, changes: &[(&str, &str)]) -> Value {
    let mut private = event
        .extended_properties
        .as_ref()
        .map(|extended| extended.private.clone())
        .unwrap_or_default();
    for (key, value) in changes {
        private.insert((*key).to_owned(), Value::String((*value).to_owned()));
    }
    let mut extended = Map::new();
    extended.insert("private".to_owned(), Value::Object(private));
    json!({ "extendedProperties": extended })
}

fn hold_until(event: &Event) -> Option<i64> {
    event.private("holdUntil")?.parse().ok()
}

/// When a paid booking was made: its hold's moment, or the event's creation for one held before
/// that was noted.
pub(crate) fn booked_at(event: &Event) -> Timestamp {
    event
        .private("bookedAt")
        .and_then(|seconds| seconds.parse().ok())
        .and_then(|seconds| Timestamp::from_second(seconds).ok())
        .unwrap_or(event.created)
}

impl App {
    pub(crate) const fn terms(&self) -> Terms {
        Terms {
            withdrawal_days: self.config.booking.withdrawal_days,
            notice_hours: self.config.booking.refund_until_hours,
        }
    }

    /// The last moment cancelling `event` is refunded in full, for a paid one.
    pub(crate) fn refund_deadline(&self, event: &Event) -> Option<Timestamp> {
        let paid = event
            .private("type")
            .and_then(|slug| self.kind(slug))
            .is_some_and(|kind| kind.price_minor > 0);
        paid.then(|| refund_deadline(booked_at(event), event.start.date_time, self.terms()))
    }

    fn kind_of(&self, event: &Event) -> Result<&EventType, ApiError> {
        event
            .private("type")
            .and_then(|slug| self.kind(slug))
            .ok_or_else(|| ApiError::upstream("The booking names no meeting type offered now."))
    }

    /// Frees the time `valid` asks for, or refuses it as taken. A hold in the way whose payment
    /// can no longer come is released first: this is the only place that looks.
    pub(crate) async fn clear(&self, valid: &Valid, now: Timestamp) -> Result<(), ApiError> {
        let (from, to) = grid::span(valid.start, Rules::of(&valid.kind));
        if self.calendar.busy(from, to).await?.is_empty() {
            return Ok(());
        }
        let mut released = false;
        for event in self
            .calendar
            .overlapping(("mfState", "held"), from, to)
            .await?
        {
            released |= self.lapsed(&event, now).await?;
        }
        if released && self.calendar.busy(from, to).await?.is_empty() {
            return Ok(());
        }
        // The free times shown were stale: the next read goes to the calendar.
        self.forget_busy();
        Err(taken())
    }

    /// Whether a hold was released because Stripe will take no payment for it: its session
    /// expired, or its time ran out, or it never had a session and its time ran out.
    async fn lapsed(&self, event: &Event, now: Timestamp) -> Result<bool, ApiError> {
        let past = hold_until(event).is_some_and(|until| now.as_second() >= until);
        // A hold with no session Stripe knows lapses with its time alone.
        let known = match event.private("mfSession") {
            Some(id) => match self.stripe.session(id).await {
                Err(StripeError::Missing) => None,
                read => Some(read?),
            },
            None => None,
        };
        let Some(mut session) = known else {
            if past {
                self.calendar.delete(&event.id).await?;
                self.forget_busy();
            }
            return Ok(past);
        };
        let id = session.id.clone();
        // Stripe expires a session some time after its moment; the hold does not wait for it.
        if session.status == "open" && past {
            session = match self.stripe.expire(&id).await {
                Err(StripeError::Refused) => self.stripe.session(&id).await?,
                expired => expired?,
            };
        }
        if session.status != "expired" {
            return Ok(false);
        }
        self.release(&session, false).await
    }

    /// Holds the time and opens its Checkout session; a session that cannot be made frees the
    /// time again.
    pub(crate) async fn hold(
        &self,
        valid: &Valid,
        (key, holder): (&str, &str),
        now: Timestamp,
    ) -> Result<Pending, ApiError> {
        self.clear(valid, now).await?;
        let minutes = SignedDuration::from_mins(self.config.booking.hold_minutes.into());
        let until = now
            .checked_add(minutes)
            .and_then(|until| until.checked_add(HOLD_MARGIN))
            .map_err(|_| ApiError::upstream("The hold is out of range."))?;
        let (event, replayed) = self.insert(valid, key, holder, Some(until)).await?;
        self.forget_busy();
        let pending = self.checkout(&event).await;
        if pending.is_err() && !replayed {
            if let Err(err) = self.calendar.delete(&event.id).await {
                tracing::error!(error = ?err, id = event.id, "the unpaid hold could not be deleted");
            }
            self.forget_busy();
        }
        pending
    }

    /// The hold's Checkout page, while its session is open.
    pub(crate) async fn checkout(&self, event: &Event) -> Result<Pending, ApiError> {
        let session = self.session_for(event).await?;
        let url = session
            .url
            .filter(|_| session.status == "open")
            .ok_or_else(|| {
                ApiError::refused(
                    &ALREADY_PAID,
                    "This time is already paid for, or its payment is under way.",
                )
            })?;
        Ok(Pending {
            id: event.id.clone(),
            url,
            expires_at: Timestamp::from_second(session.expires_at).unwrap_or(Timestamp::UNIX_EPOCH),
        })
    }

    /// The hold's Checkout session: made once per event, as Stripe replays its key to the same
    /// form, then noted on the event.
    async fn session_for(&self, event: &Event) -> Result<Session, ApiError> {
        let kind = self.kind_of(event)?;
        let expires_at =
            hold_until(event).ok_or_else(|| ApiError::upstream("The held time has no end."))?;
        let base = &self.config.public_base_url;
        let (id, slug) = (&event.id, &kind.slug);
        let success_url = format!("{base}/book/{slug}/done/?session_id={{CHECKOUT_SESSION_ID}}");
        let token = self.keys.booking(Purpose::Release, id);
        let cancel_url = format!("{base}/book/{slug}/?release={id}&token={token}");
        let methods: Vec<&str> = kind
            .payment_methods
            .iter()
            .map(|method| method.name())
            .collect();
        let when = crate::when(event);
        let form = checkout_form(&Checkout {
            event: id,
            slug,
            email: event.private("email").unwrap_or_default(),
            title: &kind.title,
            when: &when,
            price_minor: kind.price_minor,
            currency: &kind.currency,
            methods: &methods,
            expires_at,
            success_url: &success_url,
            cancel_url: &cancel_url,
        });
        let session = self
            .stripe
            .create_session(&form, &format!("checkout-{id}"))
            .await?;
        if event.private("mfSession") != Some(session.id.as_str()) {
            let noted = private_with(event, &[("mfSession", &session.id)]);
            // A hold without its session still matches it; only the release link needs it.
            if let Err(err) = self.calendar.patch(id, &noted).await {
                tracing::warn!(error = %err, id, "the session could not be noted on its hold");
            }
        }
        Ok(session)
    }

    /// Ends an earlier hold of the same visitor's: its session expires at Stripe, and the time
    /// is freed. A session already completed is left to its payment.
    pub(crate) async fn supersede(&self, event: &Event) -> Result<(), ApiError> {
        let Some(id) = event.private("mfSession") else {
            self.calendar.delete(&event.id).await?;
            self.forget_busy();
            return Ok(());
        };
        match self.stripe.expire(id).await {
            Ok(session) => self.release(&session, false).await.map(drop),
            Err(StripeError::Refused) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// The release link from Checkout's way back: the session expires at Stripe, and the hold is
    /// released by the same step Stripe's `checkout.session.expired` takes.
    pub(crate) async fn give_up(&self, event: &Event) -> Result<(), ApiError> {
        let session = match event.private("mfSession") {
            Some(id) => id.to_owned(),
            None => self.session_for(event).await?.id,
        };
        let session = match self.stripe.expire(&session).await {
            Err(StripeError::Refused) => self.stripe.session(&session).await?,
            expired => expired?,
        };
        if session.status == "complete" {
            return Err(ApiError::refused(
                &ALREADY_PAID,
                "This time is already paid for, or its payment is under way.",
            ));
        }
        let _turn = self.booking.lock().await;
        self.release(&session, false).await.map(drop)
    }

    /// Frees an unpaid hold: the step Stripe's expiry, a failed delayed payment and the way back
    /// from Checkout all take. Answers whether a hold was released.
    pub(crate) async fn release(&self, session: &Session, failed: bool) -> Result<bool, ApiError> {
        let event = match session.event.as_deref() {
            Some(id) => self.calendar.get(id).await?,
            None => None,
        };
        let Some(event) = event.filter(|event| {
            step(Hold::of(Some(event), &session.id), Signal::Released) == Step::Release
        }) else {
            return Ok(false);
        };
        self.calendar.delete(&event.id).await?;
        self.forget_busy();
        if failed {
            let kind = self.kind_of(&event)?;
            let lines = [
                format!(
                    "Your payment for the {} on {} did not go through, so the time is free again and nothing was charged.",
                    kind.title,
                    crate::when(&event)
                ),
                format!(
                    "To book again: {}/book/{}/",
                    self.config.public_base_url, kind.slug
                ),
            ];
            self.tell(Some(&event), None, "Payment not completed", &lines)
                .await;
        }
        Ok(true)
    }

    /// A plain letter to the visitor a hold names, or else to `fallback`; a refusal is logged,
    /// as the payment's outcome stands either way.
    async fn tell(
        &self,
        event: Option<&Event>,
        fallback: Option<&str>,
        subject: &str,
        lines: &[String],
    ) {
        let named = |key: &str| event.and_then(|event| event.private(key));
        let letter = Plain {
            name: named("name").unwrap_or("there"),
            email: named("email").or(fallback).unwrap_or_default(),
            subject,
            lines,
        };
        if let Err(err) = self.send(&letter, None).await {
            tracing::error!(error = %err, "the relay refused a payment notice");
        }
    }

    /// Confirms a paid session: the one step both the webhook and the success page take.
    pub(crate) async fn confirm(&self, session_id: &str) -> Result<Confirmation, ApiError> {
        let session = self.stripe.session(session_id).await?;
        if !session.paid() {
            return Ok(if session.status == "complete" {
                Confirmation::Pending
            } else {
                Confirmation::Unpaid
            });
        }
        let event = match session.event.as_deref() {
            Some(id) => self.calendar.get(id).await?,
            None => None,
        };
        match (
            step(Hold::of(event.as_ref(), &session.id), Signal::Paid),
            event,
        ) {
            (Step::Confirm, Some(event)) => {
                let paid = self.mark_paid(&event, &session).await?;
                self.send_confirmed(paid, &session).await
            }
            (Step::Resend, Some(event)) => self.send_confirmed(event, &session).await,
            (Step::Nothing, Some(event)) => Ok(Confirmation::Booked(Box::new(event))),
            (_, event) => {
                self.refund_late(&session, event.as_ref()).await?;
                Ok(Confirmation::Refunded)
            }
        }
    }

    /// The hold becomes the meeting: the guest, the video call and the type's title, and the
    /// payment it was paid by.
    async fn mark_paid(&self, event: &Event, session: &Session) -> Result<Event, ApiError> {
        let kind = self.kind_of(event)?;
        let (name, email) = (
            event.private("name").unwrap_or_default(),
            event.private("email").unwrap_or_default(),
        );
        let mut changes = vec![("mfState", "paid"), ("mfSession", session.id.as_str())];
        if let Some(intent) = session.payment_intent.as_deref() {
            changes.push(("mfPayment", intent));
        }
        let note = event
            .description
            .split_once("\n\n")
            .map(|(_, note)| format!("\n\n{note}"))
            .unwrap_or_default();
        let mut fields = private_with(event, &changes);
        if let Some(object) = fields.as_object_mut() {
            object.insert("summary".to_owned(), json!(kind.summary));
            object.insert(
                "description".to_owned(),
                json!(format!(
                    "Booked and paid on marcoferra.com by {name} <{email}>.{note}"
                )),
            );
            object.insert(
                "attendees".to_owned(),
                json!([{ "email": email, "displayName": name }]),
            );
            object.insert(
                "conferenceData".to_owned(),
                json!({ "createRequest": {
                    "requestId": format!("paid-{}", event.id),
                    "conferenceSolutionKey": { "type": "hangoutsMeet" },
                } }),
            );
        }
        let paid = self.calendar.patch(&event.id, &fields).await?;
        self.with_video(paid).await
    }

    /// Sends the invitation with its receipt, then marks the meeting confirmed. A refusal
    /// answers 503, so Stripe asks again and the next attempt sends it.
    async fn send_confirmed(
        &self,
        event: Event,
        session: &Session,
    ) -> Result<Confirmation, ApiError> {
        let lines = receipt_lines(&Receipt {
            amount_minor: session.amount_total,
            currency: &session.currency,
            method: session.method.as_deref(),
            link: session.receipt_url.as_deref(),
            terms: self.terms(),
            waiver: event.private("waiver") == Some("true"),
        });
        if let Err(err) = self.invite(Method::Request, &event, &lines).await {
            tracing::error!(error = %err, id = event.id, "the relay refused a paid invitation");
            return Err(ApiError::upstream(
                "The payment is received, but the invitation could not be sent yet. It will follow shortly.",
            ));
        }
        self.cap.record(Timestamp::now());
        let confirmed = private_with(&event, &[("mfState", "confirmed")]);
        let event = self.calendar.patch(&event.id, &confirmed).await?;
        Ok(Confirmation::Booked(Box::new(event)))
    }

    /// A payment that came after its hold was released or its time taken: refunded in full,
    /// once, and the visitor told why.
    async fn refund_late(&self, session: &Session, event: Option<&Event>) -> Result<(), ApiError> {
        let Some(intent) = session.payment_intent.as_deref() else {
            return Ok(());
        };
        if session.refunded > 0 {
            return Ok(());
        }
        let amount = self
            .stripe
            .refund(intent, &format!("late-{}", session.id))
            .await?;
        tracing::warn!(session = session.id, "a late payment was refunded");
        let lines = [
            format!(
                "Your payment arrived after the time it was for had been released, so it could not be booked. I am sorry: the {} you paid is refunded in full.",
                money(amount, &session.currency)
            ),
            format!(
                "Please choose another time: {}/book/",
                self.config.public_base_url
            ),
        ];
        let subject = "Payment refunded: the time was no longer free";
        self.tell(event, session.email.as_deref(), subject, &lines)
            .await;
        Ok(())
    }

    /// A refund made at Stripe (the host's, from the Dashboard) cancels the meeting it paid for
    /// and sends the cancellation. A partial refund is a gesture, not a cancellation.
    pub(crate) async fn refunded(&self, charge: &Value) -> Result<(), ApiError> {
        let whole = charge.get("refunded").and_then(Value::as_bool) == Some(true);
        let intent = charge.get("payment_intent").and_then(Value::as_str);
        let (true, Some(intent)) = (whole, intent) else {
            return Ok(());
        };
        let lines = ["Your payment is refunded in full.".to_owned()];
        for event in self
            .calendar
            .upcoming("mfPayment", intent, Timestamp::now())
            .await?
        {
            if step(Hold::named(&event), Signal::Refunded) == Step::Cancel {
                self.cancel(&event, &lines).await?;
            }
        }
        Ok(())
    }

    /// A visitor's cancellation of a paid meeting: refunded in full when it comes early enough,
    /// before anything is cancelled. Answers the minor units refunded and the paragraph saying so.
    pub(crate) async fn refund_on_cancel(
        &self,
        event: &Event,
        now: Timestamp,
    ) -> Result<(i64, Option<String>), ApiError> {
        let Some(intent) = event.private("mfPayment") else {
            return Ok((0, None));
        };
        let terms = self.terms();
        if !refundable(booked_at(event), event.start.date_time, now, terms) {
            let (days, hours) = (terms.withdrawal_days, terms.notice_hours);
            let line = format!(
                "As it was cancelled more than {days} days after booking and less than {hours} hours before the start, the payment is not refunded."
            );
            return Ok((0, Some(line)));
        }
        let amount = self
            .stripe
            .refund(intent, &format!("refund-{}", event.id))
            .await?;
        let currency = self
            .kind_of(event)
            .map_or("eur", |kind| kind.currency.as_str());
        let line = format!(
            "Your payment of {} is refunded in full. It reaches your account within a few days.",
            money(amount, currency)
        );
        Ok((amount, Some(line)))
    }
}
