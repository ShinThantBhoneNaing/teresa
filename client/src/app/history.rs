//! The input line: buffer, cursor, sent-history, and Tab completion.

use unicode_width::UnicodeWidthChar;

/// How many sent lines the Up-arrow history keeps.
const HISTORY_CAP: usize = 200;

/// The editable input line.
#[derive(Debug, Default)]
pub struct InputLine {
    /// The text being written.
    buffer: String,
    /// Cursor position as a **byte** offset into `buffer`, always on a char
    /// boundary.
    cursor: usize,
    /// Previously sent lines, newest last.
    history: Vec<String>,
    /// Where we are while browsing history; `None` means live editing.
    browse: Option<usize>,
    /// The line that was being edited when browsing started, restored on
    /// Down-past-the-end. Losing a half-typed message to a stray Up-arrow is
    /// the kind of papercut people uninstall over.
    stash: String,
    /// Completion state, if a Tab cycle is in progress.
    completion: Option<Completion>,
}

/// An in-progress Tab-completion cycle.
#[derive(Debug)]
struct Completion {
    /// Byte range of the fragment being replaced.
    start: usize,
    /// The candidates being cycled.
    options: Vec<String>,
    /// Which candidate is currently inserted.
    index: usize,
}

impl InputLine {
    /// The current text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// Whether there is nothing typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The cursor's display column (terminal cells, not bytes).
    #[must_use]
    pub fn cursor_column(&self) -> u16 {
        self.buffer[..self.cursor]
            .chars()
            .map(|c| c.width().unwrap_or(0) as u16)
            .sum()
    }

    /// Inserts a character at the cursor.
    pub fn insert(&mut self, c: char) {
        self.completion = None;
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Inserts a whole string (paste).
    pub fn insert_str(&mut self, s: &str) {
        self.completion = None;
        self.buffer.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        self.completion = None;
        if let Some(prev) = self.prev_boundary() {
            self.buffer.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    /// Deletes the character under the cursor.
    pub fn delete(&mut self) {
        self.completion = None;
        if let Some(next) = self.next_boundary() {
            self.buffer.replace_range(self.cursor..next, "");
        }
    }

    /// Deletes the word before the cursor (Ctrl+Backspace / Alt+Backspace).
    pub fn delete_word(&mut self) {
        self.completion = None;
        let before = &self.buffer[..self.cursor];
        let trimmed = before.trim_end();
        let cut_from = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        self.buffer.replace_range(cut_from..self.cursor, "");
        self.cursor = cut_from;
    }

    /// Clears the whole line (Ctrl+U).
    pub fn clear(&mut self) {
        self.completion = None;
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Moves the cursor left one character.
    pub fn left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    /// Moves the cursor right one character.
    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.cursor = next;
        }
    }

    /// Jumps to the start of the line (Home / Ctrl+A).
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Jumps to the end of the line (End / Ctrl+E).
    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
    }

    /// Takes the line for sending: clears the editor, records history.
    #[must_use]
    pub fn take(&mut self) -> String {
        self.completion = None;
        self.browse = None;
        let line = std::mem::take(&mut self.buffer);
        self.cursor = 0;

        // Consecutive duplicates would make Up-Up-Up useless.
        if !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY_CAP {
                self.history.remove(0);
            }
        }
        line
    }

    /// Steps back through sent history (Up).
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_index = match self.browse {
            None => {
                self.stash = self.buffer.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.browse = Some(next_index);
        self.buffer = self.history[next_index].clone();
        self.cursor = self.buffer.len();
    }

    /// Steps forward through sent history (Down), restoring the stashed line
    /// past the newest entry.
    pub fn history_next(&mut self) {
        match self.browse {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.browse = Some(i + 1);
                self.buffer = self.history[i + 1].clone();
                self.cursor = self.buffer.len();
            }
            Some(_) => {
                self.browse = None;
                self.buffer = std::mem::take(&mut self.stash);
                self.cursor = self.buffer.len();
            }
        }
    }

    /// Tab: completes the fragment at the cursor from `candidates`
    /// (usernames, room names, command names). Repeated presses cycle.
    pub fn complete(&mut self, candidates: &[String]) {
        // A second Tab in a live cycle advances it.
        if let Some(cycle) = &mut self.completion {
            cycle.index = (cycle.index + 1) % cycle.options.len();
            let replacement = cycle.options[cycle.index].clone();
            let start = cycle.start;
            self.buffer.replace_range(start..self.cursor, &replacement);
            self.cursor = start + replacement.len();
            return;
        }

        // Fragment = from the last whitespace to the cursor.
        let before = &self.buffer[..self.cursor];
        let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        let fragment = &before[start..];
        if fragment.is_empty() {
            return;
        }

        // An @ sigil participates in matching-by-name but survives completion.
        let (sigil, stem) = match fragment.strip_prefix('@') {
            Some(rest) => ("@", rest),
            None => ("", fragment),
        };

        let stem_lower = stem.to_lowercase();
        let options: Vec<String> = candidates
            .iter()
            .filter(|c| c.to_lowercase().starts_with(&stem_lower))
            .map(|c| format!("{sigil}{c}"))
            .collect();
        if options.is_empty() {
            return;
        }

        let replacement = options[0].clone();
        self.buffer.replace_range(start..self.cursor, &replacement);
        self.cursor = start + replacement.len();
        self.completion = Some(Completion {
            start,
            options,
            index: 0,
        });
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.buffer[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_with(text: &str) -> InputLine {
        let mut line = InputLine::default();
        line.insert_str(text);
        line
    }

    #[test]
    fn editing_handles_multibyte_characters() {
        let mut line = line_with("héllo 🐧");
        assert_eq!(line.text(), "héllo 🐧");

        line.backspace();
        assert_eq!(line.text(), "héllo ");

        line.left();
        line.left();
        line.insert('X');
        assert_eq!(line.text(), "héllXo ");
    }

    #[test]
    fn cursor_column_counts_cells_not_bytes() {
        let line = line_with("é🐧");
        // é = 1 cell, 🐧 = 2 cells.
        assert_eq!(line.cursor_column(), 3);
    }

    #[test]
    fn word_delete_eats_one_word() {
        let mut line = line_with("git push origin main");
        line.delete_word();
        assert_eq!(line.text(), "git push origin ");
        line.delete_word();
        assert_eq!(line.text(), "git push ");
    }

    #[test]
    fn take_records_history_without_consecutive_duplicates() {
        let mut line = InputLine::default();
        line.insert_str("first");
        assert_eq!(line.take(), "first");
        line.insert_str("first");
        let _ = line.take();
        line.insert_str("second");
        let _ = line.take();

        line.history_prev();
        assert_eq!(line.text(), "second");
        line.history_prev();
        assert_eq!(line.text(), "first");
        line.history_prev();
        assert_eq!(line.text(), "first", "history stops at the oldest entry");
    }

    #[test]
    fn browsing_history_stashes_and_restores_the_draft() {
        let mut line = InputLine::default();
        line.insert_str("sent already");
        let _ = line.take();

        line.insert_str("half typed dra");
        line.history_prev();
        assert_eq!(line.text(), "sent already");
        line.history_next();
        assert_eq!(line.text(), "half typed dra", "the draft survives");
    }

    #[test]
    fn completion_cycles_through_matches() {
        let users = vec!["alice".to_owned(), "albert".to_owned(), "bob".to_owned()];
        let mut line = line_with("hey al");

        line.complete(&users);
        assert_eq!(line.text(), "hey alice");
        line.complete(&users);
        assert_eq!(line.text(), "hey albert");
        line.complete(&users);
        assert_eq!(line.text(), "hey alice", "wraps around");
    }

    #[test]
    fn completion_preserves_the_mention_sigil() {
        let users = vec!["alice".to_owned()];
        let mut line = line_with("ping @ali");
        line.complete(&users);
        assert_eq!(line.text(), "ping @alice");
    }

    #[test]
    fn completion_is_case_insensitive_and_no_match_is_a_no_op() {
        let users = vec!["Alice".to_owned()];
        let mut line = line_with("Al");
        line.complete(&users);
        assert_eq!(line.text(), "Alice");

        let mut line = line_with("zz");
        line.complete(&users);
        assert_eq!(line.text(), "zz");
    }

    #[test]
    fn typing_cancels_a_completion_cycle() {
        let users = vec!["alice".to_owned(), "albert".to_owned()];
        let mut line = line_with("al");
        line.complete(&users);
        line.insert('!');
        // A fresh Tab now completes from "alice!" → no match → unchanged.
        line.complete(&users);
        assert_eq!(line.text(), "alice!");
    }
}
