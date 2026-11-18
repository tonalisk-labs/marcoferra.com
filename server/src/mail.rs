//! Invitations over SMTP: `multipart/mixed` of an alternative (text, HTML, then the calendar mail
//! clients show as a card) and `invite.ics`; a letter without a calendar is the alternative alone.

use std::time::Duration;

use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::config::{Email, Secrets};
use crate::ics::Method;

/// Well inside a Lambda's timeout, so a stalled relay still leaves time to compensate.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Letter<'a> {
    pub to: Mailbox,
    pub subject: &'a str,
    pub text: &'a str,
    pub html: &'a str,
    /// The invitation and its method.
    pub calendar: Option<(&'a str, Method)>,
}

pub(crate) struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    pub from: Mailbox,
}

impl Mailer {
    /// Implicit TLS on port 465, STARTTLS otherwise; the relay is never spoken to in clear.
    pub(crate) fn new(email: &Email, secrets: &Secrets) -> Result<Self, String> {
        let builder = if email.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&email.host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&email.host)
        }
        .map_err(|err| format!("invalid SMTP configuration: {err}"))?;
        let credentials = Credentials::new(
            secrets.smtp_username.expose_secret().to_owned(),
            secrets.smtp_password.expose_secret().to_owned(),
        );
        let transport = builder
            .port(email.port)
            .credentials(credentials)
            .timeout(Some(SEND_TIMEOUT))
            .build();
        let from = email
            .from
            .parse()
            .map_err(|err| format!("email.from is not a mailbox: {err}"))?;
        Ok(Self { transport, from })
    }

    /// Resolves once the relay has accepted the message.
    pub(crate) async fn send(&self, letter: &Letter<'_>) -> Result<(), String> {
        let message = compose(&self.from, letter)?;
        self.transport
            .send(message)
            .await
            .map(drop)
            .map_err(|err| err.to_string())
    }
}

pub(crate) fn compose(from: &Mailbox, letter: &Letter<'_>) -> Result<Message, String> {
    let alternative = MultiPart::alternative()
        .singlepart(SinglePart::plain(letter.text.to_owned()))
        .singlepart(SinglePart::html(letter.html.to_owned()));
    // Our own Message-ID under the sending domain, so it agrees with From (RFC 5322 §3.6.4).
    let message_id = format!("<{}@{}>", Uuid::now_v7(), from.email.domain());
    // Machine-sent: a vacation responder must not answer it (RFC 3834 §5).
    let automatic = HeaderValue::new(
        HeaderName::new_from_ascii_str("Auto-Submitted"),
        "auto-generated".to_owned(),
    );
    let builder = Message::builder()
        .from(from.clone())
        .to(letter.to.clone())
        .subject(letter.subject)
        .message_id(Some(message_id))
        .raw_header(automatic);
    let Some((ics, method)) = letter.calendar else {
        return builder
            .multipart(alternative)
            .map_err(|err| err.to_string());
    };
    let calendar = format!("text/calendar; charset=UTF-8; method={}", method.name());
    let calendar = ContentType::parse(&calendar).map_err(|err| err.to_string())?;
    let card = SinglePart::builder()
        .header(calendar.clone())
        .body(ics.to_owned());
    let attachment = Attachment::new("invite.ics".to_owned()).body(ics.to_owned(), calendar);
    builder
        .multipart(
            MultiPart::mixed()
                .multipart(alternative.singlepart(card))
                .singlepart(attachment),
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Letter, compose};
    use crate::TestResult;
    use crate::ics::Method;

    #[test]
    fn an_invitation_is_one_card_one_attachment_under_the_sending_domain() -> TestResult {
        let letter = Letter {
            to: "Ana <ana@example.com>".parse()?,
            subject: "Invitation: Meeting with Marco Ferra",
            text: "Booked.",
            html: "<p>Booked.</p>",
            calendar: Some((
                "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nEND:VCALENDAR\r\n",
                Method::Request,
            )),
        };
        let raw = String::from_utf8(
            compose(&"Marco Ferra <hello@marcoferra.com>".parse()?, &letter)?.formatted(),
        )?;
        let order = [
            "multipart/mixed",
            "multipart/alternative",
            "text/plain",
            "text/html",
            "method=REQUEST",
            "invite.ics",
        ];
        let found: Vec<_> = order.iter().filter_map(|part| raw.find(part)).collect();
        assert!(found.len() == order.len() && found.is_sorted(), "{raw}");
        assert_eq!(raw.matches("Content-Disposition: attachment").count(), 1);
        let id = raw
            .lines()
            .find(|line| line.starts_with("Message-ID:"))
            .ok_or("no Message-ID")?;
        assert!(id.ends_with("@marcoferra.com>"), "{id}");
        assert!(raw.contains("Auto-Submitted: auto-generated"));
        let plain = Letter {
            calendar: None,
            ..letter
        };
        let raw = String::from_utf8(
            compose(&"Marco Ferra <hello@marcoferra.com>".parse()?, &plain)?.formatted(),
        )?;
        assert!(raw.contains("multipart/alternative") && !raw.contains("multipart/mixed"));
        assert!(!raw.contains("text/calendar"));
        Ok(())
    }
}
