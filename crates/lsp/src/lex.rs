//! A lightweight, error-tolerant lexer for pliron textual IR.
//!
//! This is *not* a faithful reimplementation of pliron's parser. It only
//! recognizes the token shapes needed for navigation: identifiers
//! (`sum`), dotted identifiers (`llvm.add`, `builtin.integer`), block
//! labels (`^entry`), symbols (`@main`), outlined attribute references
//! (`#attr`), numbers, strings and single-character punctuation.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokKind {
    /// A bare identifier: potential SSA value name, keyword, or type word.
    Ident,
    /// `a.b` or `a.b.c` — an op name or type name; never an SSA value.
    DottedIdent,
    /// `^name`
    Block,
    /// `@name`
    Sym,
    /// `#name`
    Outlined,
    Number,
    Str,
    /// `->`
    Arrow,
    /// Any other single character.
    Punct(char),
}

#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: TokKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start..self.end]
    }

    /// Token text without a leading `^`/`@`/`#` sigil.
    pub fn name<'a>(&self, s: &'a str) -> &'a str {
        match self.kind {
            TokKind::Block | TokKind::Sym | TokKind::Outlined => &s[self.start + 1..self.end],
            _ => self.text(s),
        }
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn lex(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    let n = text.len();
    while i < n {
        let c = text[i..].chars().next().unwrap();
        let clen = c.len_utf8();
        if c.is_whitespace() {
            i += clen;
            continue;
        }
        // Strings.
        if c == '"' {
            let start = i;
            i += 1;
            while i < n {
                match bytes[i] {
                    b'\\' => i = (i + 2).min(n),
                    b'"' => {
                        i += 1;
                        break;
                    }
                    _ => {
                        // Skip a full char (strings may hold non-ASCII).
                        let ch = text[i..].chars().next().unwrap();
                        i += ch.len_utf8();
                    }
                }
            }
            toks.push(Token {
                kind: TokKind::Str,
                start,
                end: i,
            });
            continue;
        }
        // Sigiled identifiers.
        if (c == '^' || c == '@' || c == '#')
            && i + 1 < n
            && text[i + 1..].chars().next().is_some_and(is_ident_start)
        {
            let start = i;
            i += 1;
            while i < n && text[i..].chars().next().is_some_and(is_ident_cont) {
                i += 1;
            }
            let kind = match c {
                '^' => TokKind::Block,
                '@' => TokKind::Sym,
                _ => TokKind::Outlined,
            };
            toks.push(Token { kind, start, end: i });
            continue;
        }
        // Identifiers, possibly dotted.
        if is_ident_start(c) {
            let start = i;
            let mut dotted = false;
            loop {
                while i < n && text[i..].chars().next().is_some_and(is_ident_cont) {
                    i += 1;
                }
                // Consume `.` only when directly followed by an ident start.
                if i + 1 < n
                    && bytes[i] == b'.'
                    && text[i + 1..].chars().next().is_some_and(is_ident_start)
                {
                    dotted = true;
                    i += 1;
                } else {
                    break;
                }
            }
            toks.push(Token {
                kind: if dotted {
                    TokKind::DottedIdent
                } else {
                    TokKind::Ident
                },
                start,
                end: i,
            });
            continue;
        }
        // Numbers (loose: digits then alnum/dot/underscore, handles 0x..,
        // floats, suffixes; also a leading minus handled as punct).
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < n {
                let ch = text[i..].chars().next().unwrap();
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += 1;
                } else if ch == '.'
                    && i + 1 < n
                    && text[i + 1..].chars().next().is_some_and(|d| d.is_ascii_digit())
                {
                    i += 1;
                } else {
                    break;
                }
            }
            toks.push(Token {
                kind: TokKind::Number,
                start,
                end: i,
            });
            continue;
        }
        // Arrow, so that `->` does not disturb `<`/`>` depth tracking.
        if c == '-' && i + 1 < n && bytes[i + 1] == b'>' {
            toks.push(Token {
                kind: TokKind::Arrow,
                start: i,
                end: i + 2,
            });
            i += 2;
            continue;
        }
        toks.push(Token {
            kind: TokKind::Punct(c),
            start: i,
            end: i + clen,
        });
        i += clen;
    }
    toks
}
