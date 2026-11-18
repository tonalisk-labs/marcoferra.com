//! The host's slot grid: wall-clock starts on its working weekdays as UTC instants, which each
//! meeting type's notice, horizon and length then narrow. DST is the zone's to resolve.

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};

use crate::config::{Booking, EventType};

const MINUTES_PER_DAY: i64 = 24 * 60;

pub(crate) struct Grid {
    zone: TimeZone,
    weekdays: Vec<Weekday>,
    /// Minutes after midnight of the first start and of the end every meeting keeps within.
    first: i64,
    last_end: i64,
    every: i64,
}

#[derive(Clone, Copy)]
pub(crate) struct Rules {
    pub duration: SignedDuration,
    /// Kept free on each side of the meeting.
    pub buffer: SignedDuration,
    notice: SignedDuration,
    pub horizon: SignedDuration,
}

impl Rules {
    pub(crate) fn of(kind: &EventType) -> Self {
        Self {
            duration: SignedDuration::from_mins(kind.duration_minutes.into()),
            buffer: SignedDuration::from_mins(kind.buffer_minutes.into()),
            notice: SignedDuration::from_hours(kind.notice_hours.into()),
            horizon: SignedDuration::from_hours(i64::from(kind.horizon_days).saturating_mul(24)),
        }
    }
}

impl Grid {
    pub(crate) fn new(rules: &Booking) -> Result<Self, String> {
        let zone = jiff::tz::db()
            .get(&rules.host_time_zone)
            .map_err(|err| err.to_string())?;
        let weekdays = rules
            .weekdays
            .iter()
            .map(|&day| Weekday::from_monday_one_offset(day));
        let hours = |hour: i8| i64::from(hour).saturating_mul(60);
        if rules.start_every_minutes <= 0 {
            return Err("booking.start_every_minutes must be positive".to_owned());
        }
        Ok(Self {
            zone,
            weekdays: weekdays
                .collect::<Result<_, _>>()
                .map_err(|err| err.to_string())?,
            first: hours(rules.first_start_hour),
            last_end: hours(rules.last_end_hour),
            every: rules.start_every_minutes,
        })
    }

    /// The bookable starts at `now`, earliest first.
    pub(crate) fn starts(&self, now: Timestamp, rules: Rules) -> Vec<Timestamp> {
        let (Ok(earliest), Ok(latest)) = (
            now.checked_add(rules.notice),
            now.checked_add(rules.horizon),
        ) else {
            return Vec::new();
        };
        let today = now.to_zoned(self.zone.clone()).date();
        let last_day = latest.to_zoned(self.zone.clone()).date();
        let days = std::iter::successors(Some(today), |day: &Date| day.tomorrow().ok())
            .take_while(|day| *day <= last_day);
        days.filter(|day| self.weekdays.contains(&day.weekday()))
            .flat_map(|day| self.day_starts(day, rules))
            .filter(|start| (earliest..=latest).contains(start))
            .collect()
    }

    /// Whether `start` is one of `starts(now, rules)`, by arithmetic: in the window, on a working
    /// day's grid minute, at a wall-clock time the zone neither skips nor repeats.
    pub(crate) fn is_start(&self, now: Timestamp, start: Timestamp, rules: Rules) -> bool {
        let (Ok(earliest), Ok(latest)) = (
            now.checked_add(rules.notice),
            now.checked_add(rules.horizon),
        ) else {
            return false;
        };
        if !(earliest..=latest).contains(&start) {
            return false;
        }
        let local = start.to_zoned(self.zone.clone()).datetime();
        let unambiguous = self
            .zone
            .to_ambiguous_zoned(local)
            .unambiguous()
            .is_ok_and(|zoned| zoned.timestamp() == start);
        // A start past midnight belongs to an earlier day when the last end is that late.
        let days_back = self
            .last_end
            .max(0)
            .checked_div(MINUTES_PER_DAY)
            .unwrap_or(0);
        unambiguous
            && (0..=days_back).any(|back| {
                let Ok(day) = local.date().checked_sub(back.days()) else {
                    return false;
                };
                let since = day
                    .to_datetime(jiff::civil::Time::midnight())
                    .duration_until(local);
                self.weekdays.contains(&day.weekday()) && self.on_grid(since, rules)
            })
    }

    /// The minutes after midnight of a meeting's latest start: it ends by the last end.
    const fn last_start(&self, rules: Rules) -> i64 {
        self.last_end.saturating_sub(rules.duration.as_mins())
    }

    /// Whether `since` midnight is a whole grid minute between the first and the latest start.
    fn on_grid(&self, since: SignedDuration, rules: Rules) -> bool {
        let minute = since.as_mins();
        SignedDuration::from_mins(minute) == since
            && (self.first..=self.last_start(rules)).contains(&minute)
            && minute
                .checked_sub(self.first)
                .and_then(|offset| offset.checked_rem(self.every))
                == Some(0)
    }

    /// A day's starts; a wall-clock time the zone skips or repeats is no start at all.
    fn day_starts(&self, day: Date, rules: Rules) -> impl Iterator<Item = Timestamp> + '_ {
        let last = self.last_start(rules);
        let minutes =
            std::iter::successors(Some(self.first), |minute| minute.checked_add(self.every));
        minutes
            .take_while(move |minute| *minute <= last)
            .filter_map(move |minute| {
                let local = day
                    .to_datetime(jiff::civil::Time::midnight())
                    .checked_add(minute.minutes())
                    .ok()?;
                self.zone
                    .to_ambiguous_zoned(local)
                    .unambiguous()
                    .ok()
                    .map(|zoned| zoned.timestamp())
            })
    }
}

/// The starts whose meeting and buffers no busy interval overlaps, a touching edge leaving it
/// free: one sweep, as `starts` ascend and the busy intervals are sorted and merged.
pub(crate) fn free(
    starts: Vec<Timestamp>,
    busy: &[(Timestamp, Timestamp)],
    rules: Rules,
) -> Vec<Timestamp> {
    let merged = merged(busy);
    let mut ahead = merged.iter().peekable();
    starts
        .into_iter()
        .filter(|&start| {
            let (from, to) = span(start, rules);
            while ahead.next_if(|&&(_, end)| end <= from).is_some() {}
            ahead.peek().is_none_or(|&&(begin, _)| begin >= to)
        })
        .collect()
}

/// What a meeting at `start` keeps free: the meeting and its buffer on each side.
pub(crate) fn span(start: Timestamp, rules: Rules) -> (Timestamp, Timestamp) {
    let end = start.checked_add(rules.duration).unwrap_or(start);
    (
        start.checked_sub(rules.buffer).unwrap_or(start),
        end.checked_add(rules.buffer).unwrap_or(end),
    )
}

/// The busy intervals by start, overlapping ones joined; touching ones stay apart, so an end
/// that meets a start still counts as it did. Their ends then ascend too.
fn merged(busy: &[(Timestamp, Timestamp)]) -> Vec<(Timestamp, Timestamp)> {
    let mut sorted = busy.to_vec();
    sorted.sort_unstable();
    let mut joined: Vec<(Timestamp, Timestamp)> = Vec::with_capacity(sorted.len());
    for (from, to) in sorted {
        match joined.last_mut() {
            Some(last) if from < last.1 => last.1 = last.1.max(to),
            _ => joined.push((from, to)),
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use super::{Grid, Rules, free};
    use crate::TestResult;
    use crate::config::Booking;

    /// Hourly starts from 10:00, the last at 16:00 for a meeting of 30 minutes ending by 17:00.
    fn grid() -> Result<Grid, String> {
        grid_of(10, 17, 60, vec![1, 2, 3, 4, 5])
    }

    fn grid_of(first: i8, last_end: i8, every: i64, weekdays: Vec<i8>) -> Result<Grid, String> {
        Grid::new(&Booking {
            host_time_zone: "Europe/Lisbon".to_owned(),
            weekdays,
            first_start_hour: first,
            last_end_hour: last_end,
            start_every_minutes: every,
            hold_minutes: 30,
            refund_until_hours: 24,
            withdrawal_days: 14,
        })
    }

    /// A meeting of `minutes` with `buffer` on each side, 24 hours' notice and 90 days ahead.
    fn rules(minutes: i64, buffer: i64) -> Rules {
        Rules {
            duration: SignedDuration::from_mins(minutes),
            buffer: SignedDuration::from_mins(buffer),
            notice: SignedDuration::from_hours(24),
            horizon: SignedDuration::from_hours(90 * 24),
        }
    }

    fn meeting() -> Rules {
        rules(30, 0)
    }

    fn at(text: &str) -> Result<Timestamp, jiff::Error> {
        text.parse()
    }

    /// Lisbon leaves winter time on 29 March 2026 and returns on 25 October: 10:00 there is
    /// 10:00Z before, 09:00Z between, and 10:00Z again after.
    #[test]
    fn the_grid_follows_the_host_across_both_dst_changes() -> TestResult {
        let grid = grid()?;
        let starts = grid.starts(at("2026-03-25T12:00:00Z")?, meeting());
        assert!(starts.contains(&at("2026-03-27T10:00:00Z")?));
        assert!(starts.contains(&at("2026-03-30T09:00:00Z")?));
        assert!(!starts.contains(&at("2026-03-30T10:30:00Z")?));
        let autumn = grid.starts(at("2026-10-21T12:00:00Z")?, meeting());
        assert!(autumn.contains(&at("2026-10-23T09:00:00Z")?));
        assert!(autumn.contains(&at("2026-10-26T10:00:00Z")?));
        assert!(autumn.contains(&at("2026-10-26T16:00:00Z")?));
        Ok(())
    }

    #[test]
    fn the_grid_keeps_to_weekdays_the_lead_time_and_the_window() -> TestResult {
        let grid = grid()?;
        let now = at("2026-01-05T10:00:00Z")?;
        let starts = grid.starts(now, meeting());
        let first = starts.first().ok_or("no start")?;
        assert_eq!(*first, at("2026-01-06T10:00:00Z")?);
        assert!(!starts.contains(&at("2026-01-10T10:00:00Z")?), "a Saturday");
        let last = starts.last().ok_or("no start")?;
        assert!(*last <= now.checked_add(meeting().horizon)?);
        assert_eq!(
            starts
                .iter()
                .filter(|start| start.to_string().starts_with("2026-01-07"))
                .count(),
            7
        );
        Ok(())
    }

    #[test]
    fn a_busy_interval_takes_the_starts_it_overlaps_and_no_other() -> TestResult {
        let starts = vec![at("2026-01-06T10:00:00Z")?, at("2026-01-06T11:00:00Z")?];
        let touching = [(at("2026-01-06T10:30:00Z")?, at("2026-01-06T11:00:00Z")?)];
        assert_eq!(free(starts.clone(), &touching, meeting()).len(), 2);
        let overlapping = [(at("2026-01-06T10:29:00Z")?, at("2026-01-06T10:31:00Z")?)];
        assert_eq!(
            free(starts, &overlapping, meeting()),
            vec![at("2026-01-06T11:00:00Z")?]
        );
        Ok(())
    }

    #[test]
    fn a_buffer_keeps_its_minutes_free_before_and_after() -> TestResult {
        let starts = vec![
            at("2026-01-06T10:00:00Z")?,
            at("2026-01-06T11:00:00Z")?,
            at("2026-01-06T12:00:00Z")?,
        ];
        let busy = [(at("2026-01-06T10:40:00Z")?, at("2026-01-06T10:50:00Z")?)];
        assert_eq!(free(starts.clone(), &busy, rules(30, 10)).len(), 3);
        assert_eq!(
            free(starts.clone(), &busy, rules(30, 15)),
            vec![at("2026-01-06T12:00:00Z")?]
        );
        let after = [(at("2026-01-06T11:40:00Z")?, at("2026-01-06T11:45:00Z")?)];
        assert_eq!(
            free(starts, &after, rules(30, 15)),
            vec![at("2026-01-06T10:00:00Z")?, at("2026-01-06T12:00:00Z")?]
        );
        Ok(())
    }

    /// The committed grid: starts every half hour from 10:00, each meeting ending by 17:00.
    #[test]
    fn every_meeting_ends_by_the_last_end() -> TestResult {
        let grid = grid_of(10, 17, 30, vec![1, 2, 3, 4, 5])?;
        let now = at("2026-01-05T10:00:00Z")?;
        let on = |starts: &[Timestamp]| -> Vec<String> {
            starts
                .iter()
                .map(ToString::to_string)
                .filter(|start| start.starts_with("2026-01-07"))
                .collect()
        };
        let short = on(&grid.starts(now, meeting()));
        assert_eq!(short.len(), 14);
        assert_eq!(
            short.last().map(String::as_str),
            Some("2026-01-07T16:30:00Z")
        );
        let long = on(&grid.starts(now, rules(90, 0)));
        assert_eq!(
            long.last().map(String::as_str),
            Some("2026-01-07T15:30:00Z")
        );
        assert!(!grid.is_start(now, at("2026-01-07T16:00:00Z")?, rules(90, 0)));
        assert!(grid.is_start(now, at("2026-01-07T16:00:00Z")?, meeting()));
        Ok(())
    }

    #[test]
    fn a_start_is_known_by_arithmetic_exactly_when_the_grid_lists_it() -> TestResult {
        // The usual grid, an odd step, and starts every day running past midnight, through the
        // hour Lisbon skips in March and the one it repeats in October.
        let every_day = vec![1, 2, 3, 4, 5, 6, 7];
        for grid in [
            grid()?,
            grid_of(9, 18, 45, vec![1, 3, 5])?,
            grid_of(22, 27, 30, every_day)?,
        ] {
            for rules in [meeting(), rules(60, 15)] {
                known_by_arithmetic(&grid, rules)?;
            }
        }
        Ok(())
    }

    fn known_by_arithmetic(grid: &Grid, rules: Rules) -> TestResult {
        let steps = [-60, -30, -1, 0, 1, 30, 60];
        for now in [
            "2026-01-05T10:00:00Z",
            "2026-03-25T09:59:00Z",
            "2026-10-21T12:00:00Z",
            "2026-10-24T00:30:00Z",
            "2026-12-30T23:30:00Z",
        ] {
            let now = at(now)?;
            let starts = grid.starts(now, rules);
            let mut candidates = vec![
                now,
                now.checked_add(SignedDuration::from_hours(24))?,
                now.checked_add(rules.horizon)?,
            ];
            for start in &starts {
                for step in steps {
                    candidates.push(start.checked_add(SignedDuration::from_mins(step))?);
                }
                candidates.push(start.checked_add(SignedDuration::from_secs(1))?);
                candidates.push(start.checked_add(SignedDuration::from_hours(24 * 5))?);
            }
            for minute in (0..(24 * 95 * 60)).step_by(17) {
                candidates.push(now.checked_add(SignedDuration::from_mins(minute))?);
            }
            for candidate in candidates {
                assert_eq!(
                    grid.is_start(now, candidate, rules),
                    starts.contains(&candidate),
                    "{candidate} from {now}"
                );
            }
        }
        Ok(())
    }

    /// The plain definition, as a reference: every start, with its buffers, against every interval.
    fn free_by_every_pair(
        starts: &[Timestamp],
        busy: &[(Timestamp, Timestamp)],
        rules: Rules,
    ) -> Vec<Timestamp> {
        starts
            .iter()
            .copied()
            .filter(|&start| {
                let (begin, end) = super::span(start, rules);
                !busy.iter().any(|&(from, to)| begin < to && end > from)
            })
            .collect()
    }

    #[test]
    fn the_sweep_frees_exactly_the_starts_every_pair_would() -> TestResult {
        let grid = grid()?;
        let now = at("2026-01-05T10:00:00Z")?;
        let starts = grid.starts(now, meeting());
        let mut seed: i64 = 7;
        let mut next = |range: i64| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7FFF_FFFF;
            seed.checked_rem(range).unwrap_or(0)
        };
        for (minutes, buffer) in [
            (0, 0),
            (1, 0),
            (30, 0),
            (60, 0),
            (90, 0),
            (30, 10),
            (60, 25),
        ] {
            let rules = rules(minutes, buffer);
            let meeting = rules.duration;
            // Unsorted, overlapping, nested, touching and empty intervals over the window.
            let mut busy = Vec::new();
            for _ in 0..300 {
                let from = now.checked_add(SignedDuration::from_mins(next(92 * 24 * 60)))?;
                let length = match next(4) {
                    0 => 0,
                    1 => 30,
                    2 => 60,
                    _ => next(600),
                };
                busy.push((from, from.checked_add(SignedDuration::from_mins(length))?));
            }
            let (first, _) = busy.first().copied().ok_or("no interval")?;
            busy.push((first, first));
            busy.push((first.checked_add(meeting)?, first.checked_add(meeting)?));
            let expected = free_by_every_pair(&starts, &busy, rules);
            assert!(!expected.is_empty() && expected.len() < starts.len());
            assert_eq!(free(starts.clone(), &busy, rules), expected);
        }
        Ok(())
    }

    #[test]
    fn a_full_window_against_a_busy_calendar_is_one_sweep() -> TestResult {
        let grid = grid()?;
        let now = at("2026-01-05T10:00:00Z")?;
        let starts = grid.starts(now, meeting());
        let busy: Vec<_> = (0..100_000)
            .map(|minute| {
                let from = now.checked_add(SignedDuration::from_mins(minute * 3))?;
                Ok((from, from.checked_add(SignedDuration::from_mins(1))?))
            })
            .collect::<Result<_, jiff::Error>>()?;
        let started = std::time::Instant::now();
        let free = free(starts, &busy, meeting());
        assert_eq!(free, Vec::new());
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        Ok(())
    }
}
