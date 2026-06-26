//! Клиент SIGame-RS (Tauri).
//!
//! Этап 2: загрузка `.sgpack` и показ его на табло, включая медиа.
//!
//! Медиа отдаём не через схему `asset://` (её медиаплеер WebKitGTK не понимает —
//! даёт SRC_NOT_SUPPORTED), а через встроенный локальный HTTP-сервер на
//! `127.0.0.1`. GStreamer умеет такой источник и поддерживает перемотку.

// На Windows в release-сборке прячем лишнее консольное окно.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use sigame_core::{Pack, PackArchive};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

/// Папка, куда распаковываются медиа открытого пака и откуда их отдаёт HTTP-сервер.
fn media_dir() -> PathBuf {
    std::env::temp_dir().join("sigame-rs-media")
}

/// Порт локального медиа-сервера (хранится в состоянии Tauri).
struct MediaServer {
    port: u16,
}

/// Демо-пак вшит в бинарь на этапе компиляции. Так кнопка «Открыть демо-пак»
/// работает и в собранном приложении (AppImage/.deb), где рядом нет исходников.
const DEMO_PACK: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo/demo.sgpack"));

/// Путь к демо-паку: распаковываем вшитые байты во временный файл и отдаём путь.
#[tauri::command]
fn demo_pack_path() -> Result<String, String> {
    let path = std::env::temp_dir().join("sigame-rs-demo.sgpack");
    std::fs::write(&path, DEMO_PACK).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Базовый URL медиа-сервера, например `http://127.0.0.1:54321/`.
#[tauri::command]
fn media_base_url(server: State<MediaServer>) -> String {
    format!("http://127.0.0.1:{}/", server.port)
}

/// Загрузить `.sgpack`: распарсить, проверить медиа, распаковать их в [`media_dir`]
/// (откуда их отдаёт HTTP-сервер) и вернуть структуру пака фронтенду.
#[tauri::command]
fn open_pack(path: String) -> Result<sigame_core::Pack, String> {
    let archive = PackArchive::load(&path).map_err(|e| e.to_string())?;
    archive.validate_media().map_err(|e| e.to_string())?;
    extract_media(&archive.media)?;
    Ok(archive.pack)
}

/// Распаковывает медиа в [`media_dir`], откуда их отдаёт HTTP-сервер.
fn extract_media(media: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let dir = media_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, bytes) in media {
        if name.contains("..") {
            return Err(format!("недопустимое имя медиафайла: {name}"));
        }
        let out = dir.join(name);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ----------------------------- Редактор паков -----------------------------

/// Состояние редактора: редактируемый пак вместе с байтами медиа.
struct EditorState(Mutex<PackArchive>);

#[derive(Serialize)]
struct AddedMedia {
    filename: String,
}

/// Начать новый пустой пак.
#[tauri::command]
fn editor_new(name: String, author: String, state: State<EditorState>) -> Pack {
    let pack = Pack {
        name,
        author,
        format_version: sigame_core::PACK_FORMAT_VERSION,
        rounds: Vec::new(),
    };
    *state.0.lock().unwrap() = PackArchive::new(pack.clone());
    pack
}

/// Открыть существующий `.sgpack` для редактирования.
#[tauri::command]
fn editor_load(path: String, state: State<EditorState>) -> Result<Pack, String> {
    let archive = PackArchive::load(&path).map_err(|e| e.to_string())?;
    extract_media(&archive.media)?; // для превью
    let pack = archive.pack.clone();
    *state.0.lock().unwrap() = archive;
    Ok(pack)
}

/// Добавить медиафайл с диска в редактируемый пак. Возвращает итоговое имя
/// внутри пака (с учётом возможного переименования при совпадении).
#[tauri::command]
fn editor_add_media(src_path: String, state: State<EditorState>) -> Result<AddedMedia, String> {
    let bytes = std::fs::read(&src_path).map_err(|e| e.to_string())?;
    let base = Path::new(&src_path)
        .file_name()
        .ok_or("у файла нет имени")?
        .to_string_lossy()
        .into_owned();
    if base.contains("..") || base.contains('/') {
        return Err(format!("недопустимое имя файла: {base}"));
    }

    let mut guard = state.0.lock().unwrap();
    let filename = unique_media_name(&guard.media, &base, &bytes);
    guard.media.insert(filename.clone(), bytes.clone());
    drop(guard);

    // Копия в папку HTTP-сервера для немедленного превью.
    let dir = media_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(&filename), &bytes).map_err(|e| e.to_string())?;

    Ok(AddedMedia { filename })
}

/// Сохранить текущий пак в `.sgpack`. Структуру (`pack`) присылает фронтенд,
/// байты медиа берём из состояния редактора.
#[tauri::command]
fn editor_save(path: String, pack: Pack, state: State<EditorState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    guard.pack = pack;
    // Убираем медиа, на которые больше нет ссылок.
    let refs = guard.pack.media_references();
    guard.media.retain(|name, _| refs.contains(name));
    guard.validate_media().map_err(|e| e.to_string())?;
    guard.save(&path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Подбирает уникальное имя медиа: то же имя — если файл новый или совпадает
/// побайтно; иначе добавляет суффикс `_1`, `_2`, …
fn unique_media_name(media: &BTreeMap<String, Vec<u8>>, base: &str, bytes: &[u8]) -> String {
    match media.get(base) {
        None => base.to_string(),
        Some(existing) if existing == bytes => base.to_string(),
        _ => {
            let (stem, ext) = match base.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (base.to_string(), String::new()),
            };
            let mut i = 1;
            loop {
                let candidate = format!("{stem}_{i}{ext}");
                if !media.contains_key(&candidate) {
                    return candidate;
                }
                i += 1;
            }
        }
    }
}

// ----------------------------- Сетевой клиент -----------------------------
//
// Rust-часть клиента работает «трубой» к серверу: держит TCP-сокет, читает из
// него строки JSON и пересылает их фронтенду как события Tauri, а команды от
// фронтенда пишет в сокет. Вся игровая логика интерфейса остаётся в JS.

/// Открытое сетевое подключение (поток для записи команд серверу).
struct NetState(Mutex<Option<TcpStream>>);

/// Подключиться к серверу, представиться (`hello`) и начать слушать сообщения.
#[tauri::command]
fn net_connect(
    app: AppHandle,
    state: State<NetState>,
    host: String,
    port: u16,
    name: String,
    is_host: bool,
) -> Result<(), String> {
    let stream = TcpStream::connect((host.as_str(), port)).map_err(|e| e.to_string())?;
    stream.set_nodelay(true).ok();
    let write = stream.try_clone().map_err(|e| e.to_string())?;
    *state.0.lock().unwrap() = Some(write);

    // Представляемся серверу.
    let hello = serde_json::json!({ "type": "hello", "name": name, "host": is_host });
    net_write(&state, &hello.to_string())?;

    // Поток чтения: каждую строку из сокета шлём фронтенду событием "net:message".
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            match line {
                Ok(l) if !l.trim().is_empty() => {
                    let _ = app.emit("net:message", l);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = app.emit("net:closed", ());
    });

    Ok(())
}

/// Отправить серверу одну строку JSON (фронтенд формирует её сам).
#[tauri::command]
fn net_send(state: State<NetState>, line: String) -> Result<(), String> {
    net_write(&state, &line)
}

/// Закрыть подключение.
#[tauri::command]
fn net_disconnect(state: State<NetState>) {
    if let Some(stream) = state.0.lock().unwrap().take() {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
}

fn net_write(state: &State<NetState>, line: &str) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let stream = guard.as_mut().ok_or("нет подключения к серверу")?;
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    stream.write_all(b"\n").map_err(|e| e.to_string())?;
    Ok(())
}

// ----------------------------- HTTP медиа-сервер -----------------------------

/// Запускает фоновый HTTP-сервер на случайном порту 127.0.0.1, отдающий файлы
/// из [`media_dir`]. Возвращает выбранный порт.
fn start_media_server() -> std::io::Result<u16> {
    let dir = media_dir();
    std::fs::create_dir_all(&dir)?;

    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let port = server
        .server_addr()
        .to_ip()
        .expect("медиа-сервер должен слушать IP-адрес")
        .port();

    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            handle_media_request(request, &dir);
        }
    });

    Ok(port)
}

fn handle_media_request(request: tiny_http::Request, dir: &Path) {
    let raw = request.url().trim_start_matches('/').to_string();
    let name = percent_decode(&raw);

    // Защита от выхода за пределы папки.
    if name.is_empty() || name.contains("..") {
        let _ = request.respond(tiny_http::Response::from_string("bad request").with_status_code(400));
        return;
    }

    let data = match std::fs::read(dir.join(&name)) {
        Ok(d) => d,
        Err(_) => {
            let _ = request.respond(tiny_http::Response::from_string("not found").with_status_code(404));
            return;
        }
    };
    let len = data.len() as u64;

    let ctype = content_type(&name);
    let ctype_h = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()).unwrap();
    let ranges_h = tiny_http::Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap();

    // Заголовок Range (для перемотки видео/аудио).
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
        let resp = tiny_http::Response::from_data(data)
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
        // Суффиксная форма: последние N байт.
        let n: u64 = end_s.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some((len.saturating_sub(n), len - 1))
    } else {
        let start: u64 = start_s.parse().ok()?;
        let end: u64 = if end_s.is_empty() {
            len - 1
        } else {
            end_s.parse().ok()?
        };
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

fn main() {
    let port = start_media_server().expect("не удалось запустить медиа-сервер");

    tauri::Builder::default()
        .manage(MediaServer { port })
        .manage(EditorState(Mutex::new(PackArchive::new(Pack::new("")))))
        .manage(NetState(Mutex::new(None)))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            demo_pack_path,
            media_base_url,
            open_pack,
            editor_new,
            editor_load,
            editor_add_media,
            editor_save,
            net_connect,
            net_send,
            net_disconnect
        ])
        .run(tauri::generate_context!())
        .expect("ошибка при запуске Tauri-приложения");
}
