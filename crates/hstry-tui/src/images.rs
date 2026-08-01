//! Inline image support: extract image parts from messages and view them via
//! the terminal graphics protocol (kitty/iterm2/sixel, halfblocks fallback).

use base64::Engine;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;

use hstry_core::Database;
use hstry_core::parts::{MediaSource, Part};

/// Detect terminal graphics support without touching stdin.
///
/// `Picker::from_query_stdio` leaves a blocked reader thread behind when the
/// terminal never answers (e.g. under tmux); that thread later swallows
/// keypresses and drops raw mode. Env + ioctl detection avoids that entirely.
// `from_fontsize` is deprecated in favor of `from_query_stdio`, which we
// deliberately avoid (see above).
#[allow(deprecated)]
pub fn detect_picker() -> Picker {
    let font_size = crossterm::terminal::window_size()
        .ok()
        .filter(|ws| ws.width > 0 && ws.height > 0 && ws.columns > 0 && ws.rows > 0)
        .map_or((10, 20), |ws| (ws.width / ws.columns, ws.height / ws.rows));
    let mut picker = Picker::from_fontsize(font_size.into());

    let term = std::env::var("TERM").unwrap_or_default();
    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let kitty = std::env::var("KITTY_WINDOW_ID").is_ok()
        || term.contains("kitty")
        || term.contains("ghostty");
    let iterm = term_program == "iTerm.app"
        || term_program == "WezTerm"
        || std::env::var("WEZTERM_EXECUTABLE").is_ok();

    if kitty {
        picker.set_protocol_type(ProtocolType::Kitty);
    } else if iterm {
        picker.set_protocol_type(ProtocolType::Iterm2);
    } else {
        picker.set_protocol_type(ProtocolType::Halfblocks);
    }
    picker
}

/// One image found in the loaded conversation.
#[derive(Clone)]
pub struct ImageEntry {
    pub label: String,
    pub source: MediaSource,
}

/// Extract image parts from a message's `parts_json`.
pub fn extract_images(parts_json: &serde_json::Value) -> Vec<ImageEntry> {
    let Ok(parts) = serde_json::from_value::<Vec<Part>>(parts_json.clone()) else {
        return Vec::new();
    };
    parts
        .into_iter()
        .filter_map(|p| match p {
            Part::Image { source, alt, .. } => Some(ImageEntry {
                label: alt.unwrap_or_else(|| "image".to_string()),
                source,
            }),
            _ => None,
        })
        .collect()
}

pub struct ViewerState {
    pub index: usize,
    pub protocol: Option<StatefulProtocol>,
    pub error: Option<String>,
}

impl ViewerState {
    pub fn open(
        entries: &[ImageEntry],
        index: usize,
        picker: &Picker,
        db: &Database,
        rt: &tokio::runtime::Runtime,
    ) -> Self {
        let mut state = Self {
            index,
            protocol: None,
            error: None,
        };
        state.load(entries, picker, db, rt);
        state
    }

    pub fn load(
        &mut self,
        entries: &[ImageEntry],
        picker: &Picker,
        db: &Database,
        rt: &tokio::runtime::Runtime,
    ) {
        self.protocol = None;
        self.error = None;

        let Some(entry) = entries.get(self.index) else {
            self.error = Some("No image".to_string());
            return;
        };

        let bytes = match &entry.source {
            MediaSource::AttachmentRef { attachment_id, .. } => {
                match rt.block_on(db.get_attachment(attachment_id)) {
                    Ok(Some((_mime, bytes))) => bytes,
                    Ok(None) => {
                        self.error = Some(format!("Attachment '{attachment_id}' not found"));
                        return;
                    }
                    Err(e) => {
                        self.error = Some(format!("Attachment load error: {e}"));
                        return;
                    }
                }
            }
            MediaSource::Base64 { data, .. } => {
                match base64::engine::general_purpose::STANDARD.decode(data.trim()) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        self.error = Some(format!("Base64 decode error: {e}"));
                        return;
                    }
                }
            }
            MediaSource::Url { url, .. } => {
                if let Some(path) = url.strip_prefix("file://") {
                    match std::fs::read(path) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            self.error = Some(format!("Read error: {e}"));
                            return;
                        }
                    }
                } else {
                    self.error = Some(format!("Remote URL not fetched: {url}"));
                    return;
                }
            }
        };

        match image::load_from_memory(&bytes) {
            Ok(img) => self.protocol = Some(picker.new_resize_protocol(img)),
            Err(e) => self.error = Some(format!("Image decode error: {e}")),
        }
    }

    pub fn next(
        &mut self,
        entries: &[ImageEntry],
        picker: &Picker,
        db: &Database,
        rt: &tokio::runtime::Runtime,
    ) {
        if entries.is_empty() {
            return;
        }
        self.index = (self.index + 1) % entries.len();
        self.load(entries, picker, db, rt);
    }

    pub fn previous(
        &mut self,
        entries: &[ImageEntry],
        picker: &Picker,
        db: &Database,
        rt: &tokio::runtime::Runtime,
    ) {
        if entries.is_empty() {
            return;
        }
        self.index = (self.index + entries.len() - 1) % entries.len();
        self.load(entries, picker, db, rt);
    }
}
