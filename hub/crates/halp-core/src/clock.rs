use chrono::{DateTime, Local, NaiveDate, Utc};

/// Time source, injectable for tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// The user's local calendar date (for due dates and the agenda).
    fn today(&self) -> NaiveDate;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
    fn today(&self) -> NaiveDate {
        Local::now().date_naive()
    }
}

/// A clock frozen at one instant.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    pub now: DateTime<Utc>,
    pub today: NaiveDate,
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
    fn today(&self) -> NaiveDate {
        self.today
    }
}
