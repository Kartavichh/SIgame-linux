//! Движок правил игры — чистая логика, без UI и сети.
//!
//! Это конечный автомат партии. Внешний код (сервер) подаёт команды
//! ([`Game::pick`], [`Game::buzz`], [`Game::judge`] и т.д.) и читает состояние.
//! Реальное время движок не отсчитывает: о тайм-аутах ему сообщает сервер
//! методами [`Game::reveal`] и [`Game::answer_timeout`].
//!
//! Финальный раунд здесь пока не реализован — он отдельным этапом.

use crate::pack::{Pack, QuestionKind};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Идентификатор игрока.
pub type PlayerId = u64;

/// Игрок и его счёт (счёт может уходить в минус — как в классике).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub score: i64,
}

/// Настройки партии. Длительности таймеров — для сервера; движок их не отсчитывает.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameConfig {
    /// Сколько секунд ждать нажатия кнопки после показа вопроса.
    pub buzz_time_secs: u32,
    /// Сколько секунд даётся на ответ после нажатия.
    pub answer_time_secs: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            buzz_time_secs: 5,
            answer_time_secs: 20,
        }
    }
}

/// Когда открывается приём нажатий на обычном вопросе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuzzMode {
    /// Ведущий открывает кнопки вручную командой [`Game::open_buzz`].
    Manual,
    /// Кнопки открываются автоматически, когда показан последний слайд вопроса.
    AfterLastSlide,
}

/// Настройки правил партии. Ведущий задаёт их в отдельном меню до старта игры
/// (по образцу настроек SIGame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSettings {
    /// «Кот в мешке»: `true` — выбравший обязан отдать вопрос другому игроку;
    /// `false` — может оставить себе.
    pub cat_must_give: bool,
    /// «Вопрос без риска»: `true` — награда удвоенная (`+2×номинал`);
    /// `false` — обычная (`+номинал`). Штрафа за ошибку нет в обоих случаях.
    pub no_risk_double: bool,
    /// Режим открытия кнопок на обычном вопросе.
    pub buzz_mode: BuzzMode,
    /// Фальстарт: `true` — нажатие до открытия кнопок блокирует игрока на
    /// `false_start_block_secs` секунд; `false` — до открытия кнопка неактивна.
    pub false_start: bool,
    /// Длительность блокировки за фальстарт (секунды; отсчитывает сервер).
    pub false_start_block_secs: u32,
    /// Сколько секунд ждать нажатия кнопки после открытия (для бегущей полосы).
    pub buzz_time_secs: u32,
    /// Сколько секунд даётся на ответ после нажатия.
    pub answer_time_secs: u32,
}

impl Default for GameSettings {
    fn default() -> Self {
        Self {
            cat_must_give: true,
            no_risk_double: false,
            buzz_mode: BuzzMode::Manual,
            false_start: false,
            false_start_block_secs: 3,
            buzz_time_secs: 5,
            answer_time_secs: 20,
        }
    }
}

/// Фаза партии.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Сбор игроков, игра ещё не началась.
    Lobby,
    /// Текущий выбирающий должен выбрать клетку табло.
    Picking,
    /// Вопрос показан, ждём нажатия кнопки.
    Question,
    /// Аукцион: игроки по очереди торгуются ставками.
    Auction,
    /// Кот в мешке: выбравший выбирает, кому передать вопрос.
    CatGive,
    /// Кто-то отвечает (нажал кнопку, выиграл аукцион, получил кота или вопрос
    /// без риска), ждём вердикта ведущего.
    Answering,
    /// Вопрос разобран: ведущий листает слайды ответа, затем закрывает вопрос.
    ShowAnswer,
    /// Все клетки раунда сыграны, ждём перехода к следующему раунду.
    RoundOver,
    /// Финал: игроки по очереди вычёркивают темы, пока не останется одна.
    FinalThemeRemoval,
    /// Финал: участники делают тайные ставки.
    FinalBets,
    /// Финал: участники пишут тайные ответы.
    FinalAnswers,
    /// Финал: ведущий вскрывает ответы по одному и судит.
    FinalReveal,
    /// Игра окончена.
    GameOver,
}

/// Состояние открытого вопроса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentQuestion {
    pub theme: usize,
    pub question: usize,
    pub price: u32,
    /// Тип вопроса (обычный/особый).
    pub kind: QuestionKind,
    /// Одиночный ответ: отвечает один игрок, гонки кнопок нет, при ошибке вопрос
    /// закрывается (а не открывается снова). Так играются аукцион, кот, без риска.
    pub solo: bool,
    /// Сколько начислить за верный ответ.
    pub reward: i64,
    /// Сколько списать за неверный (0 — без штрафа, как в «без риска»).
    pub penalty: i64,
    /// Кто сейчас отвечает (в фазе [`Phase::Answering`]).
    pub buzzed: Option<PlayerId>,
    /// Игроки, уже ошибшиеся на этом вопросе (повторно нажать не могут).
    pub locked_out: HashSet<PlayerId>,
    /// Игроки, временно заблокированные за фальстарт (нажали до открытия кнопок).
    /// Сервер снимает блок через `false_start_block_secs` ([`Game::clear_false_start`]).
    pub false_started: HashSet<PlayerId>,
    /// Открыт ли приём нажатий прямо сейчас.
    pub buzzing_open: bool,
    /// Индекс текущего слайда: в фазах `Question`/`Answering` — в `question_slides`,
    /// в фазе `ShowAnswer` — в `answer_slides`.
    pub slide: usize,
}

/// Состояние торгов на аукционе.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuctionState {
    pub theme: usize,
    pub question: usize,
    /// Номинал клетки (минимальная открывающая ставка).
    pub price: u32,
    /// Порядок хода: первым выбравший, затем остальные игроки по порядку.
    pub order: Vec<PlayerId>,
    /// Чей сейчас ход (индекс в [`AuctionState::order`]).
    pub turn: usize,
    /// Текущая наибольшая ставка.
    pub high_bid: i64,
    /// Кто сделал текущую наибольшую ставку.
    pub high_bidder: Option<PlayerId>,
    /// Игроки, вышедшие из торгов (спасовавшие или не способные перебить).
    pub passed: HashSet<PlayerId>,
}

impl AuctionState {
    /// Чей сейчас ход торговаться.
    pub fn current_bidder(&self) -> Option<PlayerId> {
        self.order.get(self.turn).copied()
    }
}

/// Ошибка применения команды к игре.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GameError {
    #[error("действие недоступно в текущей фазе игры")]
    WrongPhase,
    #[error("сейчас не ход этого игрока")]
    NotYourTurn,
    #[error("игрок не найден")]
    NoSuchPlayer,
    #[error("нет такой клетки на табло")]
    NoSuchCell,
    #[error("этот вопрос уже сыгран")]
    AlreadyPlayed,
    #[error("игрок уже ошибся на этом вопросе")]
    LockedOut,
    #[error("нельзя начать игру без игроков")]
    NoPlayers,
    #[error("в паке нет раундов")]
    NoRounds,
    #[error("игрок не участвует в финале")]
    NotParticipant,
    #[error("недопустимая ставка")]
    BadBet,
    #[error("нельзя передать вопрос этому игроку")]
    InvalidTransfer,
    #[error("фальстарт: нажатие до открытия кнопок")]
    FalseStart,
}

/// Состояние финального раунда.
#[derive(Debug, Clone)]
pub struct FinalState {
    /// Индекс финального раунда в паке.
    pub round: usize,
    /// Темы, ещё не вычеркнутые (индексы тем финального раунда).
    pub themes_remaining: Vec<usize>,
    /// Участники в порядке вычёркивания/вскрытия (по возрастанию счёта).
    pub order: Vec<PlayerId>,
    /// Сколько тем уже вычеркнуто (определяет, чей сейчас ход вычёркивать).
    pub removals_done: usize,
    /// Оставшаяся (выбранная) тема — когда вычёркивание завершено.
    pub chosen_theme: Option<usize>,
    /// Тайные ставки участников.
    pub bets: HashMap<PlayerId, i64>,
    /// Тайные ответы участников.
    pub answers: HashMap<PlayerId, String>,
    /// Сколько участников уже вскрыто (текущий = `order[reveal_index]`).
    pub reveal_index: usize,
    /// Вердикты по уже вскрытым участникам.
    pub verdicts: HashMap<PlayerId, bool>,
}

impl FinalState {
    /// Чей сейчас ход вычёркивать тему.
    pub fn current_remover(&self) -> Option<PlayerId> {
        if self.order.is_empty() {
            None
        } else {
            Some(self.order[self.removals_done % self.order.len()])
        }
    }
    /// Кого сейчас вскрывает ведущий (в фазе [`Phase::FinalReveal`]).
    pub fn current_reveal(&self) -> Option<PlayerId> {
        self.order.get(self.reveal_index).copied()
    }
}

/// Состояние партии и правила переходов.
pub struct Game {
    config: GameConfig,
    settings: GameSettings,
    pack: Pack,
    players: Vec<Player>,
    next_id: PlayerId,
    phase: Phase,
    round_index: usize,
    /// Сыгранные клетки текущего раунда: (тема, вопрос).
    used: HashSet<(usize, usize)>,
    picker: Option<PlayerId>,
    current: Option<CurrentQuestion>,
    auction: Option<AuctionState>,
    finale: Option<FinalState>,
    /// Кто станет выбирающим после показа слайдов ответа (фаза `ShowAnswer`).
    pending_picker: Option<PlayerId>,
}

impl Game {
    /// Новая партия в фазе [`Phase::Lobby`].
    pub fn new(pack: Pack, config: GameConfig) -> Self {
        // Таймеры из конфига переносим в настройки (их можно менять в меню).
        let settings = GameSettings {
            buzz_time_secs: config.buzz_time_secs,
            answer_time_secs: config.answer_time_secs,
            ..GameSettings::default()
        };
        Self {
            config,
            settings,
            pack,
            players: Vec::new(),
            next_id: 1,
            phase: Phase::Lobby,
            round_index: 0,
            used: HashSet::new(),
            picker: None,
            current: None,
            auction: None,
            finale: None,
            pending_picker: None,
        }
    }

    /// Изменить настройки правил (только в лобби, до старта игры).
    pub fn set_settings(&mut self, settings: GameSettings) -> Result<(), GameError> {
        if self.phase != Phase::Lobby {
            return Err(GameError::WrongPhase);
        }
        self.settings = settings;
        Ok(())
    }

    // ----------------------------- Команды -----------------------------

    /// Добавить игрока (только в лобби). Возвращает его id.
    pub fn add_player(&mut self, name: impl Into<String>) -> Result<PlayerId, GameError> {
        if self.phase != Phase::Lobby {
            return Err(GameError::WrongPhase);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.players.push(Player {
            id,
            name: name.into(),
            score: 0,
        });
        Ok(id)
    }

    /// Начать игру: переход из лобби к выбору первого вопроса.
    pub fn start_game(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::Lobby {
            return Err(GameError::WrongPhase);
        }
        if self.players.is_empty() {
            return Err(GameError::NoPlayers);
        }
        if self.pack.rounds.is_empty() {
            return Err(GameError::NoRounds);
        }
        self.round_index = 0;
        self.used.clear();
        self.picker = Some(self.players[0].id);
        if self.pack.rounds[0].is_final {
            self.start_final();
        } else {
            self.phase = Phase::Picking;
        }
        Ok(())
    }

    /// Выбрать клетку табло (делает текущий выбирающий).
    pub fn pick(&mut self, player: PlayerId, theme: usize, question: usize) -> Result<(), GameError> {
        if self.phase != Phase::Picking {
            return Err(GameError::WrongPhase);
        }
        if Some(player) != self.picker {
            return Err(GameError::NotYourTurn);
        }
        let round = self
            .pack
            .rounds
            .get(self.round_index)
            .ok_or(GameError::NoRounds)?;
        let q = round
            .themes
            .get(theme)
            .and_then(|t| t.questions.get(question))
            .ok_or(GameError::NoSuchCell)?;
        if self.used.contains(&(theme, question)) {
            return Err(GameError::AlreadyPlayed);
        }

        let price = q.price;
        let kind = q.kind;
        self.used.insert((theme, question));

        match kind {
            QuestionKind::Normal => self.start_normal(theme, question, price),
            QuestionKind::Auction => self.start_auction(theme, question, price, player),
            QuestionKind::CatInBag => self.start_cat(theme, question, price, player),
            QuestionKind::NoRisk => self.start_no_risk(theme, question, price, player),
        }
        Ok(())
    }

    /// Обычный вопрос: показываем первый слайд. Кнопки открываются по режиму
    /// `buzz_mode` (вручную ведущим либо автоматически на последнем слайде).
    fn start_normal(&mut self, theme: usize, question: usize, price: u32) {
        let slides_len = self
            .question_at(theme, question)
            .map(|q| q.question_slides.len())
            .unwrap_or(0);
        // В авторежиме кнопки открыты сразу, если слайд вопроса всего один (он же
        // последний); иначе откроются, когда ведущий долистает до последнего.
        let buzzing_open = match self.settings.buzz_mode {
            BuzzMode::Manual => false,
            BuzzMode::AfterLastSlide => slides_len <= 1,
        };
        self.current = Some(CurrentQuestion {
            theme,
            question,
            price,
            kind: QuestionKind::Normal,
            solo: false,
            reward: price as i64,
            penalty: price as i64,
            buzzed: None,
            locked_out: HashSet::new(),
            false_started: HashSet::new(),
            buzzing_open,
            slide: 0,
        });
        self.phase = Phase::Question;
    }

    /// Перевести вопрос в режим одиночного ответа конкретного игрока.
    fn start_solo(
        &mut self,
        theme: usize,
        question: usize,
        price: u32,
        kind: QuestionKind,
        answerer: PlayerId,
        reward: i64,
        penalty: i64,
    ) {
        self.current = Some(CurrentQuestion {
            theme,
            question,
            price,
            kind,
            solo: true,
            reward,
            penalty,
            buzzed: Some(answerer),
            locked_out: HashSet::new(),
            false_started: HashSet::new(),
            buzzing_open: false,
            slide: 0,
        });
        self.phase = Phase::Answering;
    }

    /// Вопрос без риска: отвечает выбравший, штрафа за ошибку нет.
    fn start_no_risk(&mut self, theme: usize, question: usize, price: u32, picker: PlayerId) {
        let mult = if self.settings.no_risk_double { 2 } else { 1 };
        self.start_solo(theme, question, price, QuestionKind::NoRisk, picker, price as i64 * mult, 0);
    }

    /// Кот в мешке: выбравший должен передать вопрос (либо может оставить себе —
    /// зависит от настройки). Если других игроков нет, играет сам.
    fn start_cat(&mut self, theme: usize, question: usize, price: u32, picker: PlayerId) {
        let has_others = self.players.iter().any(|p| p.id != picker);
        if self.settings.cat_must_give && !has_others {
            // Отдавать некому — выбравший играет сам.
            self.start_solo(theme, question, price, QuestionKind::CatInBag, picker, price as i64, price as i64);
            return;
        }
        self.current = Some(CurrentQuestion {
            theme,
            question,
            price,
            kind: QuestionKind::CatInBag,
            solo: true,
            reward: price as i64,
            penalty: price as i64,
            buzzed: None, // получателя выберут командой give
            locked_out: HashSet::new(),
            false_started: HashSet::new(),
            buzzing_open: false,
            slide: 0,
        });
        self.phase = Phase::CatGive;
    }

    /// Передать «кота» игроку (делает выбравший). Если настройка разрешает —
    /// может оставить себе (`target == picker`).
    pub fn give(&mut self, player: PlayerId, target: PlayerId) -> Result<(), GameError> {
        if self.phase != Phase::CatGive {
            return Err(GameError::WrongPhase);
        }
        if Some(player) != self.picker {
            return Err(GameError::NotYourTurn);
        }
        if !self.has_player(target) {
            return Err(GameError::NoSuchPlayer);
        }
        if self.settings.cat_must_give && target == player {
            return Err(GameError::InvalidTransfer);
        }
        let cur = self.current.as_mut().expect("в фазе CatGive есть вопрос");
        cur.buzzed = Some(target);
        self.phase = Phase::Answering;
        Ok(())
    }

    /// Аукцион: запускаем торги (выбравший ходит первым).
    fn start_auction(&mut self, theme: usize, question: usize, price: u32, picker: PlayerId) {
        let mut order = vec![picker];
        for p in &self.players {
            if p.id != picker {
                order.push(p.id);
            }
        }
        let picker_score = self.score(picker).unwrap_or(0);
        // Вырожденные случаи: один игрок или выбравший не может сделать ставку.
        if order.len() == 1 || picker_score < 1 {
            self.start_solo(theme, question, price, QuestionKind::Auction, picker, price as i64, price as i64);
            return;
        }
        self.auction = Some(AuctionState {
            theme,
            question,
            price,
            order,
            turn: 0,
            high_bid: 0,
            high_bidder: None,
            passed: HashSet::new(),
        });
        self.phase = Phase::Auction;
    }

    /// Сделать (повысить) ставку на аукционе.
    pub fn bid(&mut self, player: PlayerId, amount: i64) -> Result<(), GameError> {
        if self.phase != Phase::Auction {
            return Err(GameError::WrongPhase);
        }
        let score = self.score(player).ok_or(GameError::NoSuchPlayer)?;
        {
            let a = self.auction.as_ref().expect("в аукционе есть состояние");
            if Some(player) != a.current_bidder() {
                return Err(GameError::NotYourTurn);
            }
            let opening = a.high_bidder.is_none();
            if amount > score {
                return Err(GameError::BadBet);
            }
            if opening {
                // Открывающая ставка выбравшего — не ниже номинала.
                if amount < a.price as i64 {
                    return Err(GameError::BadBet);
                }
            } else if amount <= a.high_bid {
                return Err(GameError::BadBet);
            }
        }
        let a = self.auction.as_mut().unwrap();
        a.high_bid = amount;
        a.high_bidder = Some(player);
        self.auction_next();
        Ok(())
    }

    /// Ва-банк: поставить весь свой счёт.
    pub fn all_in(&mut self, player: PlayerId) -> Result<(), GameError> {
        if self.phase != Phase::Auction {
            return Err(GameError::WrongPhase);
        }
        let score = self.score(player).ok_or(GameError::NoSuchPlayer)?;
        {
            let a = self.auction.as_ref().expect("в аукционе есть состояние");
            if Some(player) != a.current_bidder() {
                return Err(GameError::NotYourTurn);
            }
            let opening = a.high_bidder.is_none();
            if score < 1 {
                return Err(GameError::BadBet);
            }
            // Не на открытии ва-банк должен перебивать текущую ставку.
            if !opening && score <= a.high_bid {
                return Err(GameError::BadBet);
            }
        }
        let a = self.auction.as_mut().unwrap();
        a.high_bid = score;
        a.high_bidder = Some(player);
        self.auction_next();
        Ok(())
    }

    /// Пас на аукционе (выход из торгов). Выбравший не может пасовать на открытии.
    pub fn pass(&mut self, player: PlayerId) -> Result<(), GameError> {
        if self.phase != Phase::Auction {
            return Err(GameError::WrongPhase);
        }
        {
            let a = self.auction.as_ref().expect("в аукционе есть состояние");
            if Some(player) != a.current_bidder() {
                return Err(GameError::NotYourTurn);
            }
            if a.high_bidder.is_none() {
                // Открытие: выбравший обязан назвать ставку (или ва-банк).
                return Err(GameError::BadBet);
            }
        }
        self.auction.as_mut().unwrap().passed.insert(player);
        self.auction_next();
        Ok(())
    }

    /// Передать ход следующему участнику торгов; авто-пас тех, кто не может
    /// перебить ставку; завершить аукцион, когда остался один участник.
    fn auction_next(&mut self) {
        loop {
            let (order_len, passed_len, high_bid) = {
                let a = self.auction.as_ref().unwrap();
                (a.order.len(), a.passed.len(), a.high_bid)
            };
            if order_len - passed_len <= 1 {
                self.resolve_auction();
                return;
            }
            let next_p = {
                let a = self.auction.as_mut().unwrap();
                a.turn = (a.turn + 1) % a.order.len();
                a.order[a.turn]
            };
            if self.auction.as_ref().unwrap().passed.contains(&next_p) {
                continue;
            }
            // Не может перебить текущую ставку — автоматически выходит.
            let score = self.score(next_p).unwrap_or(0);
            if score <= high_bid {
                self.auction.as_mut().unwrap().passed.insert(next_p);
                continue;
            }
            return; // ход next_p
        }
    }

    /// Завершить аукцион: победитель отвечает один за свою ставку.
    fn resolve_auction(&mut self) {
        let a = self.auction.take().expect("в аукционе есть состояние");
        match a.high_bidder {
            Some(winner) => {
                let bid = a.high_bid;
                self.start_solo(a.theme, a.question, a.price, QuestionKind::Auction, winner, bid, bid);
            }
            None => self.end_question(None),
        }
    }

    /// Нажать кнопку. Первый успешный вызов получает право ответа.
    ///
    /// Если кнопки ещё не открыты:
    /// - при включённом фальстарте игрок блокируется на этом вопросе до снятия
    ///   блока сервером ([`Game::clear_false_start`]) и возвращается
    ///   [`GameError::FalseStart`];
    /// - при выключенном — нажатие просто отклоняется ([`GameError::WrongPhase`]).
    pub fn buzz(&mut self, player: PlayerId) -> Result<(), GameError> {
        if self.phase != Phase::Question {
            return Err(GameError::WrongPhase);
        }
        if !self.has_player(player) {
            return Err(GameError::NoSuchPlayer);
        }
        let false_start = self.settings.false_start;
        let cur = self.current.as_mut().expect("в фазе Question есть вопрос");
        if cur.locked_out.contains(&player) || cur.false_started.contains(&player) {
            return Err(GameError::LockedOut);
        }
        if !cur.buzzing_open {
            if false_start {
                cur.false_started.insert(player);
                return Err(GameError::FalseStart);
            }
            return Err(GameError::WrongPhase);
        }
        cur.buzzed = Some(player);
        cur.buzzing_open = false;
        self.phase = Phase::Answering;
        Ok(())
    }

    /// Снять блокировку фальстарта с игрока (сервер вызывает по истечении
    /// `false_start_block_secs`). После этого игрок снова может нажать.
    pub fn clear_false_start(&mut self, player: PlayerId) {
        if let Some(cur) = self.current.as_mut() {
            cur.false_started.remove(&player);
        }
    }

    /// Ведущий вручную открывает приём нажатий (режим `Manual` или досрочно).
    pub fn open_buzz(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::Question {
            return Err(GameError::WrongPhase);
        }
        self.current
            .as_mut()
            .expect("в фазе Question есть вопрос")
            .buzzing_open = true;
        Ok(())
    }

    /// Ведущий листает слайды вперёд (вопроса — в `Question`/`Answering`,
    /// ответа — в `ShowAnswer`). На последнем слайде вопроса в режиме
    /// `AfterLastSlide` автоматически открывает кнопки.
    pub fn next_slide(&mut self) -> Result<(), GameError> {
        let slides_len = self.active_slides_len()?;
        let show_answer = self.phase == Phase::ShowAnswer;
        let auto = self.phase == Phase::Question
            && self.settings.buzz_mode == BuzzMode::AfterLastSlide;
        let cur = self.current.as_mut().expect("есть открытый вопрос");
        if slides_len > 0 && cur.slide + 1 < slides_len {
            cur.slide += 1;
        }
        if auto && !show_answer && cur.slide + 1 >= slides_len {
            cur.buzzing_open = true;
        }
        Ok(())
    }

    /// Ведущий листает слайды назад.
    pub fn prev_slide(&mut self) -> Result<(), GameError> {
        self.active_slides_len()?; // проверка фазы/наличия вопроса
        let cur = self.current.as_mut().expect("есть открытый вопрос");
        if cur.slide > 0 {
            cur.slide -= 1;
        }
        Ok(())
    }

    /// Закрыть вопрос после показа слайдов ответа (фаза `ShowAnswer`).
    pub fn close_question(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::ShowAnswer {
            return Err(GameError::WrongPhase);
        }
        let picker = self.pending_picker.take();
        self.finish_question(picker);
        Ok(())
    }

    /// Пропустить текущий вопрос без начисления очков (выбирающий не меняется).
    /// Доступно во время вопроса/торгов/передачи/ответа/показа ответа.
    pub fn skip_question(&mut self) -> Result<(), GameError> {
        match self.phase {
            Phase::Question
            | Phase::Answering
            | Phase::Auction
            | Phase::CatGive
            | Phase::ShowAnswer => {
                self.auction = None;
                self.pending_picker = None;
                self.finish_question(None);
                Ok(())
            }
            _ => Err(GameError::WrongPhase),
        }
    }

    /// Ручная правка счёта игрока ведущим (на случай ошибок судейства).
    pub fn set_score(&mut self, player: PlayerId, value: i64) -> Result<(), GameError> {
        let p = self
            .players
            .iter_mut()
            .find(|p| p.id == player)
            .ok_or(GameError::NoSuchPlayer)?;
        p.score = value;
        Ok(())
    }

    /// Длина активного списка слайдов и проверка, что есть открытый вопрос
    /// в подходящей фазе.
    fn active_slides_len(&self) -> Result<usize, GameError> {
        let answer = match self.phase {
            Phase::Question | Phase::Answering => false,
            Phase::ShowAnswer => true,
            _ => return Err(GameError::WrongPhase),
        };
        let cur = self.current.as_ref().ok_or(GameError::WrongPhase)?;
        let q = self
            .question_at(cur.theme, cur.question)
            .ok_or(GameError::WrongPhase)?;
        Ok(if answer {
            q.answer_slides.len()
        } else {
            q.question_slides.len()
        })
    }

    /// Вопрос текущего раунда по индексам клетки.
    fn question_at(&self, theme: usize, question: usize) -> Option<&crate::pack::Question> {
        self.pack
            .rounds
            .get(self.round_index)
            .and_then(|r| r.themes.get(theme))
            .and_then(|t| t.questions.get(question))
    }

    /// Вердикт ведущего по ответу нажавшего игрока.
    pub fn judge(&mut self, correct: bool) -> Result<(), GameError> {
        if self.phase != Phase::Answering {
            return Err(GameError::WrongPhase);
        }
        let (player, solo, reward, penalty) = {
            let cur = self.current.as_ref().expect("в фазе Answering есть вопрос");
            (
                cur.buzzed.expect("в фазе Answering есть отвечающий"),
                cur.solo,
                cur.reward,
                cur.penalty,
            )
        };

        // Одиночный ответ (аукцион/кот/без риска): вопрос закрывается в любом случае.
        if solo {
            if correct {
                self.add_score(player, reward);
                self.end_question(Some(player));
            } else {
                self.add_score(player, -penalty);
                self.end_question(None);
            }
            return Ok(());
        }

        if correct {
            self.add_score(player, reward);
            // Угадавший становится выбирающим.
            self.end_question(Some(player));
        } else {
            self.add_score(player, -penalty);
            let all_locked = {
                let cur = self.current.as_mut().unwrap();
                cur.locked_out.insert(player);
                cur.buzzed = None;
                cur.locked_out.len() >= self.players.len()
            };
            if all_locked {
                // Все ошиблись — показываем ответ, выбирающий не меняется.
                self.end_question(None);
            } else {
                // Снова открываем приём нажатий для остальных.
                self.current.as_mut().unwrap().buzzing_open = true;
                self.phase = Phase::Question;
            }
        }
        Ok(())
    }

    /// Никто не нажал (истёк таймер нажатия) — показать ответ и закрыть вопрос.
    pub fn reveal(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::Question {
            return Err(GameError::WrongPhase);
        }
        self.end_question(None);
        Ok(())
    }

    /// Истёк таймер ответа — считаем как неверный ответ.
    pub fn answer_timeout(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::Answering {
            return Err(GameError::WrongPhase);
        }
        self.judge(false)
    }

    /// Перейти к следующему раунду (после [`Phase::RoundOver`]).
    pub fn next_round(&mut self) -> Result<(), GameError> {
        if self.phase != Phase::RoundOver {
            return Err(GameError::WrongPhase);
        }
        self.round_index += 1;
        self.used.clear();
        if self.pack.rounds[self.round_index].is_final {
            self.start_final();
        } else {
            self.phase = Phase::Picking;
        }
        Ok(())
    }

    // --------------------------- Финальный раунд ---------------------------

    /// Вычеркнуть тему в финале (делает игрок, чей сейчас ход).
    pub fn remove_theme(&mut self, player: PlayerId, theme: usize) -> Result<(), GameError> {
        if self.phase != Phase::FinalThemeRemoval {
            return Err(GameError::WrongPhase);
        }
        let fs = self.finale.as_mut().expect("в финале есть состояние");
        if Some(player) != fs.current_remover() {
            return Err(GameError::NotYourTurn);
        }
        let pos = fs
            .themes_remaining
            .iter()
            .position(|&t| t == theme)
            .ok_or(GameError::NoSuchCell)?;
        fs.themes_remaining.remove(pos);
        fs.removals_done += 1;
        if fs.themes_remaining.len() == 1 {
            fs.chosen_theme = Some(fs.themes_remaining[0]);
            self.phase = Phase::FinalBets;
        }
        Ok(())
    }

    /// Сделать тайную ставку (1..=свой счёт). Можно менять, пока не сделали все.
    pub fn final_bet(&mut self, player: PlayerId, amount: i64) -> Result<(), GameError> {
        if self.phase != Phase::FinalBets {
            return Err(GameError::WrongPhase);
        }
        let score = self.score(player).ok_or(GameError::NoSuchPlayer)?;
        let fs = self.finale.as_mut().expect("в финале есть состояние");
        if !fs.order.contains(&player) {
            return Err(GameError::NotParticipant);
        }
        if amount < 1 || amount > score {
            return Err(GameError::BadBet);
        }
        fs.bets.insert(player, amount);
        if fs.bets.len() == fs.order.len() {
            self.phase = Phase::FinalAnswers;
        }
        Ok(())
    }

    /// Дать тайный ответ. Можно менять, пока не ответили все.
    pub fn final_answer(&mut self, player: PlayerId, text: impl Into<String>) -> Result<(), GameError> {
        if self.phase != Phase::FinalAnswers {
            return Err(GameError::WrongPhase);
        }
        let fs = self.finale.as_mut().expect("в финале есть состояние");
        if !fs.order.contains(&player) {
            return Err(GameError::NotParticipant);
        }
        fs.answers.insert(player, text.into());
        if fs.answers.len() == fs.order.len() {
            self.phase = Phase::FinalReveal;
        }
        Ok(())
    }

    /// Вердикт ведущего по текущему вскрываемому участнику.
    pub fn final_judge(&mut self, correct: bool) -> Result<(), GameError> {
        if self.phase != Phase::FinalReveal {
            return Err(GameError::WrongPhase);
        }
        let (player, bet) = {
            let fs = self.finale.as_ref().expect("в финале есть состояние");
            let player = fs.current_reveal().expect("есть кого вскрывать");
            (player, *fs.bets.get(&player).unwrap_or(&0))
        };
        self.add_score(player, if correct { bet } else { -bet });
        let done = {
            let fs = self.finale.as_mut().unwrap();
            fs.verdicts.insert(player, correct);
            fs.reveal_index += 1;
            fs.reveal_index >= fs.order.len()
        };
        if done {
            self.phase = Phase::GameOver;
        }
        Ok(())
    }

    /// Подготовить финальный раунд (вызывается при входе в финальный раунд).
    fn start_final(&mut self) {
        let round = self.round_index;
        let order = self.participants_in_order();
        if order.is_empty() {
            // Некому играть финал — сразу конец игры.
            self.phase = Phase::GameOver;
            return;
        }
        let theme_count = self.pack.rounds[round].themes.len();
        let mut fs = FinalState {
            round,
            themes_remaining: (0..theme_count).collect(),
            order,
            removals_done: 0,
            chosen_theme: None,
            bets: HashMap::new(),
            answers: HashMap::new(),
            reveal_index: 0,
            verdicts: HashMap::new(),
        };
        if fs.themes_remaining.len() <= 1 {
            // Вычёркивать нечего — сразу к ставкам.
            fs.chosen_theme = fs.themes_remaining.first().copied();
            self.phase = Phase::FinalBets;
        } else {
            self.phase = Phase::FinalThemeRemoval;
        }
        self.finale = Some(fs);
    }

    /// Участники финала (счёт > 0) по возрастанию счёта (ничьи — по id).
    fn participants_in_order(&self) -> Vec<PlayerId> {
        let mut parts: Vec<&Player> = self.players.iter().filter(|p| p.score > 0).collect();
        parts.sort_by(|a, b| a.score.cmp(&b.score).then(a.id.cmp(&b.id)));
        parts.into_iter().map(|p| p.id).collect()
    }

    // ----------------------------- Запросы -----------------------------

    pub fn config(&self) -> &GameConfig {
        &self.config
    }
    pub fn settings(&self) -> &GameSettings {
        &self.settings
    }
    pub fn auction(&self) -> Option<&AuctionState> {
        self.auction.as_ref()
    }
    pub fn pack(&self) -> &Pack {
        &self.pack
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn players(&self) -> &[Player] {
        &self.players
    }
    pub fn round_index(&self) -> usize {
        self.round_index
    }
    pub fn picker(&self) -> Option<PlayerId> {
        self.picker
    }
    pub fn current(&self) -> Option<&CurrentQuestion> {
        self.current.as_ref()
    }
    pub fn finale(&self) -> Option<&FinalState> {
        self.finale.as_ref()
    }
    pub fn score(&self, id: PlayerId) -> Option<i64> {
        self.players.iter().find(|p| p.id == id).map(|p| p.score)
    }
    /// Сыграны ли все клетки текущего раунда.
    pub fn is_round_complete(&self) -> bool {
        !self.pack.rounds.is_empty() && self.used.len() == self.round_cell_count()
    }
    /// Сыграна ли конкретная клетка `(тема, вопрос)` текущего раунда.
    pub fn is_used(&self, theme: usize, question: usize) -> bool {
        self.used.contains(&(theme, question))
    }

    // ----------------------------- Внутреннее -----------------------------

    fn has_player(&self, id: PlayerId) -> bool {
        self.players.iter().any(|p| p.id == id)
    }

    fn add_score(&mut self, id: PlayerId, delta: i64) {
        if let Some(p) = self.players.iter_mut().find(|p| p.id == id) {
            p.score += delta;
        }
    }

    fn round_cell_count(&self) -> usize {
        self.pack
            .rounds
            .get(self.round_index)
            .map(|r| r.themes.iter().map(|t| t.questions.len()).sum())
            .unwrap_or(0)
    }

    /// Завершить розыгрыш вопроса: если у вопроса есть слайды ответа — перейти
    /// к их показу (фаза `ShowAnswer`), иначе сразу закрыть вопрос.
    fn end_question(&mut self, new_picker: Option<PlayerId>) {
        let has_answer = self
            .current
            .as_ref()
            .and_then(|c| self.question_at(c.theme, c.question))
            .map(|q| !q.answer_slides.is_empty())
            .unwrap_or(false);
        if has_answer {
            self.pending_picker = new_picker;
            if let Some(c) = self.current.as_mut() {
                c.slide = 0;
                c.buzzed = None;
                c.buzzing_open = false;
            }
            self.phase = Phase::ShowAnswer;
        } else {
            self.finish_question(new_picker);
        }
    }

    /// Закрыть текущий вопрос и определить следующую фазу.
    fn finish_question(&mut self, new_picker: Option<PlayerId>) {
        self.current = None;
        if let Some(p) = new_picker {
            self.picker = Some(p);
        }
        if self.is_round_complete() {
            self.phase = if self.round_index + 1 < self.pack.rounds.len() {
                Phase::RoundOver
            } else {
                Phase::GameOver
            };
        } else {
            self.phase = Phase::Picking;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{Content, Question, QuestionKind, Round, Slide, Theme};

    /// Пак: 1 раунд, 1 тема, два вопроса (100 и 200).
    fn one_round_pack() -> Pack {
        Pack {
            name: "T".into(),
            author: String::new(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![Round {
                name: "Р1".into(),
                is_final: false,
                themes: vec![Theme {
                    name: "Тема".into(),
                    questions: vec![
                        // Пустой ответ -> поток без фазы показа ответа (так короче проверять очки/фазы).
                        Question::simple(100, QuestionKind::Normal, vec![Content::Text { value: "q1".into() }], ""),
                        Question::simple(200, QuestionKind::Normal, vec![Content::Text { value: "q2".into() }], ""),
                    ],
                }],
            }],
        }
    }

    fn started_game() -> (Game, PlayerId, PlayerId) {
        let mut g = Game::new(one_round_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let p2 = g.add_player("P2").unwrap();
        g.start_game().unwrap();
        (g, p1, p2)
    }

    #[test]
    fn full_flow_with_wrong_then_correct() {
        let (mut g, p1, p2) = started_game();
        assert_eq!(g.phase(), Phase::Picking);
        assert_eq!(g.picker(), Some(p1));

        // p1 выбирает первый вопрос
        g.pick(p1, 0, 0).unwrap();
        assert_eq!(g.phase(), Phase::Question);
        g.open_buzz().unwrap(); // режим Manual: ведущий открывает кнопки

        // p2 нажимает и ошибается
        g.buzz(p2).unwrap();
        assert_eq!(g.phase(), Phase::Answering);
        g.judge(false).unwrap();
        assert_eq!(g.score(p2), Some(-100));
        // не все ошиблись -> снова приём нажатий
        assert_eq!(g.phase(), Phase::Question);

        // p1 нажимает и отвечает верно
        g.buzz(p1).unwrap();
        g.judge(true).unwrap();
        assert_eq!(g.score(p1), Some(100));
        // угадавший стал выбирающим
        assert_eq!(g.picker(), Some(p1));
        assert_eq!(g.phase(), Phase::Picking);
    }

    #[test]
    fn game_over_after_all_cells() {
        let (mut g, p1, _p2) = started_game();
        g.pick(p1, 0, 0).unwrap();
        g.reveal().unwrap(); // никто не нажал
        assert_eq!(g.phase(), Phase::Picking);
        g.pick(p1, 0, 1).unwrap();
        g.reveal().unwrap();
        // обе клетки сыграны, раунд один -> конец игры
        assert_eq!(g.phase(), Phase::GameOver);
    }

    #[test]
    fn all_wrong_closes_question_without_changing_picker() {
        let (mut g, p1, p2) = started_game();
        g.pick(p1, 0, 0).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p1).unwrap();
        g.judge(false).unwrap();
        g.buzz(p2).unwrap();
        g.judge(false).unwrap();
        // оба ошиблись -> вопрос закрыт, выбирающий прежний
        assert_eq!(g.phase(), Phase::Picking);
        assert_eq!(g.picker(), Some(p1));
        assert_eq!(g.score(p1), Some(-100));
        assert_eq!(g.score(p2), Some(-100));
    }

    #[test]
    fn cannot_pick_when_not_your_turn() {
        let (mut g, _p1, p2) = started_game();
        assert_eq!(g.pick(p2, 0, 0), Err(GameError::NotYourTurn));
    }

    #[test]
    fn locked_out_player_cannot_buzz_again() {
        let (mut g, p1, _p2) = started_game();
        g.pick(p1, 0, 0).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p1).unwrap();
        g.judge(false).unwrap();
        assert_eq!(g.buzz(p1), Err(GameError::LockedOut));
    }

    #[test]
    fn cannot_add_player_after_start() {
        let (mut g, _p1, _p2) = started_game();
        assert_eq!(g.add_player("late"), Err(GameError::WrongPhase));
    }

    /// Пак: обычный раунд (1 тема, 2 вопроса) + финал (2 темы по 1 вопросу).
    fn pack_with_final() -> Pack {
        // Ответ пустой -> обычные вопросы закрываются без фазы показа ответа.
        let q = |price, _a: &str| {
            Question::simple(price, QuestionKind::Normal, vec![Content::Text { value: "q".into() }], "")
        };
        Pack {
            name: "T".into(),
            author: String::new(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![
                Round {
                    name: "Р1".into(),
                    is_final: false,
                    themes: vec![Theme {
                        name: "Тема".into(),
                        questions: vec![q(100, "a1"), q(200, "a2")],
                    }],
                },
                Round {
                    name: "Финал".into(),
                    is_final: true,
                    themes: vec![
                        Theme { name: "ТемаА".into(), questions: vec![q(0, "финА")] },
                        Theme { name: "ТемаБ".into(), questions: vec![q(0, "финБ")] },
                    ],
                },
            ],
        }
    }

    #[test]
    fn final_round_full_flow() {
        let mut g = Game::new(pack_with_final(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let p2 = g.add_player("P2").unwrap();
        g.start_game().unwrap();

        // Обычный раунд: p1 берёт 100, p2 берёт 200.
        g.pick(p1, 0, 0).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p1).unwrap();
        g.judge(true).unwrap();
        assert_eq!(g.picker(), Some(p1));
        g.pick(p1, 0, 1).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p2).unwrap();
        g.judge(true).unwrap();
        // Оба вопроса сыграны, есть следующий (финальный) раунд.
        assert_eq!(g.phase(), Phase::RoundOver);

        // Переход в финал.
        g.next_round().unwrap();
        assert_eq!(g.phase(), Phase::FinalThemeRemoval);
        // Порядок по возрастанию счёта: p1(100), затем p2(300). Ходит p1.
        assert_eq!(g.finale().unwrap().current_remover(), Some(p1));

        // p1 вычёркивает тему 1 -> остаётся тема 0 -> ставки.
        g.remove_theme(p1, 1).unwrap();
        assert_eq!(g.phase(), Phase::FinalBets);
        assert_eq!(g.finale().unwrap().chosen_theme, Some(0));

        // Нельзя поставить больше своего счёта.
        assert_eq!(g.final_bet(p1, 1000), Err(GameError::BadBet));
        g.final_bet(p1, 50).unwrap();
        g.final_bet(p2, 100).unwrap();
        assert_eq!(g.phase(), Phase::FinalAnswers);

        g.final_answer(p1, "ответ1").unwrap();
        g.final_answer(p2, "ответ2").unwrap();
        assert_eq!(g.phase(), Phase::FinalReveal);

        // Вскрытие по возрастанию: сначала p1 (верно +50), потом p2 (неверно -100).
        assert_eq!(g.finale().unwrap().current_reveal(), Some(p1));
        g.final_judge(true).unwrap();
        assert_eq!(g.finale().unwrap().current_reveal(), Some(p2));
        g.final_judge(false).unwrap();

        assert_eq!(g.phase(), Phase::GameOver);
        assert_eq!(g.score(p1), Some(150)); // 100 + 50
        assert_eq!(g.score(p2), Some(100)); // 200 - 100
    }

    #[test]
    fn final_skips_when_no_positive_players() {
        // Оба игрока уходят в минус в обычном раунде -> финал пропускается.
        let mut g = Game::new(pack_with_final(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let p2 = g.add_player("P2").unwrap();
        g.start_game().unwrap();
        g.pick(p1, 0, 0).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p1).unwrap();
        g.judge(false).unwrap(); // p1 -100
        g.buzz(p2).unwrap();
        g.judge(false).unwrap(); // p2 -100, оба заблокированы -> вопрос закрыт
        g.pick(p1, 0, 1).unwrap();
        g.reveal().unwrap(); // никто не нажал
        assert_eq!(g.phase(), Phase::RoundOver);
        g.next_round().unwrap();
        // Нет игроков со счётом > 0 -> сразу конец игры.
        assert_eq!(g.phase(), Phase::GameOver);
    }

    /// Пак с одной темой из 4 вопросов: обычный, аукцион, кот, без риска.
    fn special_pack() -> Pack {
        let q = |kind: QuestionKind, _a: &str| {
            Question::simple(100, kind, vec![Content::Text { value: "q".into() }], "")
        };
        Pack {
            name: "T".into(),
            author: String::new(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![Round {
                name: "Р1".into(),
                is_final: false,
                themes: vec![Theme {
                    name: "Тема".into(),
                    questions: vec![
                        q(QuestionKind::Normal, "n"),
                        q(QuestionKind::Auction, "auc"),
                        q(QuestionKind::CatInBag, "cat"),
                        q(QuestionKind::NoRisk, "nr"),
                    ],
                }],
            }],
        }
    }

    fn special_game() -> (Game, PlayerId, PlayerId, PlayerId) {
        let mut g = Game::new(special_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let p2 = g.add_player("P2").unwrap();
        let p3 = g.add_player("P3").unwrap();
        g.start_game().unwrap();
        (g, p1, p2, p3)
    }

    #[test]
    fn no_risk_no_penalty_on_wrong() {
        let (mut g, p1, _p2, _p3) = special_game();
        g.pick(p1, 0, 3).unwrap(); // без риска
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p1)); // отвечает выбравший
        assert!(g.current().unwrap().solo);
        g.judge(false).unwrap();
        assert_eq!(g.score(p1), Some(0)); // штрафа нет
        assert_eq!(g.picker(), Some(p1)); // ход остаётся
    }

    #[test]
    fn no_risk_double_reward() {
        let mut g = Game::new(special_pack(), GameConfig::default());
        let a = g.add_player("A").unwrap();
        let _b = g.add_player("B").unwrap();
        // Настройку задаём в лобби, до старта.
        g.set_settings(GameSettings { no_risk_double: true, ..GameSettings::default() }).unwrap();
        g.start_game().unwrap();
        g.pick(a, 0, 3).unwrap();
        g.judge(true).unwrap();
        assert_eq!(g.score(a), Some(200)); // удвоенный номинал
    }

    #[test]
    fn set_settings_rejected_after_start() {
        let (mut g, _p1, _p2, _p3) = special_game();
        assert_eq!(
            g.set_settings(GameSettings::default()),
            Err(GameError::WrongPhase)
        );
    }

    #[test]
    fn cat_must_give_to_other() {
        let (mut g, p1, p2, _p3) = special_game();
        g.pick(p1, 0, 2).unwrap(); // кот
        assert_eq!(g.phase(), Phase::CatGive);
        // себе оставить нельзя (настройка по умолчанию)
        assert_eq!(g.give(p1, p1), Err(GameError::InvalidTransfer));
        g.give(p1, p2).unwrap();
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p2));
        g.judge(true).unwrap();
        assert_eq!(g.score(p2), Some(100));
        assert_eq!(g.picker(), Some(p2)); // ответивший верно стал выбирающим
    }

    #[test]
    fn cat_wrong_penalizes_receiver_keeps_picker() {
        let (mut g, p1, p2, _p3) = special_game();
        g.pick(p1, 0, 2).unwrap();
        g.give(p1, p2).unwrap();
        g.judge(false).unwrap();
        assert_eq!(g.score(p2), Some(-100)); // штраф получателю
        assert_eq!(g.picker(), Some(p1)); // ход у выбравшего
    }

    #[test]
    fn cat_can_keep_when_allowed() {
        let mut g = Game::new(special_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let _p2 = g.add_player("P2").unwrap();
        g.set_settings(GameSettings { cat_must_give: false, ..GameSettings::default() }).unwrap();
        g.start_game().unwrap();
        g.pick(p1, 0, 2).unwrap();
        assert_eq!(g.phase(), Phase::CatGive);
        g.give(p1, p1).unwrap(); // оставить себе можно
        assert_eq!(g.current().unwrap().buzzed, Some(p1));
        g.judge(true).unwrap();
        assert_eq!(g.score(p1), Some(100));
    }

    #[test]
    fn auction_full_bidding() {
        let (mut g, p1, p2, p3) = special_game();
        // Дадим игрокам очки для торгов.
        g.add_score(p1, 500);
        g.add_score(p2, 300);
        g.add_score(p3, 200);
        g.pick(p1, 0, 1).unwrap(); // аукцион
        assert_eq!(g.phase(), Phase::Auction);
        assert_eq!(g.auction().unwrap().current_bidder(), Some(p1));
        // выбравший не может пасовать на открытии
        assert_eq!(g.pass(p1), Err(GameError::BadBet));
        // открытие ниже номинала запрещено
        assert_eq!(g.bid(p1, 50), Err(GameError::BadBet));
        g.bid(p1, 100).unwrap();
        assert_eq!(g.auction().unwrap().current_bidder(), Some(p2));
        g.bid(p2, 150).unwrap();
        assert_eq!(g.auction().unwrap().current_bidder(), Some(p3));
        g.all_in(p3).unwrap(); // ва-банк 200
        assert_eq!(g.auction().unwrap().current_bidder(), Some(p1));
        g.bid(p1, 250).unwrap();
        assert_eq!(g.auction().unwrap().current_bidder(), Some(p2));
        g.pass(p2).unwrap();
        // p3 (200) не может перебить 250 -> авто-пас -> остаётся p1 -> ответ
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p1));
        assert_eq!(g.current().unwrap().reward, 250);
        g.judge(true).unwrap();
        assert_eq!(g.score(p1), Some(750)); // 500 + 250
    }

    #[test]
    fn auction_all_pass_leaves_picker() {
        let (mut g, p1, p2, p3) = special_game();
        g.add_score(p1, 300);
        g.add_score(p2, 300);
        g.add_score(p3, 300);
        g.pick(p1, 0, 1).unwrap();
        g.bid(p1, 100).unwrap(); // открытие
        g.pass(p2).unwrap();
        g.pass(p3).unwrap();
        // все спасовали -> играет p1 за 100
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p1));
        assert_eq!(g.current().unwrap().reward, 100);
    }

    #[test]
    fn auction_degenerates_when_picker_broke() {
        let (mut g, p1, _p2, _p3) = special_game();
        // У выбравшего нет очков -> торговаться нечем -> играет один за номинал.
        g.pick(p1, 0, 1).unwrap();
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p1));
        assert_eq!(g.current().unwrap().reward, 100);
    }

    // ------------------------- Этап 10b: слайды и управление -------------------------

    /// Пак: 1 раунд, 1 тема. Вопрос 0 — 2 слайда вопроса + 2 слайда ответа;
    /// вопрос 1 — обычный без слайдов ответа (чтобы раунд не закрывался сразу).
    fn slides_pack() -> Pack {
        Pack {
            name: "S".into(),
            author: String::new(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![Round {
                name: "Р1".into(),
                is_final: false,
                themes: vec![Theme {
                    name: "Тема".into(),
                    questions: vec![
                        Question {
                            price: 100,
                            kind: QuestionKind::Normal,
                            question_slides: vec![Slide::text("слайд1"), Slide::text("слайд2")],
                            answer_slides: vec![Slide::text("ответ1"), Slide::text("ответ2")],
                        },
                        Question::simple(200, QuestionKind::Normal, vec![Content::Text { value: "q2".into() }], ""),
                    ],
                }],
            }],
        }
    }

    fn slides_game() -> (Game, PlayerId, PlayerId) {
        let mut g = Game::new(slides_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let p2 = g.add_player("P2").unwrap();
        g.start_game().unwrap();
        (g, p1, p2)
    }

    #[test]
    fn manual_mode_buzz_closed_until_open() {
        let (mut g, p1, _p2) = slides_game();
        g.pick(p1, 0, 0).unwrap();
        assert_eq!(g.phase(), Phase::Question);
        assert!(!g.current().unwrap().buzzing_open);
        // Фальстарт выключен -> раннее нажатие просто отклоняется (без блокировки).
        assert_eq!(g.buzz(p1), Err(GameError::WrongPhase));
        assert!(g.current().unwrap().false_started.is_empty());
        g.open_buzz().unwrap();
        assert!(g.current().unwrap().buzzing_open);
        g.buzz(p1).unwrap();
        assert_eq!(g.phase(), Phase::Answering);
    }

    #[test]
    fn after_last_slide_mode_opens_buzz_on_last_slide() {
        let mut g = Game::new(slides_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let _p2 = g.add_player("P2").unwrap();
        g.set_settings(GameSettings {
            buzz_mode: BuzzMode::AfterLastSlide,
            ..GameSettings::default()
        })
        .unwrap();
        g.start_game().unwrap();
        g.pick(p1, 0, 0).unwrap();
        // На первом из двух слайдов кнопки ещё закрыты.
        assert_eq!(g.current().unwrap().slide, 0);
        assert!(!g.current().unwrap().buzzing_open);
        g.next_slide().unwrap();
        // Дошли до последнего слайда -> кнопки открылись автоматически.
        assert_eq!(g.current().unwrap().slide, 1);
        assert!(g.current().unwrap().buzzing_open);
    }

    #[test]
    fn after_last_slide_single_slide_opens_immediately() {
        let mut g = Game::new(one_round_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let _p2 = g.add_player("P2").unwrap();
        g.set_settings(GameSettings {
            buzz_mode: BuzzMode::AfterLastSlide,
            ..GameSettings::default()
        })
        .unwrap();
        g.start_game().unwrap();
        g.pick(p1, 0, 0).unwrap(); // один слайд -> сразу открыто
        assert!(g.current().unwrap().buzzing_open);
    }

    #[test]
    fn false_start_blocks_then_clears() {
        let mut g = Game::new(slides_pack(), GameConfig::default());
        let p1 = g.add_player("P1").unwrap();
        let _p2 = g.add_player("P2").unwrap();
        g.set_settings(GameSettings {
            false_start: true,
            ..GameSettings::default()
        })
        .unwrap();
        g.start_game().unwrap();
        g.pick(p1, 0, 0).unwrap();
        // Нажатие до открытия кнопок -> фальстарт и блокировка.
        assert_eq!(g.buzz(p1), Err(GameError::FalseStart));
        assert!(g.current().unwrap().false_started.contains(&p1));
        // Пока блок держится — даже после открытия кнопок нажать нельзя.
        g.open_buzz().unwrap();
        assert_eq!(g.buzz(p1), Err(GameError::LockedOut));
        // Сервер снял блок -> снова можно нажать.
        g.clear_false_start(p1);
        g.buzz(p1).unwrap();
        assert_eq!(g.phase(), Phase::Answering);
        assert_eq!(g.current().unwrap().buzzed, Some(p1));
    }

    #[test]
    fn show_answer_flow_then_close() {
        let (mut g, p1, p2) = slides_game();
        g.pick(p1, 0, 0).unwrap();
        g.open_buzz().unwrap();
        g.buzz(p2).unwrap();
        g.judge(true).unwrap();
        // У вопроса есть слайды ответа -> фаза показа ответа.
        assert_eq!(g.phase(), Phase::ShowAnswer);
        assert_eq!(g.current().unwrap().slide, 0);
        g.next_slide().unwrap();
        assert_eq!(g.current().unwrap().slide, 1);
        g.next_slide().unwrap(); // дальше последнего не уходит
        assert_eq!(g.current().unwrap().slide, 1);
        g.prev_slide().unwrap();
        assert_eq!(g.current().unwrap().slide, 0);
        g.close_question().unwrap();
        // Закрыт -> выбирающим стал угадавший p2.
        assert_eq!(g.phase(), Phase::Picking);
        assert_eq!(g.picker(), Some(p2));
        assert_eq!(g.score(p2), Some(100));
    }

    #[test]
    fn reveal_shows_answer_when_present() {
        let (mut g, p1, _p2) = slides_game();
        g.pick(p1, 0, 0).unwrap();
        g.reveal().unwrap(); // никто не нажал, но есть слайды ответа
        assert_eq!(g.phase(), Phase::ShowAnswer);
        g.close_question().unwrap();
        assert_eq!(g.phase(), Phase::Picking);
        assert_eq!(g.picker(), Some(p1)); // выбирающий не изменился
    }

    #[test]
    fn no_answer_slides_closes_directly() {
        // Вопрос 1 без слайдов ответа -> после reveal сразу к выбору, без ShowAnswer.
        let (mut g, p1, _p2) = slides_game();
        g.pick(p1, 0, 1).unwrap();
        g.reveal().unwrap();
        assert_eq!(g.phase(), Phase::Picking);
    }

    #[test]
    fn skip_question_closes_without_scoring() {
        let (mut g, p1, _p2) = slides_game();
        g.pick(p1, 0, 0).unwrap();
        g.skip_question().unwrap();
        assert_eq!(g.phase(), Phase::Picking);
        assert_eq!(g.picker(), Some(p1)); // выбирающий не изменился
        assert_eq!(g.score(p1), Some(0));
    }

    #[test]
    fn set_score_corrects_player_score() {
        let (mut g, p1, _p2) = slides_game();
        g.set_score(p1, 777).unwrap();
        assert_eq!(g.score(p1), Some(777));
        assert_eq!(g.set_score(9999, 1), Err(GameError::NoSuchPlayer));
    }
}
