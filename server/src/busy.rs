//! The calendar's busy intervals as last read, reused while fresh. Expiry is checked on each
//! read; a write by this instance forgets them, and a read begun before a write is never kept.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::Timestamp;

pub(crate) type Busy = Arc<Vec<(Timestamp, Timestamp)>>;

struct Entry {
    read: Instant,
    from: Timestamp,
    until: Timestamp,
    busy: Busy,
}

#[derive(Default)]
pub(crate) struct BusyCache {
    entry: Option<Entry>,
    /// Counts the writes forgotten so far.
    generation: u64,
}

/// What a read asked for: it is kept only if no write was forgotten since.
#[derive(Clone, Copy)]
pub(crate) struct Ticket {
    generation: u64,
    started: Instant,
}

impl BusyCache {
    /// The intervals from `from` to `until`, if a read covering them is younger than `lifetime`.
    pub(crate) fn fresh(
        &self,
        now: Instant,
        lifetime: Duration,
        (from, until): (Timestamp, Timestamp),
    ) -> Option<Busy> {
        let entry = self.entry.as_ref()?;
        let young = now.saturating_duration_since(entry.read) < lifetime;
        let covers = entry.from <= from && until <= entry.until;
        (young && covers).then(|| Arc::clone(&entry.busy))
    }

    /// Taken before asking the calendar.
    pub(crate) const fn ticket(&self, started: Instant) -> Ticket {
        Ticket {
            generation: self.generation,
            started,
        }
    }

    /// Dated from when the read was asked, not when it answered.
    pub(crate) fn store(
        &mut self,
        ticket: Ticket,
        (from, until): (Timestamp, Timestamp),
        busy: Busy,
    ) {
        if ticket.generation == self.generation {
            self.entry = Some(Entry {
                read: ticket.started,
                from,
                until,
                busy,
            });
        }
    }

    pub(crate) fn forget(&mut self) {
        self.entry = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use jiff::{SignedDuration, Timestamp};

    use super::BusyCache;
    use crate::TestResult;

    const LIFETIME: Duration = Duration::from_secs(60);

    fn window() -> Result<(Timestamp, Timestamp), jiff::Error> {
        let from: Timestamp = "2026-01-05T09:00:00Z".parse()?;
        Ok((from, from.checked_add(SignedDuration::from_hours(24))?))
    }

    #[test]
    fn a_read_is_reused_until_its_lifetime_ends() -> TestResult {
        let (start, window) = (std::time::Instant::now(), window()?);
        let mut cache = BusyCache::default();
        assert!(cache.fresh(start, LIFETIME, window).is_none());
        let busy = Arc::new(vec![window]);
        cache.store(cache.ticket(start), window, Arc::clone(&busy));
        let almost = LIFETIME
            .checked_sub(Duration::from_millis(1))
            .ok_or("time")?;
        let almost = start.checked_add(almost).ok_or("time")?;
        assert_eq!(cache.fresh(almost, LIFETIME, window), Some(busy));
        let expired = start.checked_add(LIFETIME).ok_or("time")?;
        assert!(cache.fresh(expired, LIFETIME, window).is_none());
        Ok(())
    }

    #[test]
    fn a_read_serves_only_the_window_it_covers() -> TestResult {
        let (start, (from, until)) = (std::time::Instant::now(), window()?);
        let mut cache = BusyCache::default();
        cache.store(cache.ticket(start), (from, until), Arc::new(Vec::new()));
        let later = until.checked_add(SignedDuration::from_secs(1))?;
        let earlier = from.checked_sub(SignedDuration::from_secs(1))?;
        assert!(cache.fresh(start, LIFETIME, (from, until)).is_some());
        assert!(cache.fresh(start, LIFETIME, (from, later)).is_none());
        assert!(cache.fresh(start, LIFETIME, (earlier, until)).is_none());
        Ok(())
    }

    #[test]
    fn a_write_forgets_and_outdates_reads_in_flight() -> TestResult {
        let (start, window) = (std::time::Instant::now(), window()?);
        let mut cache = BusyCache::default();
        cache.store(cache.ticket(start), window, Arc::new(Vec::new()));
        let in_flight = cache.ticket(start);
        cache.forget();
        assert!(cache.fresh(start, LIFETIME, window).is_none());
        cache.store(in_flight, window, Arc::new(Vec::new()));
        assert!(cache.fresh(start, LIFETIME, window).is_none());
        cache.store(cache.ticket(start), window, Arc::new(Vec::new()));
        assert!(cache.fresh(start, LIFETIME, window).is_some());
        Ok(())
    }
}
