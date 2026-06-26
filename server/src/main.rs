//! Headless игровой сервер SIGame-RS.
//!
//! Запускается в терминале (без графики), держит авторитетное состояние партии
//! и синхронизирует его между подключёнными клиентами по TCP.
//!
//! Запуск:
//! ```text
//! sigame-server <путь-к-паку.sgpack> [--port 7777]
//! ```
//! Управление с консоли: `status`, `players`, `start`, `help`, `quit`.

mod hub;
mod media;
mod protocol;

use hub::{ConnId, Hub};
use protocol::{ClientMsg, ServerMsg};
use sigame_core::{Game, GameConfig, PackArchive};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

type SharedHub = Arc<Mutex<Hub>>;

fn main() {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            eprintln!("\nИспользование: sigame-server <пак.sgpack> [--port 7777] [--media-port 7778]");
            std::process::exit(2);
        }
    };

    // Загружаем пак (медиа понадобятся на Этапе 6 — пока берём только структуру).
    let archive = match PackArchive::load(&args.pack) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Не удалось открыть пак «{}»: {e}", args.pack);
            std::process::exit(1);
        }
    };
    let pack_name = archive.pack.name.clone();
    let media = archive.media;
    let game = Game::new(archive.pack, GameConfig::default());
    let hub: SharedHub = Arc::new(Mutex::new(Hub::new(game)));

    // HTTP-раздача медиа клиентам (по умолчанию порт игры + 1).
    let media_port = args.media_port.unwrap_or(args.port + 1);
    if let Err(e) = media::start(media, media_port) {
        eprintln!("Не удалось запустить раздачу медиа на порту {media_port}: {e}");
        std::process::exit(1);
    }

    let listener = match TcpListener::bind(("0.0.0.0", args.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Не удалось занять порт {}: {e}", args.port);
            std::process::exit(1);
        }
    };

    println!("SIGame-RS сервер запущен.");
    println!("  Пак:        «{pack_name}»");
    println!("  Порт игры:  {} (клиенты подключаются на этот адрес:порт)", args.port);
    println!("  Порт медиа: {media_port} (HTTP-раздача файлов вопросов)");
    println!("Команды консоли: status, players, start, help, quit\n");

    // Поток приёма консольных команд.
    spawn_console(hub.clone());

    // Основной цикл приёма подключений: по потоку на клиента.
    let conn_ids = AtomicU64::new(1);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let conn = conn_ids.fetch_add(1, Ordering::Relaxed);
                let hub = hub.clone();
                std::thread::spawn(move || handle_conn(conn, stream, hub, media_port));
            }
            Err(e) => eprintln!("Ошибка приёма подключения: {e}"),
        }
    }
}

/// Обслуживание одного подключения: читаем строки JSON, применяем команды.
fn handle_conn(conn: ConnId, stream: TcpStream, hub: SharedHub, media_port: u16) {
    stream.set_nodelay(true).ok();
    let write = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut joined = false;

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,            // соединение закрыто
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: ClientMsg = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(e) => {
                send_direct(&write, &ServerMsg::Error { message: format!("неверный JSON: {e}") });
                continue;
            }
        };

        // Первое сообщение обязано быть Hello.
        match (joined, msg) {
            (false, ClientMsg::Hello { name, host }) => {
                let w = match write.try_clone() {
                    Ok(w) => w,
                    Err(_) => break,
                };
                let mut h = hub.lock().unwrap();
                match h.join(conn, name, host, w) {
                    Ok((id, is_host)) => {
                        joined = true;
                        h.send_to(conn, &ServerMsg::Welcome { id, host: is_host, media_port });
                        let label = h.client_label(conn);
                        h.broadcast();
                        drop(h);
                        println!("[+] {label} подключился");
                    }
                    Err(e) => {
                        send_direct(&write, &ServerMsg::Error { message: e });
                        break;
                    }
                }
            }
            (false, _) => {
                send_direct(&write, &ServerMsg::Error {
                    message: "первое сообщение должно быть hello".into(),
                });
                break;
            }
            (true, msg) => {
                let mut h = hub.lock().unwrap();
                if let Err(e) = h.handle(conn, msg) {
                    h.send_to(conn, &ServerMsg::Error { message: e });
                }
                h.broadcast();
            }
        }
    }

    // Отключение.
    let mut h = hub.lock().unwrap();
    let label = h.client_label(conn);
    h.remove(conn);
    h.broadcast();
    drop(h);
    if joined {
        println!("[-] {label} отключился");
    }
}

/// Отправить сообщение напрямую в поток (до регистрации клиента в хабе).
fn send_direct(stream: &TcpStream, msg: &ServerMsg) {
    if let Ok(s) = serde_json::to_string(msg) {
        let mut w = stream;
        let _ = writeln!(w, "{s}");
    }
}

/// Поток чтения команд из stdin сервера.
fn spawn_console(hub: SharedHub) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            match line.trim() {
                "" => {}
                "help" => println!("Команды: status, players, start, help, quit"),
                "status" => {
                    let h = hub.lock().unwrap();
                    println!(
                        "Фаза: {:?}; игроков: {}; подключений: {}",
                        h.phase(),
                        h.player_count(),
                        h.online_count()
                    );
                }
                "players" => {
                    let h = hub.lock().unwrap();
                    let list = h.players_summary();
                    if list.is_empty() {
                        println!("(игроков нет)");
                    }
                    for (name, score, online) in list {
                        let mark = if online { "●" } else { "○" };
                        println!("  {mark} {name}: {score}");
                    }
                }
                "start" => {
                    let mut h = hub.lock().unwrap();
                    match h.force_start() {
                        Ok(()) => {
                            h.broadcast();
                            println!("Игра начата.");
                        }
                        Err(e) => println!("Не удалось начать: {e}"),
                    }
                }
                "quit" | "exit" => {
                    println!("Выключение сервера.");
                    std::process::exit(0);
                }
                other => println!("Неизвестная команда: {other} (help — список)"),
            }
        }
    });
}

/// Аргументы командной строки.
struct Args {
    pack: String,
    port: u16,
    /// Порт HTTP-раздачи медиа; по умолчанию `port + 1`.
    media_port: Option<u16>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut pack: Option<String> = None;
        let mut port: u16 = 7777;
        let mut media_port: Option<u16> = None;
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            match a.as_str() {
                "--port" | "-p" => {
                    let v = it.next().ok_or("после --port нужен номер порта")?;
                    port = v.parse().map_err(|_| format!("неверный порт: {v}"))?;
                }
                "--media-port" => {
                    let v = it.next().ok_or("после --media-port нужен номер порта")?;
                    media_port = Some(v.parse().map_err(|_| format!("неверный порт: {v}"))?);
                }
                "--help" | "-h" => return Err("Показать справку".into()),
                _ if a.starts_with('-') => return Err(format!("неизвестный флаг: {a}")),
                _ => {
                    if pack.is_some() {
                        return Err("путь к паку указан дважды".into());
                    }
                    pack = Some(a);
                }
            }
        }
        let pack = pack.ok_or("не указан путь к паку .sgpack")?;
        Ok(Self { pack, port, media_port })
    }
}
