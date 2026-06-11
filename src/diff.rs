//! Wort-Diff für die Transformations-Vorschau.
//!
//! Klassisches LCS über Token (Wörter, Whitespace, Interpunktion,
//! Zeilenumbrüche). Bei sehr großen Texten wird auf eine Grob-Anzeige
//! (alles alt = entfernt, alles neu = eingefügt) zurückgefallen, damit
//! der UI-Thread nie spürbar rechnet.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Equal,
    Insert,
    Delete,
}

/// Obergrenze für die DP-Tabelle (Token_alt × Token_neu).
const MAX_CELLS: usize = 4_000_000;

pub fn word_diff(old: &str, new: &str) -> Vec<(Op, String)> {
    let a = tokenize(old);
    let b = tokenize(new);

    if a.len().saturating_mul(b.len()) > MAX_CELLS {
        let mut out = Vec::new();
        if !old.is_empty() {
            out.push((Op::Delete, old.to_string()));
        }
        if !new.is_empty() {
            out.push((Op::Insert, new.to_string()));
        }
        return out;
    }

    let (n, m) = (a.len(), b.len());
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[idx(i, j)] = if a[i] == b[j] {
                lcs[idx(i + 1, j + 1)] + 1
            } else {
                lcs[idx(i + 1, j)].max(lcs[idx(i, j + 1)])
            };
        }
    }

    let mut out: Vec<(Op, String)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            push(&mut out, Op::Equal, &a[i]);
            i += 1;
            j += 1;
        } else if lcs[idx(i + 1, j)] >= lcs[idx(i, j + 1)] {
            push(&mut out, Op::Delete, &a[i]);
            i += 1;
        } else {
            push(&mut out, Op::Insert, &b[j]);
            j += 1;
        }
    }
    while i < n {
        push(&mut out, Op::Delete, &a[i]);
        i += 1;
    }
    while j < m {
        push(&mut out, Op::Insert, &b[j]);
        j += 1;
    }
    out
}

/// Aufeinanderfolgende Token gleicher Operation zusammenfassen.
fn push(out: &mut Vec<(Op, String)>, op: Op, token: &str) {
    if let Some((last_op, text)) = out.last_mut() {
        if *last_op == op {
            text.push_str(token);
            return;
        }
    }
    out.push((op, token.to_string()));
}

fn tokenize(s: &str) -> Vec<String> {
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        None,
        Word,
        Space,
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut kind = Kind::None;
    for c in s.chars() {
        // Zeilenumbrüche und Interpunktion sind immer eigene Token.
        if c == '\n' || !(c.is_alphanumeric() || c.is_whitespace()) {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
            kind = Kind::None;
            continue;
        }
        let k = if c.is_whitespace() { Kind::Space } else { Kind::Word };
        if k != kind && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        kind = k;
        current.push(c);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(diff: &[(Op, String)]) -> Vec<(Op, &str)> {
        diff.iter().map(|(op, s)| (*op, s.as_str())).collect()
    }

    #[test]
    fn identical_text_is_all_equal() {
        let d = word_diff("hallo welt", "hallo welt");
        assert_eq!(ops(&d), vec![(Op::Equal, "hallo welt")]);
    }

    #[test]
    fn word_replacement_is_delete_plus_insert() {
        let d = word_diff("a b c", "a x c");
        assert_eq!(
            ops(&d),
            vec![
                (Op::Equal, "a "),
                (Op::Delete, "b"),
                (Op::Insert, "x"),
                (Op::Equal, " c"),
            ]
        );
    }

    #[test]
    fn newlines_are_separate_tokens() {
        let d = word_diff("a\nb", "a\nc");
        assert_eq!(
            ops(&d),
            vec![
                (Op::Equal, "a\n"),
                (Op::Delete, "b"),
                (Op::Insert, "c"),
            ]
        );
    }

    #[test]
    fn empty_old_text_is_pure_insert() {
        let d = word_diff("", "neu");
        assert_eq!(ops(&d), vec![(Op::Insert, "neu")]);
    }
}
