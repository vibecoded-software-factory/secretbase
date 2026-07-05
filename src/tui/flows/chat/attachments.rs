//! Attachment flows: downloads (with traversal-safe naming), uploads,
//! and the inline image preview pipeline.

use crate::ports::KeybaseError;
use crate::tui::action::ActionState;
use crate::tui::app::App;
use crate::tui::worker::{InFlight, WorkerRequest};

#[allow(unused_imports)]
use super::*;

// ── Attachment download ──────────────────────────────────────────────

/// Reduces an attachment filename emitted by the chat server to a
/// single safe basename that can be concatenated with `~/Downloads/`
/// without escaping the directory.
///
/// The `filename` field on an attachment is set by whoever uploaded
/// it. A malicious sender could pick something like
/// `"../../.ssh/authorized_keys"` to land the file outside the
/// expected destination once the user accepts the download. We
/// defend by:
///
/// * Stripping every path component (`/`, `\`) and keeping only the
///   final segment.
/// * Filtering control characters and NULs (which some filesystems
///   accept but would surprise the user / confuse logs).
/// * Falling back to a synthetic name when the sanitised value is
///   empty, `"."`, or `".."`.
///
/// The user can still edit the proposed path freely in the download
/// popup — this only secures the *default* the popup pre-fills with.
pub(crate) fn safe_attachment_basename(raw: &str, msg_id: u64) -> String {
    // `rsplit` over the path separators always yields at least one
    // item, so `next()` is infallible; the `unwrap_or` is for the
    // type-checker.
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return format!("attachment-{msg_id}.bin");
    }
    trimmed.to_string()
}

pub fn open_download_for_selected(app: &mut App) {
    use crate::domain::MessageContent;
    let Some(idx) = app.select.cursor else {
        app.set_action(ActionState::Error("No message selected".into()));
        return;
    };
    let Some(msg) = app.messages.get(idx) else {
        return;
    };
    let MessageContent::Attachment(att) = &msg.content else {
        app.set_action(ActionState::Error(
            "Selected message is not an attachment".into(),
        ));
        return;
    };
    let filename = safe_attachment_basename(&att.filename, msg.id);
    app.picker_action = crate::tui::app::PickerAction::Download {
        message_id: msg.id,
        filename,
    };
    // Pick the destination directory, starting at the user's Downloads.
    app.file_picker = Some(crate::tui::file_picker::FilePicker::new_dir(
        &default_download_dir(),
    ));
}

/// The OS default Downloads directory: `$XDG_DOWNLOAD_DIR`, then
/// `~/Downloads`, then `/` as a last resort.
fn default_download_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(d) = std::env::var_os("XDG_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
    {
        return d;
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    PathBuf::from("/")
}

/// Downloads attachment `message_id` from the open conversation into
/// `dir`, saved under `filename`. Fired when the directory picker returns.
pub fn request_download_to(
    app: &mut App,
    message_id: u64,
    dir: std::path::PathBuf,
    filename: String,
) {
    let output = dir.join(&filename).to_string_lossy().to_string();
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    app.submit(
        InFlight::DownloadAttachment {
            message_id,
            path: output.clone(),
        },
        &format!("Downloading {filename}…"),
        WorkerRequest::DownloadAttachment {
            channel,
            message_id,
            output,
        },
    );
}

/// Cache directory for downloaded inline-preview images
/// (`$XDG_CACHE_HOME/secretbase/images`, falling back to `~/.cache` / tmp).
pub fn image_cache_dir() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("secretbase").join("images")
}

/// Stable cache path for an attachment image, `{conv}-{msg}.{ext}` — unique
/// per conversation so per-conversation message ids can't collide.
pub fn image_path_for(conv_id: &str, msg_id: u64, filename: &str) -> String {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("img");
    image_cache_dir()
        .join(format!("{conv_id}-{msg_id}.{ext}"))
        .to_string_lossy()
        .to_string()
}

/// Enqueues background downloads for every visible image that isn't already
/// cached, in flight, or known-failed. Called from the run loop after each
/// draw (the view fills `image_to_fetch` with `(message_id, cache path)`).
pub fn ensure_visible_images(app: &mut App) {
    if app.images.to_fetch.is_empty() {
        return;
    }
    let to_fetch = std::mem::take(&mut app.images.to_fetch);
    let Some(conv_id) = app.open_conv_id.clone() else {
        return;
    };
    let Some(conv) = app.conversations.iter().find(|c| c.id == conv_id) else {
        return;
    };
    let Ok(channel) = read_channel_from_conv(conv) else {
        return;
    };
    let _ = std::fs::create_dir_all(image_cache_dir());
    for (msg_id, output) in to_fetch {
        if !app.images.needs_download(&output) {
            continue;
        }
        // Reuse a file fetched in an earlier session rather than re-downloading.
        if std::path::Path::new(&output).exists() {
            app.images.adopt_cached_file(output);
            continue;
        }
        app.images.pending.insert(output.clone());
        // Web media (giphy) goes to the web fetcher on the background lane
        // — a slow CDN must never queue ahead of user ops; attachments use
        // the keybase download as before.
        if let Some(url) = app.images.web_urls.get(&output).cloned() {
            let _ = app
                .bg_worker_tx
                .send(WorkerRequest::FetchWebImage { url, output });
        } else {
            let _ = app.worker_tx.send(WorkerRequest::PreviewImage {
                channel: channel.clone(),
                message_id: msg_id,
                output,
            });
        }
    }
}

/// Cache path for a fetched **web media** file — content-addressed by the
/// URL (stable across sessions, no conversation coupling), always `.gif`
/// (the only rendition the pipeline fetches).
pub fn web_image_path_for(url: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    image_cache_dir()
        .join(format!("web-{:016x}.gif", h.finish()))
        .to_string_lossy()
        .to_string()
}

/// Whether the single selected message is an image whose file is on disk —
/// returns its `(cache path, mime)` so `c` can copy the image itself.
fn selected_ready_image(app: &App) -> Option<(String, String)> {
    if !app.select.marks.is_empty() {
        return None; // multi-select copies text, not a single image
    }
    let m = app.messages.get(app.select.cursor?)?;
    let crate::domain::MessageContent::Attachment(att) = &m.content else {
        return None;
    };
    if !crate::tui::image::is_image(&att.mime_type, &att.filename) {
        return None;
    }
    let conv_id = app.open_conv_id.as_deref()?;
    let path = image_path_for(conv_id, m.id, &att.filename);
    if app.images.ready.contains(&path) || std::path::Path::new(&path).exists() {
        let mime = if att.mime_type.is_empty() {
            "image/png".to_string()
        } else {
            att.mime_type.clone()
        };
        Some((path, mime))
    } else {
        None
    }
}

/// `c` in select mode: copy the **image** to the clipboard when a downloaded
/// image is selected, otherwise copy the message body text.
pub fn do_copy_content(app: &mut App) {
    let Some((path, mime)) = selected_ready_image(app) else {
        do_copy_messages(app, false);
        return;
    };
    match crate::tui::image::copy_to_clipboard(&path, &mime) {
        Ok(()) => {
            app.push_cmd("copy image", true, &path);
            app.set_action(ActionState::Done("Image copied to clipboard".into()));
        }
        Err(e) => {
            // Log the failure too — the toast expires in seconds, and every
            // other op records both outcomes in the command log.
            app.push_cmd("copy image", false, e.to_string());
            app.set_action(ActionState::Error(format!("Copy failed: {e}")));
        }
    }
}

/// Applies a finished background image download: mark the cache path ready (or
/// failed) and flag a repaint.
pub fn handle_preview_image_response(
    app: &mut App,
    path: String,
    result: Result<(), KeybaseError>,
) {
    app.images.on_download_finished(path, result.is_ok());
}

/// Enqueues background GIF decodes for every visible GIF that's downloaded but
/// not yet decoded (and not already in flight). Called from the run loop after
/// each draw; the view fills `gif_to_decode` with cache paths. Runs on the
/// background lane so a multi-second decode never stalls the user's keybase
/// calls on the main worker.
pub fn ensure_pending_gif_decodes(app: &mut App) {
    if app.images.to_decode.is_empty() {
        return;
    }
    let to_decode = std::mem::take(&mut app.images.to_decode);
    for path in to_decode {
        if !app.images.needs_decode(&path) {
            continue;
        }
        app.images.decoding.insert(path.clone());
        let _ = app.bg_worker_tx.send(WorkerRequest::DecodeGif { path });
    }
}

/// Stores a finished off-thread GIF decode: `Some(frames)` for an animated GIF,
/// `None` for a still / single-frame GIF (then rendered as a static image).
/// Clears the pending flag and flags a repaint so the skeleton gives way.
pub fn handle_decode_gif_response(
    app: &mut App,
    path: String,
    frames: Option<crate::tui::image::GifFrames>,
) {
    app.images.on_decode_finished(path, frames);
}

pub fn handle_download_attachment_response(
    app: &mut App,
    result: Result<(), KeybaseError>,
    message_id: u64,
    path: String,
) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Saved to {path}")));
            app.push_cmd(
                "keybase chat api download",
                true,
                format!("msg #{message_id} → {path}"),
            );
            app.select.cursor = None;
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api download", false, e.to_string());
        }
    }
}

// ── Upload attachment ────────────────────────────────────────────────

/// Uploads a local file to the open conversation as an attachment
/// (`keybase chat api {"method":"attach"}`). The path comes from the
/// embedded file picker.
pub fn request_upload_attachment(app: &mut App, path: std::path::PathBuf) {
    let Some((_, channel)) = open_channel(app) else {
        return;
    };
    let filename = path.to_string_lossy().to_string();
    let display = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| filename.clone());
    app.submit(
        InFlight::UploadAttachment {
            filename: display.clone(),
        },
        &format!("Uploading {display}…"),
        WorkerRequest::UploadAttachment {
            channel,
            filename,
            title: String::new(),
        },
    );
}

pub fn handle_upload_response(app: &mut App, result: Result<(), KeybaseError>, filename: String) {
    match result {
        Ok(()) => {
            app.set_action(ActionState::Done(format!("Uploaded {filename}")));
            app.push_cmd("keybase chat api attach", true, filename);
            // Re-read so the new attachment message appears.
            app.pagination.scroll = 0;
            request_load_messages(app);
        }
        Err(e) => {
            app.set_action(ActionState::Error(e.to_string()));
            app.push_cmd("keybase chat api attach", false, e.to_string());
        }
    }
}
