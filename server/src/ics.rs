//! The iMIP calendar (RFC 5545, RFC 6047), written by `icalendar`: TEXT escaped, lines folded at
//! 75 octets and ended by CRLF, every time in UTC. One builder serves the email and the download,
//! so both carry the same bytes.

use icalendar::{Calendar, Component, Event, Property};
use jiff::Timestamp;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Method {
    Request,
    Cancel,
}

impl Method {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Request => "REQUEST",
            Self::Cancel => "CANCEL",
        }
    }
}

pub(crate) struct Party<'a> {
    pub name: &'a str,
    pub email: &'a str,
}

pub(crate) struct Invite<'a> {
    pub method: Method,
    pub uid: &'a str,
    pub sequence: u32,
    pub stamp: Timestamp,
    pub start: Timestamp,
    pub end: Timestamp,
    pub summary: &'a str,
    pub description: &'a str,
    pub video: Option<&'a str>,
    pub organizer: Party<'a>,
    pub attendee: Party<'a>,
}

/// TEXT with one kind of line break: `icalendar` escapes LF, and a CR would be left bare.
fn text(value: &str) -> String {
    value.replace("\r\n", "\n")
}

/// A parameter value without DQUOTE or control characters, which one may not hold (§3.1).
fn parameter(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '"' && !c.is_control())
        .collect()
}

fn utc(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

/// Someone taking part, by name and address.
fn party(key: &str, who: &Party<'_>, extra: &[(&str, &str)]) -> Property {
    let mut property = Property::new(key, format!("mailto:{}", who.email));
    property.add_parameter("CN", &parameter(who.name));
    for (name, value) in extra {
        property.add_parameter(name, value);
    }
    property.done()
}

pub(crate) fn build(invite: &Invite<'_>) -> String {
    let cancelled = invite.method == Method::Cancel;
    let mut event = Event::new();
    event
        .uid(invite.uid)
        .add_property("SEQUENCE", invite.sequence.to_string())
        .add_property("DTSTAMP", utc(invite.stamp))
        .add_property("DTSTART", utc(invite.start))
        .add_property("DTEND", utc(invite.end))
        .add_property("SUMMARY", text(invite.summary))
        .add_property("DESCRIPTION", text(invite.description))
        .add_property("STATUS", if cancelled { "CANCELLED" } else { "CONFIRMED" })
        .add_property("TRANSP", "OPAQUE")
        .append_property(party("ORGANIZER", &invite.organizer, &[]));
    if let Some(video) = invite.video {
        event
            .add_property("LOCATION", text(video))
            .add_property("URL", video);
    }
    let reply: &[(&str, &str)] = if cancelled {
        &[("ROLE", "REQ-PARTICIPANT")]
    } else {
        &[
            ("ROLE", "REQ-PARTICIPANT"),
            ("PARTSTAT", "NEEDS-ACTION"),
            ("RSVP", "TRUE"),
        ]
    };
    event.append_multi_property(party("ATTENDEE", &invite.attendee, reply));
    let mut calendar = Calendar::empty();
    calendar
        .append_property(("VERSION", "2.0"))
        .append_property(("PRODID", "-//marcoferra.com//booking//EN"))
        .append_property(("CALSCALE", "GREGORIAN"))
        .append_property(("METHOD", invite.method.name()))
        .push(event.done());
    calendar.to_string()
}

#[cfg(test)]
mod tests {
    use icalendar::parser::{read_calendar, unfold as read_unfolded};

    use super::{Invite, Method, Party, build};
    use crate::TestResult;

    fn invite(method: Method) -> Result<Invite<'static>, jiff::Error> {
        Ok(Invite {
            method,
            uid: "mfabc@google.com",
            sequence: 0,
            stamp: "2026-01-05T09:00:01.250Z".parse()?,
            start: "2026-01-06T10:00:00Z".parse()?,
            end: "2026-01-06T10:30:00Z".parse()?,
            summary: "Meeting with Marco Ferra",
            description: "Video call: https://meet.google.com/abc-defg-hij\nNotes; with, commas \\ and a long line that has to be folded because it passes seventy-five octets: ação",
            video: Some("https://meet.google.com/abc-defg-hij"),
            organizer: Party {
                name: "Marco Ferra",
                email: "hello@marcoferra.com",
            },
            attendee: Party {
                name: "Ana \"Nana\" Silva",
                email: "ana@example.com",
            },
        })
    }

    /// Unfolds as a reader does: a CRLF followed by one space joins the lines.
    fn unfold(ics: &str) -> String {
        ics.replace("\r\n ", "")
    }

    #[test]
    fn a_request_is_crlf_folded_escaped_and_utc() -> TestResult {
        let ics = build(&invite(Method::Request)?);
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert_eq!(
            ics.matches('\n').count(),
            ics.matches("\r\n").count(),
            "bare LF"
        );
        assert!(
            ics.split("\r\n").all(|physical| physical.len() <= 75),
            "{ics}"
        );
        let unfolded = unfold(&ics);
        for expected in [
            "METHOD:REQUEST",
            "UID:mfabc@google.com",
            "SEQUENCE:0",
            "DTSTAMP:20260105T090001Z",
            "DTSTART:20260106T100000Z",
            "DTEND:20260106T103000Z",
            "DESCRIPTION:Video call: https://meet.google.com/abc-defg-hij\\nNotes\\; with\\, commas \\\\ and",
            "ORGANIZER;CN=Marco Ferra:mailto:hello@marcoferra.com",
            "ATTENDEE;CN=Ana Nana Silva;PARTSTAT=NEEDS-ACTION;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:ana@example.com",
            "STATUS:CONFIRMED",
        ] {
            assert!(
                unfolded.contains(expected),
                "{expected} missing from\n{unfolded}"
            );
        }
        assert!(
            unfolded.contains("octets: ação\r\n"),
            "the text survives its folds"
        );
        Ok(())
    }

    #[test]
    fn a_cancel_names_the_same_event_with_its_method_and_status() -> TestResult {
        let mut cancel = invite(Method::Cancel)?;
        cancel.sequence = 1;
        let unfolded = unfold(&build(&cancel));
        for expected in [
            "METHOD:CANCEL",
            "UID:mfabc@google.com",
            "SEQUENCE:1",
            "STATUS:CANCELLED",
        ] {
            assert!(unfolded.contains(expected), "{expected}");
        }
        assert!(!unfolded.contains("RSVP=TRUE"));
        Ok(())
    }

    /// What a calendar reader takes back: every value as written, a name that must be quoted
    /// quoted, and a CRLF in the notes one line break.
    #[test]
    fn a_reader_takes_back_each_value_as_written() -> TestResult {
        let mut request = invite(Method::Request)?;
        request.attendee.name = "Doe, John";
        request.description = "Two\r\nlines";
        let unfolded = read_unfolded(&build(&request));
        let calendar = read_calendar(&unfolded)?;
        let method = calendar.properties.iter().find(|p| p.name == "METHOD");
        assert_eq!(method.map(|p| p.val.as_str()), Some("REQUEST"));
        let event = calendar.components.first().ok_or("no event")?;
        let value = |name: &str| {
            event
                .properties
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.val.as_str().to_owned())
        };
        assert_eq!(value("DESCRIPTION").as_deref(), Some("Two\nlines"));
        assert_eq!(
            value("URL").as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        let attendee = event
            .properties
            .iter()
            .find(|p| p.name == "ATTENDEE")
            .ok_or("no attendee")?;
        assert_eq!(attendee.val.as_str(), "mailto:ana@example.com");
        let cn = attendee.params.iter().find(|q| q.key == "CN");
        let cn = cn
            .and_then(|q| q.val.as_ref())
            .map(|v| v.as_str().to_owned());
        assert_eq!(cn.as_deref(), Some("Doe, John"));
        assert!(
            unfolded.contains("ATTENDEE;CN=\"Doe, John\";"),
            "{unfolded}"
        );
        Ok(())
    }
}
