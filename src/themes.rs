//! Board themes: categories of related words a board can be built around.
//!
//! Themes cannot be found by rolling dice and hoping. Measured over 300 random
//! boards, animals turned up three or more times on 30% of them, food on 6%,
//! and countries on none at all -- a 4x4 grid simply does not hold enough words
//! from one category by chance. So themed boards are *constructed*: the words are
//! planted first and the rest of the grid is filled around them.

use std::collections::HashSet;

const EMBEDDED: &str = include_str!("../themes.txt");

/// Theme words have to be short enough that several fit on one board.
pub const MAX_THEME_WORD: usize = 6;
pub const MIN_THEME_WORD: usize = 3;

pub struct Theme {
    pub name: String,
    /// Words short enough to plant on a board.
    pub plantable: Vec<String>,
    /// Every word in the category, for recognising them on a finished board.
    pub all: HashSet<String>,
}

pub struct Themes {
    pub list: Vec<Theme>,
}

impl Themes {
    pub fn load() -> Self {
        let mut list: Vec<Theme> = Vec::new();

        for line in EMBEDDED.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                list.push(Theme {
                    name: name.to_string(),
                    plantable: Vec::new(),
                    all: HashSet::new(),
                });
                continue;
            }
            if let Some(theme) = list.last_mut() {
                let word = line.to_ascii_lowercase();
                if (MIN_THEME_WORD..=MAX_THEME_WORD).contains(&word.len()) {
                    theme.plantable.push(word.clone());
                }
                theme.all.insert(word);
            }
        }

        // A category with too few plantable words cannot seed a board.
        list.retain(|t| t.plantable.len() >= 12);
        Themes { list }
    }

    #[cfg(test)]
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.list.iter().find(|t| t.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_load_with_plantable_words() {
        let themes = Themes::load();
        assert!(themes.list.len() >= 15, "only {} themes", themes.list.len());

        for theme in &themes.list {
            assert!(!theme.name.is_empty());
            assert!(theme.plantable.len() >= 12, "{} is too small", theme.name);
            for word in &theme.plantable {
                assert!((MIN_THEME_WORD..=MAX_THEME_WORD).contains(&word.len()));
                assert!(theme.all.contains(word));
            }
        }
    }

    #[test]
    fn the_named_categories_are_present() {
        let themes = Themes::load();
        for name in ["Animals", "Food", "Countries", "Verbs", "Science"] {
            assert!(themes.get(name).is_some(), "missing {name}");
        }
        assert!(themes.get("Animals").unwrap().all.contains("otter"));
        assert!(themes.get("Countries").unwrap().all.contains("cuba"));
    }

    #[test]
    fn theme_words_are_playable() {
        // A board built around words the dictionary rejects would be unwinnable.
        let dict = crate::dictionary::Dictionary::new();
        let themes = Themes::load();
        for theme in &themes.list {
            for word in &theme.plantable {
                assert!(dict.contains(word), "{}: {word} is not accepted", theme.name);
            }
        }
    }
}
