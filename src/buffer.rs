//! Minimaler, UTF-8-sicherer Zeilenpuffer mit Cursor.
//! Bewusst ohne Rope – für eine Workbench dieser Größe reicht Vec<String>,
//! und die UI bleibt dadurch trivial schnell.

pub struct Buffer {
    pub lines: Vec<String>,
    pub row: usize,
    /// Spalte als *Zeichen*-Index (nicht Bytes).
    pub col: usize,
}

fn byte_idx(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(i, _)| i)
        .unwrap_or(line.len())
}

fn char_len(line: &str) -> usize {
    line.chars().count()
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(|l| l.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.lines.len() - 1;
        self.col = char_len(&self.lines[self.row]);
    }

    pub fn current_line(&self) -> &str {
        &self.lines[self.row]
    }

    pub fn replace_line(&mut self, row: usize, content: &str) {
        if row < self.lines.len() {
            self.lines[row] = content.to_string();
            if self.row == row {
                self.col = self.col.min(char_len(&self.lines[row]));
            }
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let idx = byte_idx(&self.lines[self.row], self.col);
        self.lines[self.row].insert(idx, c);
        self.col += 1;
    }

    pub fn newline(&mut self) {
        let idx = byte_idx(&self.lines[self.row], self.col);
        let tail = self.lines[self.row].split_off(idx);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            let idx = byte_idx(&self.lines[self.row], self.col - 1);
            self.lines[self.row].remove(idx);
            self.col -= 1;
        } else if self.row > 0 {
            let removed = self.lines.remove(self.row);
            self.row -= 1;
            self.col = char_len(&self.lines[self.row]);
            self.lines[self.row].push_str(&removed);
        }
    }

    /// Entf: Zeichen unter dem Cursor bzw. Zeilenumbruch am Zeilenende.
    pub fn delete_forward(&mut self) {
        if self.col < char_len(&self.lines[self.row]) {
            let idx = byte_idx(&self.lines[self.row], self.col);
            self.lines[self.row].remove(idx);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = char_len(&self.lines[self.row]);
        }
    }

    pub fn move_right(&mut self) {
        if self.col < char_len(&self.lines[self.row]) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(char_len(&self.lines[self.row]));
        }
    }

    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(char_len(&self.lines[self.row]));
        }
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = char_len(&self.lines[self.row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_newline_roundtrip() {
        let mut b = Buffer::new();
        for c in "hällo".chars() {
            b.insert_char(c);
        }
        b.newline();
        b.insert_char('x');
        assert_eq!(b.text(), "hällo\nx");
        assert_eq!((b.row, b.col), (1, 1));
    }

    #[test]
    fn backspace_joins_lines_utf8() {
        let mut b = Buffer::new();
        b.set_text("aü\nb");
        b.row = 1;
        b.col = 0;
        b.backspace();
        assert_eq!(b.text(), "aüb");
        assert_eq!((b.row, b.col), (0, 2));
    }

    #[test]
    fn replace_line_clamps_cursor() {
        let mut b = Buffer::new();
        b.set_text("eine lange zeile");
        b.replace_line(0, "kurz");
        assert_eq!(b.text(), "kurz");
        assert!(b.col <= 4);
    }
}
