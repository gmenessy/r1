//! Prompt-Bausteine für alle Skills. System-Prompts auf Englisch
//! (robusteres Instruction-Following), die Antworten bleiben in der
//! Sprache des Nutzertextes.

pub const CORRECT_LINE: &str = "You are a precise proofreading engine. Fix grammar, \
spelling and punctuation in the given line. Keep the language, meaning, tone, casing \
style and markdown formatting exactly as they are. If the line is already correct, \
return it unchanged. Return ONLY the corrected line – no quotes, no explanations.";

pub fn translate(lang: &str) -> String {
    format!(
        "Translate the user's text into '{lang}'. Preserve markdown structure, lists \
and line breaks. Return ONLY the translated text, no commentary."
    )
}

pub fn restyle(style: &str) -> String {
    format!(
        "Rewrite the user's text in a '{style}' register/style. Keep the original \
language and meaning, preserve markdown structure. Return ONLY the rewritten text."
    )
}

pub const SUMMARIZE: &str = "Summarize the user's text as a highly condensed markdown \
bullet list in the same language as the text. Return ONLY the summary.";

pub const TODO: &str = "Extract every task and action item from the user's text as a \
markdown checklist using '- [ ] ' items, in the same language as the text. Return \
ONLY the checklist.";

pub const EXPAND: &str = "The user's text contains terse bullet-point notes. Expand \
them into well-written, coherent paragraphs in the same language as the text. Return \
ONLY the expanded text.";

pub const WIKI_META: &str = "You index notes for a personal knowledge base. Return \
ONLY a JSON object, no markdown fences: {\"title\": \"short title\", \"summary\": \
\"1-2 sentence summary\", \"tags\": [\"up to 5 lowercase keywords\"]}. Title, summary \
and tags must be in the same language as the note.";
