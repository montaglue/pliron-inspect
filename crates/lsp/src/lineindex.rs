//! Mapping between byte offsets, pliron source positions (1-based
//! line/column in characters) and LSP positions (0-based line, UTF-16
//! column).

/// A line index over an immutable document text.
#[derive(Debug)]
pub struct LineIndex {
    /// Byte offset at which each line starts. Always non-empty
    /// (line 0 starts at offset 0).
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len(),
        }
    }

    pub fn num_lines(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte range of line `line` (0-based), excluding the trailing newline.
    pub fn line_range(&self, line: usize, text: &str) -> (usize, usize) {
        let start = *self
            .line_starts
            .get(line)
            .unwrap_or(&self.len.min(text.len()));
        let end = self
            .line_starts
            .get(line + 1)
            .map(|s| s.saturating_sub(1))
            .unwrap_or(text.len());
        (start, end.max(start))
    }

    /// 0-based line containing byte `offset`.
    pub fn line_of(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        }
    }

    /// Convert a byte offset into an LSP position (0-based line, UTF-16 col).
    pub fn position_of(&self, offset: usize, text: &str) -> lsp_types::Position {
        let offset = offset.min(text.len());
        let line = self.line_of(offset);
        let line_start = self.line_starts[line];
        let col16: usize = text[line_start..offset]
            .chars()
            .map(|c| c.len_utf16())
            .sum();
        lsp_types::Position {
            line: line as u32,
            character: col16 as u32,
        }
    }

    /// Convert an LSP position into a byte offset (clamped).
    pub fn offset_of(&self, pos: lsp_types::Position, text: &str) -> usize {
        let line = pos.line as usize;
        if line >= self.line_starts.len() {
            return text.len();
        }
        let (start, end) = self.line_range(line, text);
        let mut col16 = 0usize;
        for (i, c) in text[start..end].char_indices() {
            if col16 >= pos.character as usize {
                return start + i;
            }
            col16 += c.len_utf16();
        }
        end
    }

    /// Convert a pliron source position (1-based line, 1-based column in
    /// characters) to a byte offset.
    pub fn offset_of_src_pos(&self, line1: i32, col1: i32, text: &str) -> usize {
        let line = (line1.max(1) as usize) - 1;
        if line >= self.line_starts.len() {
            return text.len();
        }
        let (start, end) = self.line_range(line, text);
        let want = (col1.max(1) as usize) - 1;
        let mut nchars = 0usize;
        for (i, _) in text[start..end].char_indices() {
            if nchars == want {
                return start + i;
            }
            nchars += 1;
        }
        end
    }

    /// LSP range covering bytes `start..end`.
    pub fn range_of(&self, start: usize, end: usize, text: &str) -> lsp_types::Range {
        lsp_types::Range {
            start: self.position_of(start, text),
            end: self.position_of(end.max(start), text),
        }
    }
}
