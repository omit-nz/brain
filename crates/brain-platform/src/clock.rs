use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

/// The only way brain reads the time, so review dates, clocks and leases are testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
    fn today(&self) -> Date;
}

pub struct SystemClock;

#[allow(clippy::disallowed_methods)]
impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }

    fn today(&self) -> Date {
        Timestamp::now().to_zoned(TimeZone::system()).date()
    }
}

/// A clock stopped at one instant, read in UTC.
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }

    fn today(&self) -> Date {
        self.0.to_zoned(TimeZone::UTC).date()
    }
}
