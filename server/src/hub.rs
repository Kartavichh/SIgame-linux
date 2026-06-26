//! «Хаб» — авторитетное состояние сервера: сама игра плюс список подключённых
//! клиентов. Вся логика выполняется под одним мьютексом (`Arc<Mutex<Hub>>`),
//! поэтому команды применяются строго по очереди — это и определяет, кто нажал
//! кнопку «первым».

use crate::protocol::*;
use sigame_core::{Game, Phase, PlayerId};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::TcpStream;

/// Идентификатор сетевого подключения (не путать с `PlayerId` в игре).
pub type ConnId = u64;

/// Одно подключение к серверу.
pub struct Client {
    /// id игрока в партии; `None` — это ведущий (он не играет).
    pub pid: Option<PlayerId>,
    pub name: String,
    pub host: bool,
    /// Поток для записи ответов этому клиенту (клон сокета).
    pub out: TcpStream,
}

pub struct Hub {
    game: Game,
    clients: HashMap<ConnId, Client>,
    /// Имя игрока → его `PlayerId`. Нужен для переподключения по имени.
    name_to_pid: HashMap<String, PlayerId>,
}

impl Hub {
    pub fn new(game: Game) -> Self {
        Self {
            game,
            clients: HashMap::new(),
            name_to_pid: HashMap::new(),
        }
    }

    // --------------------------- Подключение ---------------------------

    /// Зарегистрировать клиента после сообщения `Hello`.
    /// Возвращает его `PlayerId` (для игрока) и флаг ведущего.
    pub fn join(
        &mut self,
        conn: ConnId,
        name: String,
        want_host: bool,
        out: TcpStream,
    ) -> Result<(Option<PlayerId>, bool), String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("пустое имя".into());
        }

        if want_host {
            if self.clients.values().any(|c| c.host) {
                return Err("ведущий уже подключён".into());
            }
            self.clients.insert(conn, Client { pid: None, name, host: true, out });
            return Ok((None, true));
        }

        // Игрок: уже подключён кто-то с таким именем — отказ.
        if self.clients.values().any(|c| !c.host && c.name == name) {
            return Err("игрок с таким именем уже в игре".into());
        }
        // Известное имя → переподключение к существующему игроку; иначе новый.
        let pid = match self.name_to_pid.get(&name) {
            Some(&pid) => pid,
            None => {
                let pid = self.game.add_player(name.clone()).map_err(|e| e.to_string())?;
                self.name_to_pid.insert(name.clone(), pid);
                pid
            }
        };
        self.clients.insert(conn, Client { pid: Some(pid), name, host: false, out });
        Ok((Some(pid), false))
    }

    /// Убрать подключение (при обрыве связи). Игрок остаётся в партии — его счёт
    /// сохраняется, а имя можно переподключить.
    pub fn remove(&mut self, conn: ConnId) {
        self.clients.remove(&conn);
    }

    pub fn client_label(&self, conn: ConnId) -> String {
        match self.clients.get(&conn) {
            Some(c) if c.host => format!("ведущий «{}»", c.name),
            Some(c) => format!("игрок «{}»", c.name),
            None => "?".into(),
        }
    }

    pub fn player_count(&self) -> usize {
        self.game.players().len()
    }
    pub fn online_count(&self) -> usize {
        self.clients.len()
    }
    pub fn phase(&self) -> Phase {
        self.game.phase()
    }
    pub fn players_summary(&self) -> Vec<(String, i64, bool)> {
        let online: HashSet<PlayerId> = self.clients.values().filter_map(|c| c.pid).collect();
        self.game
            .players()
            .iter()
            .map(|p| (p.name.clone(), p.score, online.contains(&p.id)))
            .collect()
    }

    // --------------------------- Команды ---------------------------

    /// Применить команду клиента. Команды, требующие роли ведущего, проверяются.
    pub fn handle(&mut self, conn: ConnId, msg: ClientMsg) -> Result<(), String> {
        let (pid, is_host) = match self.clients.get(&conn) {
            Some(c) => (c.pid, c.host),
            None => return Err("неизвестное подключение".into()),
        };

        match msg {
            ClientMsg::Hello { .. } => Err("повторный hello".into()),
            ClientMsg::Start => {
                self.require_host(is_host)?;
                self.game.start_game().map_err(|e| e.to_string())
            }
            ClientMsg::Pick { theme, question } => {
                let pid = pid.ok_or("ведущий не выбирает клетку")?;
                self.game.pick(pid, theme, question).map_err(|e| e.to_string())
            }
            ClientMsg::Buzz => {
                let pid = pid.ok_or("ведущий не нажимает кнопку")?;
                self.game.buzz(pid).map_err(|e| e.to_string())
            }
            ClientMsg::Judge { correct } => {
                self.require_host(is_host)?;
                self.game.judge(correct).map_err(|e| e.to_string())
            }
            ClientMsg::Reveal => {
                self.require_host(is_host)?;
                self.game.reveal().map_err(|e| e.to_string())
            }
            ClientMsg::NextRound => {
                self.require_host(is_host)?;
                self.game.next_round().map_err(|e| e.to_string())
            }
            ClientMsg::RemoveTheme { theme } => {
                let pid = pid.ok_or("ведущий не вычёркивает темы")?;
                self.game.remove_theme(pid, theme).map_err(|e| e.to_string())
            }
            ClientMsg::FinalBet { amount } => {
                let pid = pid.ok_or("ведущий не делает ставок")?;
                self.game.final_bet(pid, amount).map_err(|e| e.to_string())
            }
            ClientMsg::FinalAnswer { text } => {
                let pid = pid.ok_or("ведущий не отвечает")?;
                self.game.final_answer(pid, text).map_err(|e| e.to_string())
            }
            ClientMsg::FinalJudge { correct } => {
                self.require_host(is_host)?;
                self.game.final_judge(correct).map_err(|e| e.to_string())
            }
        }
    }

    /// Принудительно начать игру по команде из консоли сервера.
    pub fn force_start(&mut self) -> Result<(), String> {
        self.game.start_game().map_err(|e| e.to_string())
    }

    fn require_host(&self, is_host: bool) -> Result<(), String> {
        if is_host {
            Ok(())
        } else {
            Err("команда доступна только ведущему".into())
        }
    }

    // --------------------------- Рассылка ---------------------------

    /// Отправить сообщение одному подключению.
    pub fn send_to(&mut self, conn: ConnId, msg: &ServerMsg) {
        if let Some(c) = self.clients.get_mut(&conn) {
            write_msg(&mut c.out, msg);
        }
    }

    /// Разослать снимок состояния всем клиентам. Снимок строится под каждого
    /// получателя: ведущий видит правильные ответы, а каждый игрок — свою
    /// ставку/ответ в финале.
    pub fn broadcast(&mut self) {
        // Сначала собираем строки (нужен &self), потом пишем (нужен &mut out).
        let mut lines: Vec<(ConnId, String)> = Vec::with_capacity(self.clients.len());
        for (&conn, c) in &self.clients {
            let snap = self.snapshot(c.pid, c.host);
            let line = serde_json::to_string(&ServerMsg::State(snap)).unwrap_or_default();
            lines.push((conn, line));
        }
        for (conn, line) in lines {
            if let Some(c) = self.clients.get_mut(&conn) {
                let _ = writeln!(c.out, "{line}");
            }
        }
    }

    /// Собрать снимок состояния для конкретного получателя.
    /// `for_host` включает правильные ответы; `viewer` — id игрока-получателя
    /// (нужен для его личной ставки/ответа в финале).
    fn snapshot(&self, viewer: Option<PlayerId>, for_host: bool) -> Snapshot {
        let online: HashSet<PlayerId> = self.clients.values().filter_map(|c| c.pid).collect();
        let game = &self.game;
        let ri = game.round_index();
        let rounds = &game.pack().rounds;

        let players = game
            .players()
            .iter()
            .map(|p| PlayerView {
                id: p.id,
                name: p.name.clone(),
                score: p.score,
                online: online.contains(&p.id),
            })
            .collect();

        let board = rounds
            .get(ri)
            .map(|r| {
                r.themes
                    .iter()
                    .enumerate()
                    .map(|(ti, t)| ThemeView {
                        name: t.name.clone(),
                        cells: t
                            .questions
                            .iter()
                            .enumerate()
                            .map(|(qi, q)| CellView {
                                theme: ti,
                                question: qi,
                                price: q.price,
                                used: game.is_used(ti, qi),
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let current = game.current().map(|cur| {
            let q = &rounds[ri].themes[cur.theme].questions[cur.question];
            CurrentView {
                theme: cur.theme,
                question: cur.question,
                price: cur.price,
                content: q.content.clone(),
                buzzed: cur.buzzed,
                locked_out: cur.locked_out.iter().copied().collect(),
                answer: if for_host { Some(q.answer.clone()) } else { None },
            }
        });

        let finale = game.finale().map(|fs| self.final_view(fs, viewer, for_host));

        Snapshot {
            phase: phase_name(game.phase()).to_string(),
            players,
            round_index: ri,
            round_count: rounds.len(),
            round_name: rounds.get(ri).map(|r| r.name.clone()).unwrap_or_default(),
            picker: game.picker(),
            board,
            current,
            finale,
        }
    }

    /// Построить снимок финала для получателя.
    fn final_view(
        &self,
        fs: &sigame_core::FinalState,
        viewer: Option<PlayerId>,
        for_host: bool,
    ) -> FinalView {
        let round = &self.game.pack().rounds[fs.round];
        let name_of = |id: PlayerId| {
            self.game
                .players()
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        };

        let themes = round
            .themes
            .iter()
            .enumerate()
            .map(|(i, t)| FinalThemeView {
                index: i,
                name: t.name.clone(),
                removed: !fs.themes_remaining.contains(&i),
            })
            .collect();

        // Выбранная тема и её вопрос.
        let chosen_q = fs
            .chosen_theme
            .and_then(|ti| round.themes.get(ti).and_then(|t| t.questions.first()));
        let chosen_theme = fs
            .chosen_theme
            .and_then(|ti| round.themes.get(ti))
            .map(|t| t.name.clone());
        let content = chosen_q.map(|q| q.content.clone()).unwrap_or_default();
        let answer = if for_host {
            chosen_q.map(|q| q.answer.clone())
        } else {
            None
        };

        // Вскрытые участники (и текущий — без вердикта).
        let reveal_view = |id: PlayerId, verdict: Option<bool>| RevealView {
            id,
            name: name_of(id),
            answer: fs.answers.get(&id).cloned().unwrap_or_default(),
            bet: *fs.bets.get(&id).unwrap_or(&0),
            verdict,
        };
        let revealed = fs.order[..fs.reveal_index]
            .iter()
            .map(|&id| reveal_view(id, fs.verdicts.get(&id).copied()))
            .collect();
        let current_reveal = fs.current_reveal().map(|id| reveal_view(id, None));

        let you_participant = viewer.map(|v| fs.order.contains(&v)).unwrap_or(false);

        FinalView {
            themes,
            remover: fs.current_remover(),
            chosen_theme,
            content,
            answer,
            participants: fs.order.clone(),
            bets_in: fs.bets.len(),
            answers_in: fs.answers.len(),
            total: fs.order.len(),
            you_bet: viewer.and_then(|v| fs.bets.get(&v).copied()),
            you_answered: viewer.map(|v| fs.answers.contains_key(&v)).unwrap_or(false),
            you_participant,
            current_reveal,
            revealed,
        }
    }
}

/// Записать сообщение в поток как строку JSON + перевод строки.
fn write_msg(stream: &mut TcpStream, msg: &ServerMsg) {
    if let Ok(s) = serde_json::to_string(msg) {
        let _ = writeln!(stream, "{s}");
    }
}

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Lobby => "lobby",
        Phase::Picking => "picking",
        Phase::Question => "question",
        Phase::Answering => "answering",
        Phase::RoundOver => "round_over",
        Phase::FinalThemeRemoval => "final_theme_removal",
        Phase::FinalBets => "final_bets",
        Phase::FinalAnswers => "final_answers",
        Phase::FinalReveal => "final_reveal",
        Phase::GameOver => "game_over",
    }
}
