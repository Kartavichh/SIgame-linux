//! «Хаб» — авторитетное состояние сервера: сама игра плюс список подключённых
//! клиентов. Вся логика выполняется под одним мьютексом (`Arc<Mutex<Hub>>`),
//! поэтому команды применяются строго по очереди — это и определяет, кто нажал
//! кнопку «первым».

use crate::protocol::*;
use sigame_core::{BuzzMode, Game, GameError, GameSettings, Phase, PlayerId, QuestionKind};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
    /// Когда снять блок фальстарта с игрока (отсчитывает таймер-поток сервера).
    false_start_deadlines: HashMap<PlayerId, Instant>,
    /// Аватарки игроков (data-URL), сохраняются между переподключениями.
    player_avatars: HashMap<PlayerId, String>,
    /// Имя и аватарка ведущего (он не игрок, поэтому отдельно).
    host_name: Option<String>,
    host_avatar: Option<String>,
}

impl Hub {
    pub fn new(game: Game) -> Self {
        Self {
            game,
            clients: HashMap::new(),
            name_to_pid: HashMap::new(),
            false_start_deadlines: HashMap::new(),
            player_avatars: HashMap::new(),
            host_name: None,
            host_avatar: None,
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
        avatar: Option<String>,
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
            self.host_name = Some(name.clone());
            if avatar.is_some() {
                self.host_avatar = avatar;
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
        // Запоминаем аватарку (при переподключении сохраняем прежнюю, если новой нет).
        if let Some(av) = avatar {
            self.player_avatars.insert(pid, av);
        }
        self.clients.insert(conn, Client { pid: Some(pid), name, host: false, out });
        Ok((Some(pid), false))
    }

    /// Сменить аватарку клиента (по подключению). `None` — убрать.
    pub fn set_avatar(&mut self, conn: ConnId, avatar: Option<String>) {
        let Some(c) = self.clients.get(&conn) else { return };
        if c.host {
            self.host_avatar = avatar;
        } else if let Some(pid) = c.pid {
            match avatar {
                Some(av) => {
                    self.player_avatars.insert(pid, av);
                }
                None => {
                    self.player_avatars.remove(&pid);
                }
            }
        }
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
            ClientMsg::SetAvatar { avatar } => {
                self.set_avatar(conn, avatar);
                Ok(())
            }
            ClientMsg::Settings {
                cat_must_give,
                no_risk_double,
                buzz_mode,
                false_start,
                false_start_block_secs,
                buzz_time_secs,
                answer_time_secs,
            } => {
                self.require_host(is_host)?;
                let settings = GameSettings {
                    cat_must_give,
                    no_risk_double,
                    buzz_mode: parse_buzz_mode(&buzz_mode),
                    false_start,
                    false_start_block_secs,
                    buzz_time_secs,
                    answer_time_secs,
                };
                self.game.set_settings(settings).map_err(|e| e.to_string())
            }
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
                match self.game.buzz(pid) {
                    Ok(()) => Ok(()),
                    // Фальстарт: блокируем игрока и запоминаем, когда снять блок.
                    // Это не ошибка для клиента — блок виден в снимке состояния.
                    Err(GameError::FalseStart) => {
                        let secs = self.game.settings().false_start_block_secs;
                        self.false_start_deadlines
                            .insert(pid, Instant::now() + Duration::from_secs(secs as u64));
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            ClientMsg::OpenBuzz => {
                self.require_host(is_host)?;
                self.game.open_buzz().map_err(|e| e.to_string())
            }
            ClientMsg::NextSlide => {
                self.require_host(is_host)?;
                self.game.next_slide().map_err(|e| e.to_string())
            }
            ClientMsg::PrevSlide => {
                self.require_host(is_host)?;
                self.game.prev_slide().map_err(|e| e.to_string())
            }
            ClientMsg::CloseQuestion => {
                self.require_host(is_host)?;
                self.game.close_question().map_err(|e| e.to_string())
            }
            ClientMsg::SkipQuestion => {
                self.require_host(is_host)?;
                self.game.skip_question().map_err(|e| e.to_string())
            }
            ClientMsg::SetScore { player, value } => {
                self.require_host(is_host)?;
                self.game.set_score(player, value).map_err(|e| e.to_string())
            }
            ClientMsg::Bid { amount } => {
                let pid = pid.ok_or("ведущий не торгуется")?;
                self.game.bid(pid, amount).map_err(|e| e.to_string())
            }
            ClientMsg::AllIn => {
                let pid = pid.ok_or("ведущий не торгуется")?;
                self.game.all_in(pid).map_err(|e| e.to_string())
            }
            ClientMsg::Pass => {
                let pid = pid.ok_or("ведущий не торгуется")?;
                self.game.pass(pid).map_err(|e| e.to_string())
            }
            ClientMsg::Give { target } => {
                let pid = pid.ok_or("ведущий не передаёт кота")?;
                self.game.give(pid, target).map_err(|e| e.to_string())
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

    /// Снять истёкшие блоки фальстарта. Возвращает `true`, если что-то снято
    /// (тогда вызывающий рассылает новый снимок). Вызывает таймер-поток сервера.
    pub fn clear_expired_false_starts(&mut self) -> bool {
        if self.false_start_deadlines.is_empty() {
            return false;
        }
        let now = Instant::now();
        let expired: Vec<PlayerId> = self
            .false_start_deadlines
            .iter()
            .filter(|(_, &deadline)| deadline <= now)
            .map(|(&pid, _)| pid)
            .collect();
        for pid in &expired {
            self.game.clear_false_start(*pid);
            self.false_start_deadlines.remove(pid);
        }
        !expired.is_empty()
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
                avatar: self.player_avatars.get(&p.id).cloned(),
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

        let phase = game.phase();
        let current = game.current().map(|cur| {
            let q = &rounds[ri].themes[cur.theme].questions[cur.question];
            // В фазе показа ответа активны слайды ответа, иначе — вопроса.
            let slides = if phase == Phase::ShowAnswer {
                &q.answer_slides
            } else {
                &q.question_slides
            };
            let slide_count = slides.len();
            // Содержимое «кота» во время передачи видит только ведущий —
            // выбравший и остальные не должны знать вопрос заранее.
            let hide_content = phase == Phase::CatGive && !for_host;
            let content = if hide_content {
                Vec::new()
            } else {
                slides.get(cur.slide).map(|s| s.items.clone()).unwrap_or_default()
            };
            CurrentView {
                theme: cur.theme,
                question: cur.question,
                price: cur.price,
                kind: kind_name(cur.kind).to_string(),
                solo: cur.solo,
                reward: cur.reward,
                content,
                slide: cur.slide,
                slide_count,
                buzzing_open: cur.buzzing_open,
                buzzed: cur.buzzed,
                locked_out: cur.locked_out.iter().copied().collect(),
                false_started: cur.false_started.iter().copied().collect(),
                answer: if for_host { Some(q.answer_text()) } else { None },
            }
        });

        let auction = game.auction().map(|a| AuctionView {
            price: a.price,
            current_bidder: a.current_bidder(),
            high_bid: a.high_bid,
            high_bidder: a.high_bidder,
            passed: a.passed.iter().copied().collect(),
            opening: a.high_bidder.is_none(),
        });

        let finale = game.finale().map(|fs| self.final_view(fs, viewer, for_host));

        let s = game.settings();
        let settings = SettingsView {
            cat_must_give: s.cat_must_give,
            no_risk_double: s.no_risk_double,
            buzz_mode: buzz_mode_name(s.buzz_mode).to_string(),
            false_start: s.false_start,
            false_start_block_secs: s.false_start_block_secs,
            buzz_time_secs: s.buzz_time_secs,
            answer_time_secs: s.answer_time_secs,
        };

        let host = self.host_name.as_ref().map(|name| HostView {
            name: name.clone(),
            avatar: self.host_avatar.clone(),
            online: self.clients.values().any(|c| c.host),
        });

        Snapshot {
            phase: phase_name(phase).to_string(),
            players,
            round_index: ri,
            round_count: rounds.len(),
            round_name: rounds.get(ri).map(|r| r.name.clone()).unwrap_or_default(),
            picker: game.picker(),
            board,
            current,
            finale,
            auction,
            settings,
            host,
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
        let content = chosen_q.map(|q| q.question_content()).unwrap_or_default();
        let answer = if for_host {
            chosen_q.map(|q| q.answer_text())
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

fn kind_name(k: QuestionKind) -> &'static str {
    match k {
        QuestionKind::Normal => "normal",
        QuestionKind::Auction => "auction",
        QuestionKind::CatInBag => "cat_in_bag",
        QuestionKind::NoRisk => "no_risk",
    }
}

fn buzz_mode_name(m: BuzzMode) -> &'static str {
    match m {
        BuzzMode::Manual => "manual",
        BuzzMode::AfterLastSlide => "after_last_slide",
    }
}

fn parse_buzz_mode(s: &str) -> BuzzMode {
    match s {
        "after_last_slide" => BuzzMode::AfterLastSlide,
        _ => BuzzMode::Manual,
    }
}

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Lobby => "lobby",
        Phase::Picking => "picking",
        Phase::Question => "question",
        Phase::Auction => "auction",
        Phase::CatGive => "cat_give",
        Phase::Answering => "answering",
        Phase::ShowAnswer => "show_answer",
        Phase::RoundOver => "round_over",
        Phase::FinalThemeRemoval => "final_theme_removal",
        Phase::FinalBets => "final_bets",
        Phase::FinalAnswers => "final_answers",
        Phase::FinalReveal => "final_reveal",
        Phase::GameOver => "game_over",
    }
}
