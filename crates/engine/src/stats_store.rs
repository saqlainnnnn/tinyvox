use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::stats::{
    DayKey,
    DayStats,
    DictationStats,
};

#[derive(Debug)]
pub enum StatsStoreError {
    Io(io::Error),
    Serialization(serde_json::Error),
}

impl std::fmt::Display for StatsStoreError {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Io(error) => {
                write!(
                    f,
                    "stats I/O error: {error}"
                )
            }

            Self::Serialization(error) => {
                write!(
                    f,
                    "stats serialization error: {error}"
                )
            }
        }
    }
}

impl std::error::Error for StatsStoreError {}

impl From<io::Error> for StatsStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for StatsStoreError {
    fn from(
        error: serde_json::Error,
    ) -> Self {
        Self::Serialization(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredStats {
    total_words: u64,
    total_dictations: u64,
    total_recording_ms: u64,
    daily_activity: Vec<StoredDayStats>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDayStats {
    day: StoredDayKey,
    words: u64,
    dictations: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDayKey {
    year: u16,
    month: u8,
    day: u8,
}

pub struct StatsStore {
    path: PathBuf,
}

impl StatsStore {
    pub fn new(
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            path: path.into(),
        }
    }

    pub fn load(
        &self,
    ) -> Result<DictationStats, StatsStoreError> {
        if !self.path.exists() {
            return Ok(DictationStats::new());
        }

        let contents =
            fs::read_to_string(&self.path)?;

        let stored: StoredStats =
            serde_json::from_str(&contents)?;

        let daily_activity = stored
            .daily_activity
            .into_iter()
            .map(|entry| {
                (
                    DayKey::new(
                        entry.day.year,
                        entry.day.month,
                        entry.day.day,
                    ),
                    DayStats {
                        words: entry.words,
                        dictations: entry.dictations,
                    },
                )
            })
            .collect();

        Ok(DictationStats::from_parts(
            stored.total_words,
            stored.total_dictations,
            stored.total_recording_ms,
            daily_activity,
        ))
    }

    pub fn save(
        &self,
        stats: &DictationStats,
    ) -> Result<(), StatsStoreError> {
        if let Some(parent) =
            self.path.parent()
        {
            fs::create_dir_all(parent)?;
        }

        let stored =
            StoredStats {
                total_words: stats.total_words(),
                total_dictations:
                    stats.total_dictations(),
                total_recording_ms:
                    stats.total_recording_ms(),
                daily_activity: stats
                    .daily_activity()
                    .iter()
                    .map(
                        |(day, day_stats)| {
                            StoredDayStats {
                                day: StoredDayKey {
                                    year: day.year,
                                    month: day.month,
                                    day: day.day,
                                },
                                words: day_stats.words,
                                dictations:
                                    day_stats.dictations,
                            }
                        },
                    )
                    .collect(),
            };

        let contents =
            serde_json::to_string_pretty(
                &stored,
            )?;

        fs::write(
            &self.path,
            contents,
        )?;

        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(
            format!(
                "tinyvox-stats-{}-{}.json",
                std::process::id(),
                name
            ),
        )
    }

    #[test]
    fn missing_stats_loads_empty() {
        let path =
            test_path("missing");

        let _ =
            fs::remove_file(&path);

        let store =
            StatsStore::new(&path);

        let stats =
            store.load().unwrap();

        assert_eq!(
            stats.total_words(),
            0
        );

        assert_eq!(
            stats.total_dictations(),
            0
        );

        assert_eq!(
            stats.total_recording_ms(),
            0
        );
    }

    #[test]
    fn stats_round_trip() {
        let path =
            test_path("round-trip");

        let _ =
            fs::remove_file(&path);

        let store =
            StatsStore::new(&path);

        let mut stats =
            DictationStats::new();

        let day =
            DayKey::new(2026, 8, 12);

        stats.record(
            day,
            100,
            60_000,
        );

        stats.record(
            day,
            50,
            30_000,
        );

        store.save(&stats).unwrap();

        let loaded =
            store.load().unwrap();

        assert_eq!(
            loaded.total_words(),
            150
        );

        assert_eq!(
            loaded.total_dictations(),
            2
        );

        assert_eq!(
            loaded.total_recording_ms(),
            90_000
        );

        assert_eq!(
            loaded.today(day),
            DayStats {
                words: 150,
                dictations: 2,
            }
        );

        let _ =
            fs::remove_file(&path);
    }

    #[test]
    fn daily_activity_survives_restart() {
        let path =
            test_path("daily");

        let _ =
            fs::remove_file(&path);

        let store =
            StatsStore::new(&path);

        let mut stats =
            DictationStats::new();

        let day_one =
            DayKey::new(2026, 8, 11);

        let day_two =
            DayKey::new(2026, 8, 12);

        stats.record(
            day_one,
            25,
            20_000,
        );

        stats.record(
            day_two,
            50,
            40_000,
        );

        store.save(&stats).unwrap();

        let loaded =
            store.load().unwrap();

        assert_eq!(
            loaded.today(day_one),
            DayStats {
                words: 25,
                dictations: 1,
            }
        );

        assert_eq!(
            loaded.today(day_two),
            DayStats {
                words: 50,
                dictations: 1,
            }
        );

        assert_eq!(
            loaded.current_streak(day_two),
            2
        );

        let _ =
            fs::remove_file(&path);
    }
}