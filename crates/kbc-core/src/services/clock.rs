//! 現在時刻を利用するCommandへ注入するClockを定義する。

use chrono::{DateTime, Utc};

pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
