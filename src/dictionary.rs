//! Word lists backed by a single prefix trie, in two tiers.
//!
//! Every word in `dictionary.txt` is *accepted* when the player traces it, so a
//! legitimate find is never rejected. Only the smaller *common* tier is used to
//! build boards and to report missed words -- the full list is padded with
//! obscurities like "usninic" that would make a scorecard read as nonsense.
//!
//! The trie matters more than the raw word set: generating a good board means
//! solving a few hundred candidate boards, and each solve is a depth-first walk
//! over 16 tiles that has to bail out the moment a path stops spelling a prefix.

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

pub const MIN_WORD_LEN: usize = 3;
pub const MAX_WORD_LEN: usize = 16;

/// Shipped word lists, so the game runs from any working directory.
const EMBEDDED_ACCEPTED: &str = include_str!("../dictionary.txt");
const EMBEDDED_COMMON: &str = include_str!("../common.txt");
const EMBEDDED_THEMES: &str = include_str!("../themes.txt");

pub type NodeId = u32;

struct Node {
    /// Sorted `(letter, child)` pairs. Most nodes have one or two children, so
    /// a small sorted vec beats a 26-wide array by an order of magnitude in memory.
    children: Vec<(u8, NodeId)>,
    is_word: bool,
    /// Part of the curated tier, i.e. worth putting on a scorecard.
    is_common: bool,
}

pub struct Dictionary {
    nodes: Vec<Node>,
    word_count: usize,
    common_count: usize,
}

impl Dictionary {
    pub fn new() -> Self {
        let mut dict = Dictionary {
            nodes: vec![Node { children: Vec::new(), is_word: false, is_common: false }],
            word_count: 0,
            common_count: 0,
        };

        // A word list sitting next to the binary or the project wins over the
        // embedded copy, so the lists can be swapped without a rebuild.
        dict.load(&Self::read_list("dictionary.txt", EMBEDDED_ACCEPTED), false);
        dict.load(&Self::read_list("common.txt", EMBEDDED_COMMON), true);
        // Theme words count as ordinary words: a themed board is unplayable if the
        // very words it was built around are rejected. This is also what puts the
        // proper nouns -- CUBA, OSLO, TEXAS -- into play, since building common.txt
        // from SCOWL dropped anything not lowercase.
        dict.load(&Self::read_list("themes.txt", EMBEDDED_THEMES), true);

        dict
    }

    /// On the web there is no filesystem to override from, so the embedded copy is
    /// the only copy.
    #[cfg(target_arch = "wasm32")]
    fn read_list(_name: &str, embedded: &'static str) -> String {
        embedded.to_string()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_list(name: &str, embedded: &'static str) -> String {
        match Self::find_word_list(name).and_then(|p| fs::read_to_string(p).ok()) {
            Some(text) => text,
            None => embedded.to_string(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn find_word_list(name: &str) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = vec![PathBuf::from(name)];
        if let Ok(exe) = std::env::current_exe() {
            let mut dir = exe.parent().map(Path::to_path_buf);
            // Walk up out of target/debug/ so a `cargo run` binary finds the repo copy.
            for _ in 0..4 {
                match dir {
                    Some(d) => {
                        candidates.push(d.join(name));
                        dir = d.parent().map(Path::to_path_buf);
                    }
                    None => break,
                }
            }
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    fn load(&mut self, text: &str, common: bool) {
        for line in text.lines() {
            let word = line.trim().to_ascii_lowercase();
            let len = word.len();
            if len < MIN_WORD_LEN || len > MAX_WORD_LEN {
                continue;
            }
            if !word.bytes().all(|b| b.is_ascii_lowercase()) {
                continue;
            }
            self.insert(&word, common);
        }
    }

    fn insert(&mut self, word: &str, common: bool) {
        let mut node: NodeId = 0;
        for b in word.bytes() {
            node = match self.child(node, b) {
                Some(next) => next,
                None => {
                    let next = self.nodes.len() as NodeId;
                    self.nodes.push(Node { children: Vec::new(), is_word: false, is_common: false });
                    let kids = &mut self.nodes[node as usize].children;
                    let at = kids.partition_point(|(letter, _)| *letter < b);
                    kids.insert(at, (b, next));
                    next
                }
            };
        }
        let leaf = &mut self.nodes[node as usize];
        if !leaf.is_word {
            leaf.is_word = true;
            self.word_count += 1;
        }
        if common && !leaf.is_common {
            leaf.is_common = true;
            self.common_count += 1;
        }
    }

    fn child(&self, node: NodeId, letter: u8) -> Option<NodeId> {
        let kids = &self.nodes[node as usize].children;
        kids.binary_search_by_key(&letter, |(l, _)| *l)
            .ok()
            .map(|i| kids[i].1)
    }

    /// Root of the trie; the starting point for a board walk.
    pub fn root(&self) -> NodeId {
        0
    }

    /// Follow one letter. `None` means no word in the dictionary continues this way,
    /// which is the signal for the solver to prune the whole branch.
    pub fn step(&self, node: NodeId, letter: u8) -> Option<NodeId> {
        self.child(node, letter)
    }

    /// Follow a whole tile, which may carry more than one letter (the `Qu` die).
    pub fn step_str(&self, node: NodeId, letters: &str) -> Option<NodeId> {
        let mut node = node;
        for b in letters.bytes() {
            node = self.step(node, b)?;
        }
        Some(node)
    }

    pub fn is_word_node(&self, node: NodeId) -> bool {
        self.nodes[node as usize].is_word
    }

    /// Curated tier: safe to show the player as a word they should have found.
    pub fn is_common_node(&self, node: NodeId) -> bool {
        self.nodes[node as usize].is_common
    }

    pub fn is_dead_end(&self, node: NodeId) -> bool {
        self.nodes[node as usize].children.is_empty()
    }

    pub fn contains(&self, word: &str) -> bool {
        let word = word.to_ascii_lowercase();
        match self.step_str(self.root(), &word) {
            Some(node) => self.is_word_node(node),
            None => false,
        }
    }

    pub fn word_count(&self) -> usize {
        self.word_count
    }

    pub fn common_count(&self) -> usize {
        self.common_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_real_words_and_rejects_junk() {
        let dict = Dictionary::new();
        assert!(dict.contains("hero"));
        assert!(dict.contains("herd"));
        assert!(dict.contains("QUEST"));
        assert!(!dict.contains("zzzzq"));
    }

    #[test]
    fn short_words_are_excluded() {
        let dict = Dictionary::new();
        assert!(!dict.contains("an"));
    }

    #[test]
    fn common_tier_excludes_obscurities() {
        let dict = Dictionary::new();
        let common = |w: &str| {
            dict.step_str(dict.root(), w).is_some_and(|n| dict.is_common_node(n))
        };
        // Accepted when traced, but never shown on a scorecard.
        assert!(dict.contains("usninic") && !common("usninic"));
        assert!(dict.contains("aeolist") && !common("aeolist"));
        assert!(common("herd") && common("quest"));
        assert!(dict.common_count() < dict.word_count());
    }

    #[test]
    fn prefix_walk_prunes() {
        let dict = Dictionary::new();
        let node = dict.step_str(dict.root(), "her").unwrap();
        assert!(dict.is_word_node(node));
        assert!(dict.step_str(dict.root(), "xqz").is_none());
    }
}
