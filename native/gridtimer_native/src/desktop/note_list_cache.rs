// List rows own only presentation data. Editors always resolve the full note by ID.
#[derive(Clone, Debug)]
struct DesktopNoteListItem {
    id: String,
    title: String,
    row_title: String,
    folder_id: Option<String>,
    body_preview: String,
    metadata_prefix: String,
    search_snippet: String,
    encrypted: bool,
    is_database: bool,
    updated_at_epoch_millis: i64,
}

impl DesktopNoteListItem {
    fn from_note(note: &DesktopNote, locked: bool, search_text: Option<&str>, query: &str) -> Self {
        let encrypted = note.encryption.is_some();
        let body = if locked {
            String::new()
        } else {
            note_body_text(note)
        };
        let title = if locked {
            "加密页面".to_string()
        } else {
            note.title.clone()
        };
        let row_title = if title.trim().is_empty() {
            preview_text(&body, "未命名")
        } else {
            title.clone()
        };
        let search_snippet = if locked {
            String::new()
        } else {
            search_text
                .unwrap_or_default()
                .lines()
                .filter(|line| query.is_empty() || line.to_lowercase().contains(query))
                .take(2)
                .map(|line| line.chars().take(95).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Self {
            id: note.id.clone(),
            title,
            row_title,
            folder_id: note.folder_id.clone(),
            body_preview: preview_text(&body, ""),
            metadata_prefix: note_metadata_prefix(note, &body),
            search_snippet,
            encrypted,
            is_database: !encrypted
                && note
                    .document
                    .knowledge
                    .as_ref()
                    .is_some_and(|meta| meta.database.is_some()),
            updated_at_epoch_millis: note.updated_at_epoch_millis,
        }
    }

    fn preview<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.body_preview.is_empty() {
            fallback
        } else {
            &self.body_preview
        }
    }

    fn metadata(&self) -> String {
        format!(
            "{}{}",
            self.metadata_prefix,
            format_relative_time(self.updated_at_epoch_millis)
        )
    }
}
