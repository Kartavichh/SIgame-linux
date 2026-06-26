//! HTTP-раздача медиа пака для клиентов.
//!
//! Медиаплеер WebKitGTK не воспроизводит видео/аудио по схеме `asset://`
//! (см. `docs/architecture.md`), поэтому в сетевой игре сервер отдаёт медиа по
//! обычному HTTP — так же, как клиент делал при локальном просмотре. Файлы
//! берутся из памяти (байты пака), с поддержкой заголовка `Range` (перемотка).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Запустить фоновый HTTP-сервер медиа на `0.0.0.0:<port>`.
pub fn start(media: BTreeMap<String, Vec<u8>>, port: u16) -> std::io::Result<()> {
    let server = tiny_http::Server::http(("0.0.0.0", port))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let media = Arc::new(media);
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            handle(request, &media);
        }
    });
    Ok(())
}

fn handle(request: tiny_http::Request, media: &BTreeMap<String, Vec<u8>>) {
    let raw = request.url().trim_start_matches('/').to_string();
    let name = percent_decode(&raw);

    if name.is_empty() || name.contains("..") {
        let _ = request.respond(tiny_http::Response::from_string("bad request").with_status_code(400));
        return;
    }

    let data = match media.get(&name) {
        Some(d) => d,
        None => {
            let _ = request.respond(tiny_http::Response::from_string("not found").with_status_code(404));
            return;
        }
    };
    let len = data.len() as u64;

    let ctype = content_type(&name);
    let ctype_h = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()).unwrap();
    let ranges_h = tiny_http::Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap();

    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());

    if let Some((start, end)) = range.as_deref().and_then(|r| parse_range(r, len)) {
        let slice = data[start as usize..=end as usize].to_vec();
        let cr = format!("bytes {start}-{end}/{len}");
        let cr_h = tiny_http::Header::from_bytes(&b"Content-Range"[..], cr.as_bytes()).unwrap();
        let resp = tiny_http::Response::from_data(slice)
            .with_status_code(206)
            .with_header(ctype_h)
            .with_header(ranges_h)
            .with_header(cr_h);
        let _ = request.respond(resp);
    } else {
        let resp = tiny_http::Response::from_data(data.clone())
            .with_header(ctype_h)
            .with_header(ranges_h);
        let _ = request.respond(resp);
    }
}

/// Разбирает заголовок `Range: bytes=start-end` в пару (start, end) включительно.
fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some((len.saturating_sub(n), len - 1))
    } else {
        let start: u64 = start_s.parse().ok()?;
        let end: u64 = if end_s.is_empty() { len - 1 } else { end_s.parse().ok()? };
        if start > end || start >= len {
            return None;
        }
        Some((start, end.min(len - 1)))
    }
}

fn content_type(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "webm" => "video/webm",
        "mp4" => "video/mp4",
        "ogv" => "video/ogg",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Простое percent-декодирование URL-пути (`%20`, кириллица и т.п.).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
