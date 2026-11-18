//! The Stripe calls a paid booking makes, on the shared client: Checkout sessions and refunds.

use kit_server::stripe::{NO_FORM, Stripe, StripeError, text};
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde_json::Value;

use crate::error::ApiError;
use crate::google::url_segment;

impl From<StripeError> for ApiError {
    fn from(err: StripeError) -> Self {
        match err {
            StripeError::Missing => {
                Self::status(StatusCode::NOT_FOUND, "There is no such payment.")
            }
            StripeError::Unavailable => Self::upstream(
                "The payment service is not answering. Nothing was charged. Please try again shortly.",
            ),
            StripeError::Refused => Self::upstream(
                "The payment could not be processed. Nothing was charged. Please try again later or email hello@marcoferra.com.",
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    pub id: String,
    /// `open`, `complete` or `expired`.
    pub status: String,
    /// `paid`, `unpaid` or `no_payment_required`.
    pub payment_status: String,
    /// The held event's id, as `client_reference_id`.
    pub event: Option<String>,
    pub url: Option<String>,
    pub expires_at: i64,
    pub amount_total: i64,
    pub currency: String,
    pub payment_intent: Option<String>,
    /// How it was paid, once it was: `card`, `mb_way`, `paypal` and so on.
    pub method: Option<String>,
    pub receipt_url: Option<String>,
    /// Minor units already refunded.
    pub refunded: i64,
    /// The address the buyer paid with.
    pub email: Option<String>,
}

impl Session {
    /// Reads a session; its payment intent is either an id or, expanded, the intent itself.
    pub(crate) fn read(object: &Value) -> Option<Self> {
        let text = |pointer: &str| text(object, pointer);
        let intent = object.get("payment_intent");
        let charge = intent.and_then(|intent| intent.pointer("/latest_charge"));
        let of_charge = |pointer: &str| charge.and_then(|charge| self::text(charge, pointer));
        Some(Self {
            id: text("/id")?,
            status: text("/status")?,
            payment_status: text("/payment_status")?,
            event: text("/client_reference_id"),
            url: text("/url"),
            expires_at: object.get("expires_at")?.as_i64()?,
            amount_total: object.get("amount_total").and_then(Value::as_i64)?,
            currency: text("/currency")?,
            payment_intent: intent.and_then(|intent| {
                intent
                    .as_str()
                    .or_else(|| intent.get("id")?.as_str())
                    .map(str::to_owned)
            }),
            method: of_charge("/payment_method_details/type"),
            receipt_url: of_charge("/receipt_url"),
            refunded: charge
                .and_then(|charge| charge.get("amount_refunded")?.as_i64())
                .unwrap_or_default(),
            email: text("/customer_details/email").or_else(|| text("/customer_email")),
        })
    }

    pub(crate) fn paid(&self) -> bool {
        self.payment_status == "paid"
    }
}

pub(crate) struct Payments(Stripe);

impl Payments {
    pub(crate) fn new(http: reqwest::Client, api_base: &str, secret_key: SecretString) -> Self {
        Self(Stripe::new(http, api_base, secret_key))
    }

    fn session_of(reply: &Value) -> Result<Session, StripeError> {
        Session::read(reply).ok_or_else(|| {
            tracing::error!("stripe answered a session this client cannot read");
            StripeError::Refused
        })
    }

    /// Creates a Checkout session; the key makes a repeat answer the same session.
    pub(crate) async fn create_session(
        &self,
        form: &[(String, String)],
        key: &str,
    ) -> Result<Session, StripeError> {
        let path = "/v1/checkout/sessions";
        Self::session_of(&self.0.call(Method::POST, path, form, Some(key)).await?)
    }

    /// Read with its charge: how it was paid, its receipt, and what was refunded.
    pub(crate) async fn session(&self, id: &str) -> Result<Session, StripeError> {
        let id = url_segment(id);
        let path = format!("/v1/checkout/sessions/{id}?expand[]=payment_intent.latest_charge");
        Self::session_of(&self.0.call(Method::GET, &path, NO_FORM, None).await?)
    }

    /// Expires an open session, so it can no longer be paid.
    pub(crate) async fn expire(&self, id: &str) -> Result<Session, StripeError> {
        let path = format!("/v1/checkout/sessions/{}/expire", url_segment(id));
        Self::session_of(&self.0.call(Method::POST, &path, NO_FORM, None).await?)
    }

    /// Refunds what is left of a payment, once per key; answers the minor units refunded.
    pub(crate) async fn refund(&self, payment_intent: &str, key: &str) -> Result<i64, StripeError> {
        let form = [("payment_intent", payment_intent)];
        let reply = self
            .0
            .call(Method::POST, "/v1/refunds", &form, Some(key))
            .await?;
        reply.get("amount").and_then(Value::as_i64).ok_or_else(|| {
            tracing::error!("stripe answered a refund without its amount");
            StripeError::Refused
        })
    }
}

pub(crate) struct Checkout<'a> {
    pub event: &'a str,
    pub slug: &'a str,
    pub email: &'a str,
    /// The line item's name and description.
    pub title: &'a str,
    pub when: &'a str,
    pub price_minor: i64,
    pub currency: &'a str,
    pub methods: &'a [&'a str],
    pub expires_at: i64,
    pub success_url: &'a str,
    pub cancel_url: &'a str,
}

/// The form of `POST /v1/checkout/sessions` for a booking: one priced line, paid by the type's
/// methods, with Stripe's receipt sent to the visitor.
pub(crate) fn checkout_form(checkout: &Checkout<'_>) -> Vec<(String, String)> {
    let pair = |key: &str, value: &str| (key.to_owned(), value.to_owned());
    let mut form = vec![
        pair("mode", "payment"),
        pair("submit_type", "book"),
        pair("client_reference_id", checkout.event),
        pair("customer_email", checkout.email),
        pair("line_items[0][quantity]", "1"),
        pair("line_items[0][price_data][currency]", checkout.currency),
        pair(
            "line_items[0][price_data][unit_amount]",
            &checkout.price_minor.to_string(),
        ),
        pair(
            "line_items[0][price_data][product_data][name]",
            checkout.title,
        ),
        pair(
            "line_items[0][price_data][product_data][description]",
            checkout.when,
        ),
        pair("expires_at", &checkout.expires_at.to_string()),
        pair("success_url", checkout.success_url),
        pair("cancel_url", checkout.cancel_url),
        pair("locale", "auto"),
        pair("allow_promotion_codes", "true"),
        pair("payment_intent_data[receipt_email]", checkout.email),
        pair("metadata[event]", checkout.event),
        pair("metadata[type]", checkout.slug),
        pair("payment_intent_data[metadata][event]", checkout.event),
    ];
    form.extend(
        checkout
            .methods
            .iter()
            .enumerate()
            .map(|(index, method)| pair(&format!("payment_method_types[{index}]"), method)),
    );
    form
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Checkout, Session, checkout_form};
    use crate::TestResult;

    #[test]
    fn a_checkout_form_names_the_booking_its_price_its_methods_and_its_way_back() {
        let form = checkout_form(&Checkout {
            event: "mfabc",
            slug: "focused-session",
            email: "ana@example.com",
            title: "Focused Session",
            when: "Tuesday 6 January 2026, 10:00 to 10:30 (Europe/Lisbon)",
            price_minor: 4000,
            currency: "eur",
            methods: &["card", "mb_way", "paypal"],
            expires_at: 1_767_694_200,
            success_url: "https://www.marcoferra.com/book/focused-session/done/?session_id={CHECKOUT_SESSION_ID}",
            cancel_url: "https://www.marcoferra.com/book/focused-session/?release=mfabc&token=t",
        });
        let pairs: Vec<(&str, &str)> = form
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        for expected in [
            ("mode", "payment"),
            ("client_reference_id", "mfabc"),
            ("customer_email", "ana@example.com"),
            ("line_items[0][quantity]", "1"),
            ("line_items[0][price_data][currency]", "eur"),
            ("line_items[0][price_data][unit_amount]", "4000"),
            (
                "line_items[0][price_data][product_data][name]",
                "Focused Session",
            ),
            ("expires_at", "1767694200"),
            ("locale", "auto"),
            ("allow_promotion_codes", "true"),
            ("payment_intent_data[receipt_email]", "ana@example.com"),
            ("metadata[event]", "mfabc"),
            ("payment_method_types[0]", "card"),
            ("payment_method_types[1]", "mb_way"),
            ("payment_method_types[2]", "paypal"),
        ] {
            assert!(pairs.contains(&expected), "{expected:?} in {pairs:?}");
        }
        let success = pairs.iter().find(|(key, _)| *key == "success_url");
        assert!(success.is_some_and(|(_, url)| url.ends_with("{CHECKOUT_SESSION_ID}")));
        assert!(
            !pairs
                .iter()
                .any(|(key, _)| key.starts_with("automatic_tax"))
        );
    }

    /// A webhook names the intent by id; a read with `expand` hands over the intent and charge.
    #[test]
    fn a_session_is_read_with_or_without_its_charge() -> TestResult {
        let plain = json!({"id": "cs_1", "status": "complete", "payment_status": "paid",
            "client_reference_id": "mfabc", "expires_at": 1, "amount_total": 4000,
            "currency": "eur", "payment_intent": "pi_1"});
        let session = Session::read(&plain).ok_or("unread")?;
        assert_eq!(session.payment_intent.as_deref(), Some("pi_1"));
        assert!(session.paid() && session.method.is_none());
        let expanded = json!({"id": "cs_1", "status": "complete", "payment_status": "paid",
            "expires_at": 1, "amount_total": 4000, "currency": "eur", "url": null,
            "payment_intent": {"id": "pi_1", "latest_charge": {"receipt_url": "https://r",
            "amount_refunded": 4000, "payment_method_details": {"type": "mb_way"}}}});
        let session = Session::read(&expanded).ok_or("unread")?;
        assert_eq!(session.payment_intent.as_deref(), Some("pi_1"));
        assert_eq!(session.method.as_deref(), Some("mb_way"));
        assert_eq!(
            (session.receipt_url.as_deref(), session.refunded),
            (Some("https://r"), 4000)
        );
        Ok(())
    }
}
