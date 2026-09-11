//! Modulares Skill-System: eigene Prompt-Skills als Markdown-Dateien.
//!
//! Jede Datei `<name>.md` in einem Skill-Ordner wird zum Befehl
//! `/<name>`; der Dateiinhalt ist der System-Prompt, der auf den Puffer
//! angewendet wird (Ergebnis erscheint wie alle Transformationen als
//! Diff-Vorschau). Optionales YAML-Frontmatter mit `description:` wird
//! für /help genutzt.
//!
//! Suchreihenfolge (spätere überschreiben frühere):
//!   1. `$XDG_CONFIG_HOME/vibe/skills` bzw. `~/.config/vibe/skills`
//!   2. `./.vibe/skills` (projektlokal)
//!   3. `$VIBE_SKILLS_DIR`

use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt: String,
}

pub fn skill_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")));
    if let Some(base) = config_home {
        dirs.push(base.join("vibe").join("skills"));
    }
    dirs.push(PathBuf::from(".vibe").join("skills"));
    if let Ok(extra) = std::env::var("VIBE_SKILLS_DIR") {
        dirs.push(PathBuf::from(extra));
    }
    dirs
}

pub fn load_skills() -> BTreeMap<String, Skill> {
    let mut skills = BTreeMap::new();
    for dir in skill_dirs() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for item in read.flatten() {
            let path = item.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Some(skill) = parse_skill(&path, &raw) {
                        skills.insert(skill.name.clone(), skill);
                    }
                }
            }
        }
    }
    skills
}

fn parse_skill(path: &std::path::Path, raw: &str) -> Option<Skill> {
    let name = path.file_stem()?.to_string_lossy().to_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let (description, prompt) = split_frontmatter(raw);
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return None;
    }
    Some(Skill { name, description, prompt })
}

/// Liefert (description, body). Frontmatter ist optional.
fn split_frontmatter(raw: &str) -> (String, String) {
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some((fm, body)) = rest.split_once("\n---\n") {
            let description = fm
                .lines()
                .find_map(|l| l.strip_prefix("description:"))
                .map(|d| d.trim().trim_matches('"').to_string())
                .unwrap_or_default();
            return (description, body.to_string());
        }
    }
    (String::new(), raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_skill_with_frontmatter() {
        let s = parse_skill(
            std::path::Path::new("/x/Tone-Check.md"),
            "---\ndescription: \"Prüft den Ton\"\n---\nYou check tone.",
        )
        .unwrap();
        assert_eq!(s.name, "tone-check");
        assert_eq!(s.description, "Prüft den Ton");
        assert_eq!(s.prompt, "You check tone.");
    }

    #[test]
    fn parses_skill_without_frontmatter() {
        let s = parse_skill(std::path::Path::new("simplify.md"), "Simplify the text.").unwrap();
        assert_eq!(s.name, "simplify");
        assert!(s.description.is_empty());
    }

    #[test]
    fn rejects_empty_or_invalid_names() {
        assert!(parse_skill(std::path::Path::new("empty.md"), "   ").is_none());
        assert!(parse_skill(std::path::Path::new("bad name.md"), "x").is_none());
    }
}
