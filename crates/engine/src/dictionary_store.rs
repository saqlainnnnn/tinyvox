use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::dictionary::{Dictionary, DictionaryEntry, EntryId, EntrySource};

#[derive(Debug)]
pub enum DictionaryStoreError {
    Io(io::Error),
    Serialization(serde_json::Error),
}

impl std::fmt::Display for DictionaryStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => {
                write!(f, "dictionary I/O error: {error}")
            }

            Self::Serialization(error) => {
                write!(f, "dictionary serialization error: {error}")
            }
        }
    }
}

impl std::error::Error for DictionaryStoreError {}

impl From<io::Error> for DictionaryStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for DictionaryStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDictionary {
    entries: Vec<StoredDictionaryEntry>,
    next_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDictionaryEntry {
    id: u64,
    wrong: String,
    correct: String,
    source: StoredEntrySource,
    hit_count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
enum StoredEntrySource {
    Manual,
    AutoLearned,
}

pub struct DictionaryStore {
    path: PathBuf,
}

impl DictionaryStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load(&self) -> Result<Dictionary, DictionaryStoreError> {
        if !self.path.exists() {
            return Ok(Dictionary::new());
        }

        let contents = fs::read_to_string(&self.path)?;

        let stored: StoredDictionary = serde_json::from_str(&contents)?;

        let entries = stored
            .entries
            .into_iter()
            .map(|entry| DictionaryEntry {
                id: EntryId(entry.id),
                wrong: entry.wrong,
                correct: entry.correct,
                source: match entry.source {
                    StoredEntrySource::Manual => EntrySource::Manual,

                    StoredEntrySource::AutoLearned => EntrySource::AutoLearned,
                },
                hit_count: entry.hit_count,
            })
            .collect();

        Ok(Dictionary::from_entries(entries, stored.next_id))
    }

    pub fn save(&self, dictionary: &Dictionary) -> Result<(), DictionaryStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let stored = StoredDictionary {
            entries: dictionary
                .entries()
                .iter()
                .map(|entry| StoredDictionaryEntry {
                    id: entry.id.0,
                    wrong: entry.wrong.clone(),
                    correct: entry.correct.clone(),
                    source: match entry.source {
                        EntrySource::Manual => StoredEntrySource::Manual,

                        EntrySource::AutoLearned => StoredEntrySource::AutoLearned,
                    },
                    hit_count: entry.hit_count,
                })
                .collect(),
            next_id: dictionary.next_id(),
        };

        let contents = serde_json::to_string_pretty(&stored)?;

        fs::write(&self.path, contents)?;

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
        std::env::temp_dir().join(format!(
            "tinyvox-dictionary-{}-{}.json",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn missing_dictionary_loads_empty() {
        let path = test_path("missing");

        let _ = fs::remove_file(&path);

        let store = DictionaryStore::new(&path);

        let dictionary = store.load().unwrap();

        assert!(dictionary.entries().is_empty());
    }

    #[test]
    fn dictionary_round_trips() {
        let path = test_path("round-trips");

        let _ = fs::remove_file(&path);

        let store = DictionaryStore::new(&path);

        let mut dictionary = Dictionary::new();

        dictionary.add("kubernets", "Kubernetes", EntrySource::Manual);

        dictionary.add("saqlain", "Saqlain", EntrySource::AutoLearned);

        store.save(&dictionary).unwrap();

        let mut loaded = store.load().unwrap();

        assert_eq!(loaded.entries(), dictionary.entries());

        assert_eq!(
            loaded.apply("Kubernets by SAQLAIN"),
            "Kubernetes by Saqlain"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn hit_count_survives_restart() {
        let path = test_path("hit-count");

        let _ = fs::remove_file(&path);

        let store = DictionaryStore::new(&path);

        let mut dictionary = Dictionary::new();

        dictionary.add("kubernets", "Kubernetes", EntrySource::Manual);

        dictionary.apply("I use Kubernets");

        store.save(&dictionary).unwrap();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.entries()[0].hit_count, 1);

        let _ = fs::remove_file(&path);
    }
}
