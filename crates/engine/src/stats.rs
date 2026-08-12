use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DayKey {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl DayKey {
    pub const fn new(
        year: u16,
        month: u8,
        day: u8,
    ) -> Self {
        Self {
            year,
            month,
            day,
        }
    }

    fn previous_day(self) -> Option<Self> {
        let days_in_month = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,

            4 | 6 | 9 | 11 => 30,

            2 => {
                if is_leap_year(self.year) {
                    29
                } else {
                    28
                }
            }

            _ => return None,
        };

        if self.day > 1 {
            return Some(Self {
                year: self.year,
                month: self.month,
                day: self.day - 1,
            });
        }

        if self.month > 1 {
            let previous_month = self.month - 1;

            let previous_month_days =
                match previous_month {
                    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,

                    4 | 6 | 9 | 11 => 30,

                    2 => {
                        if is_leap_year(self.year) {
                            29
                        } else {
                            28
                        }
                    }

                    _ => return None,
                };

            return Some(Self {
                year: self.year,
                month: previous_month,
                day: previous_month_days,
            });
        }

        if self.year == 0 {
            return None;
        }

        Some(Self {
            year: self.year - 1,
            month: 12,
            day: 31,
        })
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0
        && (year % 100 != 0
            || year % 400 == 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DayStats {
    pub words: u64,
    pub dictations: u64,
}

#[derive(Debug, Default)]
pub struct DictationStats {
    total_words: u64,
    total_dictations: u64,
    total_recording_ms: u64,
    daily_activity: HashMap<DayKey, DayStats>,
}

impl DictationStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(
        &mut self,
        day: DayKey,
        words: u32,
        recording_ms: u64,
    ) {
        self.total_words =
            self.total_words
                .saturating_add(words as u64);

        self.total_dictations =
            self.total_dictations
                .saturating_add(1);

        self.total_recording_ms =
            self.total_recording_ms
                .saturating_add(recording_ms);

        let day_stats =
            self.daily_activity
                .entry(day)
                .or_default();

        day_stats.words =
            day_stats
                .words
                .saturating_add(words as u64);

        day_stats.dictations =
            day_stats
                .dictations
                .saturating_add(1);
    }

    pub fn total_words(&self) -> u64 {
        self.total_words
    }

    pub fn total_dictations(&self) -> u64 {
        self.total_dictations
    }

    pub fn total_recording_ms(&self) -> u64 {
        self.total_recording_ms
    }

    pub fn today(
        &self,
        day: DayKey,
    ) -> DayStats {
        self.daily_activity
            .get(&day)
            .copied()
            .unwrap_or_default()
    }

    pub fn current_streak(
        &self,
        today: DayKey,
    ) -> u32 {
        if self.today(today).dictations == 0 {
            return 0;
        }

        let mut streak = 1;
        let mut current_day = today;

        loop {
            let Some(previous_day) =
                current_day.previous_day()
            else {
                break;
            };

            if self
                .daily_activity
                .get(&previous_day)
                .is_some_and(|stats| {
                    stats.dictations > 0
                })
            {
                streak += 1;
                current_day = previous_day;
            } else {
                break;
            }
        }

        streak
    }

    pub fn wpm(
        words: u32,
        recording_ms: u64,
    ) -> f32 {
        if recording_ms == 0 {
            return 0.0;
        }

        words as f32
            / (recording_ms as f32 / 60_000.0)
    }

    pub fn from_parts(
        total_words: u64,
        total_dictations: u64,
        total_recording_ms: u64,
        daily_activity: HashMap<DayKey, DayStats>,
    ) -> Self {
        Self {
            total_words,
            total_dictations,
            total_recording_ms,
            daily_activity,
        }
    }

    pub fn daily_activity(
        &self,
    ) -> &HashMap<DayKey, DayStats> {
        &self.daily_activity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_updates_total_stats() {
        let mut stats =
            DictationStats::new();

        let day =
            DayKey::new(2026, 8, 12);

        stats.record(
            day,
            100,
            60_000,
        );

        assert_eq!(
            stats.total_words(),
            100
        );

        assert_eq!(
            stats.total_dictations(),
            1
        );

        assert_eq!(
            stats.total_recording_ms(),
            60_000
        );
    }

    #[test]
    fn record_updates_daily_stats() {
        let mut stats =
            DictationStats::new();

        let day =
            DayKey::new(2026, 8, 12);

        stats.record(
            day,
            42,
            30_000,
        );

        assert_eq!(
            stats.today(day),
            DayStats {
                words: 42,
                dictations: 1,
            }
        );
    }

    #[test]
    fn multiple_dictations_accumulate() {
        let mut stats =
            DictationStats::new();

        let day =
            DayKey::new(2026, 8, 12);

        stats.record(
            day,
            50,
            30_000,
        );

        stats.record(
            day,
            25,
            20_000,
        );

        assert_eq!(
            stats.total_words(),
            75
        );

        assert_eq!(
            stats.total_dictations(),
            2
        );

        assert_eq!(
            stats.total_recording_ms(),
            50_000
        );

        assert_eq!(
            stats.today(day),
            DayStats {
                words: 75,
                dictations: 2,
            }
        );
    }

    #[test]
    fn wpm_uses_recording_time() {
        let wpm =
            DictationStats::wpm(
                60,
                60_000,
            );

        assert_eq!(wpm, 60.0);
    }

    #[test]
    fn wpm_handles_zero_recording_time() {
        assert_eq!(
            DictationStats::wpm(
                100,
                0,
            ),
            0.0
        );
    }

    #[test]
    fn consecutive_days_increment_streak() {
        let mut stats =
            DictationStats::new();

        let day_one =
            DayKey::new(2026, 8, 10);

        let day_two =
            DayKey::new(2026, 8, 11);

        let day_three =
            DayKey::new(2026, 8, 12);

        stats.record(
            day_one,
            20,
            20_000,
        );

        stats.record(
            day_two,
            30,
            30_000,
        );

        stats.record(
            day_three,
            40,
            40_000,
        );

        assert_eq!(
            stats.current_streak(day_three),
            3
        );
    }

    #[test]
    fn one_gap_day_resets_streak() {
        let mut stats =
            DictationStats::new();

        let day_one =
            DayKey::new(2026, 8, 10);

        let day_three =
            DayKey::new(2026, 8, 12);

        stats.record(
            day_one,
            20,
            20_000,
        );

        stats.record(
            day_three,
            40,
            40_000,
        );

        assert_eq!(
            stats.current_streak(day_three),
            1
        );
    }

    #[test]
    fn no_activity_today_means_zero_streak() {
        let mut stats =
            DictationStats::new();

        let yesterday =
            DayKey::new(2026, 8, 11);

        let today =
            DayKey::new(2026, 8, 12);

        stats.record(
            yesterday,
            20,
            20_000,
        );

        assert_eq!(
            stats.current_streak(today),
            0
        );
    }

    #[test]
    fn month_boundary_works() {
        let mut stats =
            DictationStats::new();

        let previous =
            DayKey::new(2026, 7, 31);

        let current =
            DayKey::new(2026, 8, 1);

        stats.record(
            previous,
            20,
            20_000,
        );

        stats.record(
            current,
            30,
            30_000,
        );

        assert_eq!(
            stats.current_streak(current),
            2
        );
    }

    #[test]
    fn year_boundary_works() {
        let mut stats =
            DictationStats::new();

        let previous =
            DayKey::new(2025, 12, 31);

        let current =
            DayKey::new(2026, 1, 1);

        stats.record(
            previous,
            20,
            20_000,
        );

        stats.record(
            current,
            30,
            30_000,
        );

        assert_eq!(
            stats.current_streak(current),
            2
        );
    }
}