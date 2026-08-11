#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    Manual,
    AutoLearned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    pub id: EntryId,
    pub wrong: String,
    pub correct: String,
    pub source: EntrySource,
    pub hit_count: u32,
}

#[derive(Debug, Default)]
pub struct Dictionary {
    entries: Vec<DictionaryEntry>,
    next_id: u64,
}

impl Dictionary {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
        }
    }

    pub(crate) fn from_entries(
        entries: Vec<DictionaryEntry>,
        next_id: u64,
    ) -> Self {
        Self {
            entries,
            next_id,
        }
    }

    pub(crate) fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn apply(
        &mut self,
        transcript: &str,
    ) -> String {
        let mut result =
            transcript.to_string();

        for entry in &mut self.entries {
            let replaced =
                replace_tokens(
                    &result,
                    &entry.wrong,
                    &entry.correct,
                );

            if replaced != result {
                entry.hit_count =
                    entry.hit_count.saturating_add(1);

                result = replaced;
            }
        }

        result
    }

    pub fn add(
        &mut self,
        wrong: &str,
        correct: &str,
        source: EntrySource,
    ) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id += 1;

        self.entries.push(
            DictionaryEntry {
                id,
                wrong: wrong.to_string(),
                correct: correct.to_string(),
                source,
                hit_count: 0,
            },
        );

        id
    }

    pub fn remove(
        &mut self,
        id: EntryId,
    ) -> bool {
        let original_len =
            self.entries.len();

        self.entries.retain(
            |entry| entry.id != id,
        );

        self.entries.len()
            != original_len
    }

    pub fn edit(
        &mut self,
        id: EntryId,
        wrong: &str,
        correct: &str,
    ) -> bool {
        if let Some(entry) =
            self.entries
                .iter_mut()
                .find(|entry| entry.id == id)
        {
            entry.wrong = wrong.to_string();
            entry.correct = correct.to_string();

            true
        } else {
            false
        }
    }

    pub fn entries(
        &self,
    ) -> &[DictionaryEntry] {
        &self.entries
    }
}

fn replace_tokens(
    text: &str,
    wrong: &str,
    correct: &str,
) -> String {
    if wrong.is_empty() {
        return text.to_string();
    }

    text.split_inclusive(
        |character: char| {
            character.is_whitespace()
                || character.is_ascii_punctuation()
        },
    )
    .map(|token| {
        let mut boundary = token.len();

        while boundary > 0 {
            let character =
                token[..boundary]
                    .chars()
                    .next_back();

            if character
                .is_some_and(|c| {
                    c.is_whitespace()
                        || c.is_ascii_punctuation()
                })
            {
                boundary -=
                    character.unwrap().len_utf8();
            } else {
                break;
            }
        }

        let word = &token[..boundary];
        let suffix = &token[boundary..];

        if word.eq_ignore_ascii_case(wrong) {
            format!("{correct}{suffix}")
        } else {
            token.to_string()
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_entry() {
        let mut dictionary =
            Dictionary::new();

        let id = dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.entries().len(),
            1
        );

        assert_eq!(
            dictionary.entries()[0].id,
            id
        );
    }

    #[test]
    fn replaces_exact_token() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "kubernets",
            "Kubernetes",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "I use Kubernets every day."
            ),
            "I use Kubernetes every day."
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "SAQLAIN is here."
            ),
            "Saqlain is here."
        );
    }

    #[test]
    fn punctuation_is_preserved() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "kubernets",
            "Kubernetes",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "Kubernets, Kubernets!"
            ),
            "Kubernetes, Kubernetes!"
        );
    }

    #[test]
    fn does_not_replace_inside_words() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "cat",
            "dog",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "concatenate cat bobcat"
            ),
            "concatenate dog bobcat"
        );
    }

    #[test]
    fn multiple_entries_are_applied() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "kubernets",
            "Kubernetes",
            EntrySource::Manual,
        );

        dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "Kubernets by Saqlain"
            ),
            "Kubernetes by Saqlain"
        );
    }

    #[test]
    fn hit_count_increments() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        dictionary.apply(
            "hello saqlain",
        );

        assert_eq!(
            dictionary.entries()[0].hit_count,
            1
        );

        dictionary.apply(
            "hello saqlain again",
        );

        assert_eq!(
            dictionary.entries()[0].hit_count,
            2
        );
    }

    #[test]
    fn remove_entry() {
        let mut dictionary =
            Dictionary::new();

        let id = dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        assert!(dictionary.remove(id));

        assert!(
            dictionary.entries().is_empty()
        );
    }

    #[test]
    fn removing_unknown_entry_returns_false() {
        let mut dictionary =
            Dictionary::new();

        assert!(
            !dictionary.remove(
                EntryId(999)
            )
        );
    }

    #[test]
    fn unchanged_text_is_returned() {
        let mut dictionary =
            Dictionary::new();

        dictionary.add(
            "saqlain",
            "Saqlain",
            EntrySource::Manual,
        );

        assert_eq!(
            dictionary.apply(
                "hello from TinyVox"
            ),
            "hello from TinyVox"
        );
    }
}