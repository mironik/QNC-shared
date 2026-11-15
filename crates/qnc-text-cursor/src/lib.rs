//! The keyboard on a text of timed words, in any form that shows one (the transcript, the
//! script; user 2026-11-15: "u tekst panelu nemamo kursora, pa ne možemo selektirati tekst
//! tipkovnicom"; "kocka treba biti univerzalna"). The text is its lines of words, each word by
//! its frames `[from, to)`; the cursor is a word (the word at the playhead unless placed); the
//! arrows move it a word left or right, a line up or down; Shift with the arrows selects from
//! the word the selection started on to the cursor. What a selection marks (the source IN/OUT)
//! is the caller's.

/// Where an arrow moves the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStep {
    Left,
    Right,
    Up,
    Down,
}

/// A word by its frames `[from, to)`.
pub type WordSpan = (u64, u64);

/// The rows a text is shown in, as its panel laid them out on screen (a line of the text
/// wraps into several; user 2026-11-15: the cursor skipped rows). The panel that paints the
/// text writes them, the keys read them; a handle shared by the clones of a view, not part of
/// what the view says (two views are equal whatever their rows).
#[derive(Debug, Clone, Default)]
pub struct ShownRows(std::sync::Arc<std::sync::Mutex<Vec<Vec<WordSpan>>>>);

impl ShownRows {
    /// The rows painted now, top to bottom.
    pub fn set(&self, rows: Vec<Vec<WordSpan>>) {
        if let Ok(mut shown) = self.0.lock() {
            *shown = rows;
        }
    }

    /// The rows last painted (empty before the first paint).
    pub fn get(&self) -> Vec<Vec<WordSpan>> {
        self.0.lock().map(|rows| rows.clone()).unwrap_or_default()
    }
}

impl PartialEq for ShownRows {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// The keyboard on a text: the word of the cursor until the playhead goes elsewhere (a player
/// takes a moment to reach a cued word, quick presses go on from the cursor), and the word a
/// selection started on, while Shift moves on from it.
#[derive(Debug, Default, Clone)]
pub struct TextCursor {
    at: Option<(WordSpan, Option<u64>)>,
    anchor: Option<WordSpan>,
}

impl TextCursor {
    /// The cursor on `word` (a click on it, an arrow to it); `playhead` where it is now.
    pub fn place(&mut self, word: WordSpan, playhead: Option<u64>) {
        self.at = Some((word, playhead));
    }

    /// The word of the cursor: the one placed while the playhead has not gone elsewhere, else
    /// the word at the playhead.
    pub fn word(&self, lines: &[Vec<WordSpan>], playhead: Option<u64>) -> Option<WordSpan> {
        match self.at {
            Some((word, then)) if then == playhead || playhead.is_some_and(|at| (word.0..word.1).contains(&at)) => Some(word),
            _ => cursor_word(lines, playhead),
        }
    }

    /// The cursor moved without Shift: no selection goes on.
    pub fn moved(&mut self) {
        self.anchor = None;
    }

    /// The cursor moved with Shift from `cursor` to `to`: the selection from where it
    /// started to `to`, earlier word first.
    pub fn selected(&mut self, cursor: WordSpan, to: WordSpan) -> (WordSpan, WordSpan) {
        let anchor = *self.anchor.get_or_insert(cursor);
        if anchor.0 <= to.0 { (anchor, to) } else { (to, anchor) }
    }

    /// One arrow on `lines`: where the cursor goes, and the selection when `select` (Shift);
    /// `None` at the edge of the text or without words.
    pub fn step(&mut self, lines: &[Vec<WordSpan>], playhead: Option<u64>, step: TextStep, select: bool) -> Option<(WordSpan, Option<(WordSpan, WordSpan)>)> {
        let cursor = self.word(lines, playhead)?;
        let to = word_beside(lines, cursor, step)?;
        let selection = if select {
            Some(self.selected(cursor, to))
        } else {
            self.moved();
            None
        };
        self.place(to, playhead);
        Some((to, selection))
    }
}

/// The word of the cursor: the word said at `playhead`, else the last before it, else the
/// first.
pub fn cursor_word(lines: &[Vec<WordSpan>], playhead: Option<u64>) -> Option<WordSpan> {
    let words = || lines.iter().flatten().copied();
    let at = playhead.unwrap_or(0);
    words().find(|word| (word.0..word.1).contains(&at)).or_else(|| words().filter(|word| word.0 <= at).last()).or_else(|| words().next())
}

/// The word an arrow moves the cursor to from `cursor`: the next or previous word, or the word
/// at the same place in the line above or below; `None` at the edge of the text.
pub fn word_beside(lines: &[Vec<WordSpan>], cursor: WordSpan, step: TextStep) -> Option<WordSpan> {
    let lines: Vec<&Vec<WordSpan>> = lines.iter().filter(|line| !line.is_empty()).collect();
    let (line_at, word_at) = lines.iter().enumerate().find_map(|(l, line)| line.iter().position(|word| *word == cursor).map(|w| (l, w)))?;
    let span = |l: usize, w: usize| lines.get(l).map(|line| line[w.min(line.len() - 1)]);
    match step {
        TextStep::Right if word_at + 1 < lines[line_at].len() => span(line_at, word_at + 1),
        TextStep::Right => span(line_at + 1, 0),
        TextStep::Left if word_at > 0 => span(line_at, word_at - 1),
        TextStep::Left => line_at.checked_sub(1).and_then(|l| span(l, usize::MAX)),
        TextStep::Down => span(line_at + 1, word_at),
        TextStep::Up => line_at.checked_sub(1).and_then(|l| span(l, word_at)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_move_the_cursor_a_word_and_a_line_and_shift_selects_from_where_it_started() {
        let lines = vec![vec![(0, 10), (10, 20), (20, 30)], vec![(40, 50), (50, 60)]];
        assert_eq!(cursor_word(&lines, Some(15)), Some((10, 20)));
        assert_eq!(cursor_word(&lines, Some(35)), Some((20, 30)), "between words: the last before");
        assert_eq!(word_beside(&lines, (20, 30), TextStep::Right), Some((40, 50)), "on to the next line");
        assert_eq!(word_beside(&lines, (40, 50), TextStep::Left), Some((20, 30)));
        assert_eq!(word_beside(&lines, (20, 30), TextStep::Down), Some((50, 60)), "the same place, or the line's last");
        assert_eq!(word_beside(&lines, (50, 60), TextStep::Up), Some((10, 20)));
        assert_eq!(word_beside(&lines, (0, 10), TextStep::Left), None);
        let mut cursor = TextCursor::default();
        assert_eq!(cursor.step(&lines, Some(15), TextStep::Right, true), Some(((20, 30), Some(((10, 20), (20, 30))))));
        assert_eq!(cursor.step(&lines, Some(15), TextStep::Right, true), Some(((40, 50), Some(((10, 20), (40, 50))))), "quick presses go on from the cursor");
        assert_eq!(cursor.step(&lines, Some(15), TextStep::Left, false), Some(((20, 30), None)), "an arrow without Shift ends the selection");
    }
}
