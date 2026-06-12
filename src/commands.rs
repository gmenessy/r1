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
    ("help", "Diese Übersicht"),
    ("quit", "Editor beenden"),
];

pub fn parse(input: &str) -> Result<Command, String> {
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
        "help" => Ok(Command::Help),
        "quit" | "q" | "exit" => Ok(Command::Quit),
        other => Err(format!("Unbekannter Befehl: /{other} – /help zeigt alle")),
    }
}

/// Vervollständigungen für den aktuellen Eingabe-Präfix.
pub fn completions(input: &str) -> Vec<&'static str> {
    let prefix = input.trim_start_matches('/');
    let word = prefix.split_whitespace().next().unwrap_or("");
    COMMANDS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| name.split(' ').next().unwrap_or("").starts_with(word))
        .collect()
}

/// Erster Treffer als Text, der die Eingabe ersetzt (für Tab).
pub fn complete_first(input: &str) -> Option<String> {
    completions(input)
        .first()
        .map(|c| c.split(' ').next().unwrap_or(c).to_string())
}

pub fn help_text() -> String {
    COMMANDS
        .iter()
        .map(|(name, desc)| format!("/{name} – {desc}"))
        .collect::<Vec<_>>()
        .join("  ·  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lang_with_argument() {
        assert!(matches!(parse("/lang en"), Ok(Command::Lang(l)) if l == "en"));
    }

    #[test]
    fn rejects_lang_without_argument() {
        assert!(parse("/lang").is_err());
    }

    #[test]
    fn parses_save_with_and_without_title() {
        assert!(matches!(parse("save"), Ok(Command::Save(None))));
        assert!(matches!(parse("save Mein Titel"), Ok(Command::Save(Some(t))) if t == "Mein Titel"));
    }

    #[test]
    fn parses_export_with_format_and_optional_target() {
        use crate::export::Format;
        assert!(matches!(
            parse("/export pdf"),
            Ok(Command::Export { format: Format::Pdf, target: None })
        ));
        assert!(matches!(
            parse("/export word mein brief"),
            Ok(Command::Export { format: Format::Docx, target: Some(t) }) if t == "mein brief"
        ));
        assert!(parse("/export").is_err());
        assert!(parse("/export xlsx").is_err());
    }

    #[test]
    fn completes_prefixes() {
        assert!(completions("/su").contains(&"summarize"));
        assert_eq!(complete_first("/sum"), Some("summarize".to_string()));
    }
}
