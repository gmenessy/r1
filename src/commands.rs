//! Intent-Driven Command Bar: Parsen + Autovervollständigung.

#[derive(Debug, Clone)]
pub enum Command {
    Lang(String),
    Style(String),
    Model(String),
    Summarize,
    Todo,
    Expand,
    Save(Option<String>),
    Export {
        format: crate::export::Format,
        target: Option<String>,
    },
    Open(String),
    Write(Option<String>),
    Ghost(bool),
    /// Benutzerdefinierter Prompt-Skill (siehe `skills.rs`).
    Skill(String),
    Help,
    Quit,
}

/// (Name, Beschreibung) – Quelle für Autocomplete und /help.
pub const COMMANDS: &[(&str, &str)] = &[
    ("lang <code>", "Puffer übersetzen, z. B. /lang en"),
    ("style <stil>", "Register umformen, z. B. /style professional"),
    ("model <name>", "Modell wechseln: gemma | claude | gpt"),
    ("summarize", "Hochverdichtete Zusammenfassung"),
    ("todo", "Action-Items als Checkliste extrahieren"),
    ("expand", "Stichpunkte ausformulieren"),
    ("save [titel]", "Puffer ins Wiki speichern (Auto-Tags)"),
    ("export <format> [datei]", "Puffer exportieren: pdf|docx|html|md|txt"),
    ("open <datei>", "Datei in den Puffer laden"),
    ("write [datei]", "Puffer in Datei schreiben"),
    ("ghost on|off", "Ghost-Text-Korrekturen an-/abschalten"),
    ("help", "Diese Übersicht"),
    ("quit", "Editor beenden"),
];

/// `skills` sind die Namen der geladenen Custom-Skills – sie werden als
/// eigene Befehle akzeptiert, aber eingebaute Befehle haben Vorrang.
pub fn parse(input: &str, skills: &[String]) -> Result<Command, String> {
    let s = input.trim().trim_start_matches('/');
    let mut parts = s.split_whitespace();
    let head = parts.next().ok_or_else(|| "Leerer Befehl".to_string())?;
    let rest = parts.collect::<Vec<_>>().join(" ");
    match head {
        "lang" if !rest.is_empty() => Ok(Command::Lang(rest)),
        "lang" => Err("Nutzung: /lang <sprachcode>, z. B. /lang en".into()),
        "style" if !rest.is_empty() => Ok(Command::Style(rest)),
        "style" => Err("Nutzung: /style <stil>, z. B. /style professional".into()),
        "model" if !rest.is_empty() => Ok(Command::Model(rest.to_lowercase())),
        "model" => Err("Nutzung: /model gemma|claude|gpt".into()),
        "summarize" => Ok(Command::Summarize),
        "todo" => Ok(Command::Todo),
        "expand" => Ok(Command::Expand),
        "save" => Ok(Command::Save(if rest.is_empty() { None } else { Some(rest) })),
        "export" => {
            let mut args = rest.split_whitespace();
            let format_arg = args
                .next()
                .ok_or("Nutzung: /export pdf|docx|html|md|txt [datei]")?;
            let format = crate::export::Format::parse(format_arg)?;
            let target = args.collect::<Vec<_>>().join(" ");
            Ok(Command::Export {
                format,
                target: if target.is_empty() { None } else { Some(target) },
            })
        }
        "open" if !rest.is_empty() => Ok(Command::Open(rest)),
        "open" => Err("Nutzung: /open <datei>".into()),
        "write" | "w" => Ok(Command::Write(if rest.is_empty() { None } else { Some(rest) })),
        "ghost" => match rest.as_str() {
            "on" | "an" => Ok(Command::Ghost(true)),
            "off" | "aus" => Ok(Command::Ghost(false)),
            _ => Err("Nutzung: /ghost on|off".into()),
        },
        "help" => Ok(Command::Help),
        "quit" | "q" | "exit" => Ok(Command::Quit),
        other if skills.iter().any(|s| s == other) => Ok(Command::Skill(other.to_string())),
        other => Err(format!("Unbekannter Befehl: /{other} – /help zeigt alle")),
    }
}

/// Vervollständigungen für den aktuellen Eingabe-Präfix (eingebaut + Skills).
pub fn completions(input: &str, skills: &[String]) -> Vec<String> {
    let prefix = input.trim_start_matches('/');
    let word = prefix.split_whitespace().next().unwrap_or("");
    let mut out: Vec<String> = COMMANDS
        .iter()
        .map(|(name, _)| name.to_string())
        .filter(|name| name.split(' ').next().unwrap_or("").starts_with(word))
        .collect();
    out.extend(skills.iter().filter(|s| s.starts_with(word)).cloned());
    out
}

/// Erster Treffer als Text, der die Eingabe ersetzt (für Tab).
pub fn complete_first(input: &str, skills: &[String]) -> Option<String> {
    completions(input, skills)
        .first()
        .map(|c| c.split(' ').next().unwrap_or(c).to_string())
}

pub fn help_text(skills: &[String]) -> String {
    let mut parts: Vec<String> = COMMANDS
        .iter()
        .map(|(name, desc)| format!("/{name} – {desc}"))
        .collect();
    if !skills.is_empty() {
        parts.push(format!("Skills: /{}", skills.join(" /")));
    }
    parts.join("  ·  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_SKILLS: &[String] = &[];

    #[test]
    fn parses_lang_with_argument() {
        assert!(matches!(parse("/lang en", NO_SKILLS), Ok(Command::Lang(l)) if l == "en"));
    }

    #[test]
    fn rejects_lang_without_argument() {
        assert!(parse("/lang", NO_SKILLS).is_err());
    }

    #[test]
    fn parses_save_with_and_without_title() {
        assert!(matches!(parse("save", NO_SKILLS), Ok(Command::Save(None))));
        assert!(matches!(
            parse("save Mein Titel", NO_SKILLS),
            Ok(Command::Save(Some(t))) if t == "Mein Titel"
        ));
    }

    #[test]
    fn parses_export_with_format_and_optional_target() {
        use crate::export::Format;
        assert!(matches!(
            parse("/export pdf", NO_SKILLS),
            Ok(Command::Export { format: Format::Pdf, target: None })
        ));
        assert!(matches!(
            parse("/export word mein brief", NO_SKILLS),
            Ok(Command::Export { format: Format::Docx, target: Some(t) }) if t == "mein brief"
        ));
        assert!(parse("/export", NO_SKILLS).is_err());
        assert!(parse("/export xlsx", NO_SKILLS).is_err());
    }

    #[test]
    fn parses_file_and_ghost_commands() {
        assert!(matches!(parse("/open notes.md", NO_SKILLS), Ok(Command::Open(p)) if p == "notes.md"));
        assert!(parse("/open", NO_SKILLS).is_err());
        assert!(matches!(parse("/w", NO_SKILLS), Ok(Command::Write(None))));
        assert!(matches!(parse("/ghost off", NO_SKILLS), Ok(Command::Ghost(false))));
        assert!(parse("/ghost maybe", NO_SKILLS).is_err());
    }

    #[test]
    fn custom_skills_are_commands_but_builtins_win() {
        let skills = vec!["simplify".to_string(), "todo".to_string()];
        assert!(matches!(parse("/simplify", &skills), Ok(Command::Skill(s)) if s == "simplify"));
        assert!(matches!(parse("/todo", &skills), Ok(Command::Todo)));
        assert!(parse("/simplify", NO_SKILLS).is_err());
    }

    #[test]
    fn completes_prefixes_including_skills() {
        let skills = vec!["summary-de".to_string()];
        let c = completions("/su", &skills);
        assert!(c.iter().any(|s| s == "summarize"));
        assert!(c.iter().any(|s| s == "summary-de"));
        assert_eq!(complete_first("/sum", NO_SKILLS), Some("summarize".to_string()));
    }
}
