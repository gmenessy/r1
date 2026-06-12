//! Export-Skills: Puffer als PDF, Word (docx), HTML, Markdown oder Text.
//!
//! Local-First auch hier: docx, html, md und txt entstehen komplett
//! offline in-process. Nur PDF delegiert an ein installiertes `pandoc` –
//! pure-Rust-PDF hätte eingebettete Fonts und ein Layout-Modul gekostet,
//! ohne die Qualität von pandoc zu erreichen.

use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use docx_rs::{Docx, Paragraph, Run};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Docx,
    Html,
    Markdown,
    Text,
}

impl Format {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "docx" | "word" | "doc" => Ok(Self::Docx),
            "html" => Ok(Self::Html),
            "md" | "markdown" => Ok(Self::Markdown),
            "txt" | "text" => Ok(Self::Text),
            other => Err(format!("Unbekanntes Format '{other}' – pdf|docx|html|md|txt")),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Html => "html",
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }
}

/// Exportiert den Puffer; `target` ist ein optionaler Wunsch-Dateiname.
/// Gibt den tatsächlichen Pfad zurück.
pub async fn export(format: Format, text: &str, target: Option<&str>) -> Result<PathBuf> {
    if text.trim().is_empty() {
        bail!("Puffer ist leer – nichts zu exportieren");
    }
    let path = resolve_path(format, text, target);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Export-Ordner {} anlegen", parent.display()))?;
        }
    }
    match format {
        Format::Markdown | Format::Text => {
            std::fs::write(&path, text)
                .with_context(|| format!("{} schreiben", path.display()))?;
        }
        Format::Html => {
            std::fs::write(&path, to_html(text))
                .with_context(|| format!("{} schreiben", path.display()))?;
        }
        Format::Docx => {
            let file = std::fs::File::create(&path)
                .with_context(|| format!("{} anlegen", path.display()))?;
            docx_from_markdown(text)
                .build()
                .pack(file)
                .map_err(|e| anyhow!("docx packen: {e}"))?;
        }
        Format::Pdf => pdf_via_pandoc(text, &path).await?,
    }
    Ok(path)
}

/// Zielpfad: Wunschname (Endung wird ergänzt) oder
/// `<slug-der-ersten-zeile>-<timestamp>.<ext>` in `VIBE_EXPORT_DIR` (./).
fn resolve_path(format: Format, text: &str, target: Option<&str>) -> PathBuf {
    let ext = format.extension();
    if let Some(name) = target {
        let mut path = PathBuf::from(name);
        if path.extension().is_none() {
            path.set_extension(ext);
        }
        return path;
    }
    let dir = std::env::var("VIBE_EXPORT_DIR").unwrap_or_else(|_| ".".into());
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("export")
        .trim_start_matches(['#', ' ']);
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(dir).join(format!("{}-{stamp}.{ext}", crate::wiki::slugify(first_line)))
}

fn to_html(text: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    let mut body = String::new();
    html::push_html(&mut body, Parser::new_ext(text, options));
    format!(
        "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>Vibe-Editor Export</title>\n<style>\n\
body {{ max-width: 46rem; margin: 3rem auto; padding: 0 1rem; \
font-family: Georgia, 'Times New Roman', serif; line-height: 1.6; color: #222; }}\n\
code, pre {{ font-family: ui-monospace, monospace; background: #f4f4f4; }}\n\
pre {{ padding: .75rem; overflow-x: auto; }}\n\
blockquote {{ border-left: 3px solid #ccc; margin-left: 0; padding-left: 1rem; color: #555; }}\n\
</style>\n</head>\n<body>\n{body}</body>\n</html>\n"
    )
}

/// Zeilenbasiertes Markdown → docx-Mapping (Überschriften, Listen, Absätze).
/// Bewusst ohne Inline-Formatierung – für volle Treue gibt es /export pdf.
fn docx_from_markdown(text: &str) -> Docx {
    let mut docx = Docx::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let paragraph = if let Some(rest) = trimmed.strip_prefix("# ") {
            heading(rest, 36)
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            heading(rest, 30)
        } else if let Some(rest) = trimmed.strip_prefix("### ") {
            heading(rest, 26)
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            Paragraph::new().add_run(Run::new().add_text(format!("•  {rest}")))
        } else {
            Paragraph::new().add_run(Run::new().add_text(line))
        };
        docx = docx.add_paragraph(paragraph);
    }
    docx
}

fn heading(text: &str, half_points: usize) -> Paragraph {
    Paragraph::new().add_run(Run::new().add_text(text).bold().size(half_points))
}

async fn pdf_via_pandoc(text: &str, path: &Path) -> Result<()> {
    let mut child = tokio::process::Command::new("pandoc")
        .args(["-f", "markdown", "-o"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            anyhow!(
                "pandoc nicht gefunden – PDF-Export benötigt pandoc (pandoc.org). \
Alternative: /export html und im Browser als PDF drucken"
            )
        })?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        bail!("pandoc: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_format_aliases() {
        assert_eq!(Format::parse("word"), Ok(Format::Docx));
        assert_eq!(Format::parse("PDF"), Ok(Format::Pdf));
        assert_eq!(Format::parse("markdown"), Ok(Format::Markdown));
        assert!(Format::parse("xlsx").is_err());
    }

    #[test]
    fn resolve_path_adds_missing_extension() {
        let p = resolve_path(Format::Docx, "text", Some("brief"));
        assert_eq!(p, PathBuf::from("brief.docx"));
        let p = resolve_path(Format::Pdf, "text", Some("a/b.pdf"));
        assert_eq!(p, PathBuf::from("a/b.pdf"));
    }

    #[test]
    fn default_filename_uses_first_line_slug() {
        let p = resolve_path(Format::Html, "# Mein Bericht\n\ntext", None);
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("mein-bericht-"), "{name}");
        assert!(name.ends_with(".html"));
    }

    #[test]
    fn html_export_renders_markdown() {
        let html = to_html("# Titel\n\n- punkt");
        assert!(html.contains("<h1>Titel</h1>"));
        assert!(html.contains("<li>punkt</li>"));
        assert!(html.contains("charset=\"utf-8\""));
    }

    #[test]
    fn docx_builds_without_error() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        docx_from_markdown("# Titel\n\n- punkt\nabsatz")
            .build()
            .pack(&mut cursor)
            .expect("docx pack");
        assert!(!cursor.into_inner().is_empty());
    }

    #[tokio::test]
    async fn markdown_export_roundtrip() {
        let dir = std::env::temp_dir().join("vibe-export-test");
        let target = dir.join("roundtrip.md");
        let path = export(Format::Markdown, "hallo", Some(target.to_str().unwrap()))
            .await
            .expect("export");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hallo");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn empty_buffer_is_rejected() {
        assert!(export(Format::Text, "   \n  ", None).await.is_err());
    }
}
