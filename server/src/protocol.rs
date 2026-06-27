//! Сетевой протокол: сообщения и снимки состояния.
//!
//! Транспорт — TCP, одно сообщение = одна строка JSON (заканчивается `\n`).
//! Тип сообщения кодируется полем `"type"` (тегированные перечисления serde),
//! поэтому JSON остаётся человекочитаемым и его удобно отлаживать.

use serde::{Deserialize, Serialize};
use sigame_core::{Content, PlayerId};

/// Сообщение от клиента серверу.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Первое сообщение после подключения: имя, роль и (необязательно) аватарка
    /// — компактный data-URL картинки, уменьшенной клиентом.
    Hello {
        name: String,
        #[serde(default)]
        host: bool,
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Сменить свою аватарку (в лобби). `null` — убрать (вернуть заглушку).
    SetAvatar {
        #[serde(default)]
        avatar: Option<String>,
    },
    /// Изменить настройки партии в лобби (только ведущий).
    Settings {
        cat_must_give: bool,
        no_risk_double: bool,
        /// Режим открытия кнопок: `"manual"` или `"after_last_slide"`.
        buzz_mode: String,
        false_start: bool,
        false_start_block_secs: u32,
        buzz_time_secs: u32,
        answer_time_secs: u32,
    },
    /// Начать игру (только ведущий).
    Start,
    /// Выбрать клетку табло (текущий выбирающий игрок).
    Pick { theme: usize, question: usize },
    /// Нажать кнопку (игрок).
    Buzz,
    /// Открыть приём нажатий (только ведущий; режим `manual` или досрочно).
    OpenBuzz,
    /// Листать слайды вопроса/ответа вперёд (только ведущий).
    NextSlide,
    /// Листать слайды назад (только ведущий).
    PrevSlide,
    /// Закрыть вопрос после показа слайдов ответа (только ведущий).
    CloseQuestion,
    /// Пропустить текущий вопрос без начисления очков (только ведущий).
    SkipQuestion,
    /// Ручная правка счёта игрока (только ведущий).
    SetScore { player: PlayerId, value: i64 },
    /// Аукцион: повысить ставку (игрок, чей ход).
    Bid { amount: i64 },
    /// Аукцион: ва-банк — весь свой счёт (игрок, чей ход).
    AllIn,
    /// Аукцион: пас (игрок, чей ход).
    Pass,
    /// Кот в мешке: передать вопрос игроку `target` (выбравший).
    Give { target: PlayerId },
    /// Вердикт по ответу (только ведущий).
    Judge { correct: bool },
    /// Никто не нажал — показать ответ и закрыть вопрос (только ведущий).
    Reveal,
    /// Перейти к следующему раунду (только ведущий).
    NextRound,
    /// Финал: вычеркнуть тему (игрок, чей ход).
    RemoveTheme { theme: usize },
    /// Финал: тайная ставка (участник).
    FinalBet { amount: i64 },
    /// Финал: тайный ответ (участник).
    FinalAnswer { text: String },
    /// Финал: вердикт по текущему вскрываемому участнику (только ведущий).
    FinalJudge { correct: bool },
}

/// Сообщение от сервера клиенту.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Подтверждение подключения: id клиента (если игрок), роль и порт, на
    /// котором сервер раздаёт медиа по HTTP.
    Welcome {
        id: Option<PlayerId>,
        host: bool,
        media_port: u16,
    },
    /// Полный снимок состояния игры (присылается при каждом изменении).
    State(Snapshot),
    /// Ошибка обработки команды.
    Error { message: String },
}

/// Полный снимок состояния партии, пригодный для отрисовки на клиенте.
///
/// Сервер шлёт снимок целиком при каждом изменении — это проще и надёжнее
/// инкрементальной синхронизации (нет рассинхрона). Данных немного.
#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub phase: String,
    pub players: Vec<PlayerView>,
    pub round_index: usize,
    pub round_count: usize,
    pub round_name: String,
    pub picker: Option<PlayerId>,
    pub board: Vec<ThemeView>,
    pub current: Option<CurrentView>,
    /// Состояние финала (заполняется только в финальных фазах).
    pub finale: Option<FinalView>,
    /// Состояние аукциона (только в фазе торгов).
    pub auction: Option<AuctionView>,
    /// Настройки правил партии (видны всем, меняет ведущий в лобби).
    pub settings: SettingsView,
    /// Ведущий партии (имя и аватарка), если он подключён.
    pub host: Option<HostView>,
}

/// Ведущий в снимке (он не игрок, поэтому отдельно).
#[derive(Debug, Serialize)]
pub struct HostView {
    pub name: String,
    pub avatar: Option<String>,
    pub online: bool,
}

/// Настройки правил партии в снимке.
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub cat_must_give: bool,
    pub no_risk_double: bool,
    /// Режим открытия кнопок: `"manual"` или `"after_last_slide"`.
    pub buzz_mode: String,
    pub false_start: bool,
    pub false_start_block_secs: u32,
    pub buzz_time_secs: u32,
    pub answer_time_secs: u32,
}

/// Снимок текущих торгов на аукционе.
#[derive(Debug, Serialize)]
pub struct AuctionView {
    pub price: u32,
    /// Чей сейчас ход торговаться.
    pub current_bidder: Option<PlayerId>,
    pub high_bid: i64,
    pub high_bidder: Option<PlayerId>,
    /// Игроки, вышедшие из торгов.
    pub passed: Vec<PlayerId>,
    /// Открытие: ставку ещё никто не делал (ходит выбравший, пас запрещён).
    pub opening: bool,
}

/// Снимок финального раунда (адаптирован под получателя).
#[derive(Debug, Serialize)]
pub struct FinalView {
    pub themes: Vec<FinalThemeView>,
    /// Чей ход вычёркивать (в фазе вычёркивания).
    pub remover: Option<PlayerId>,
    pub chosen_theme: Option<String>,
    /// Содержимое финального вопроса (когда тема выбрана).
    pub content: Vec<Content>,
    /// Правильный ответ — только в снимке для ведущего.
    pub answer: Option<String>,
    /// Участники финала по порядку (по возрастанию счёта).
    pub participants: Vec<PlayerId>,
    pub bets_in: usize,
    pub answers_in: usize,
    pub total: usize,
    /// Ставка самого получателя (если он участник и уже поставил).
    pub you_bet: Option<i64>,
    pub you_answered: bool,
    pub you_participant: bool,
    /// Кого ведущий вскрывает сейчас (вердикт ещё не вынесен).
    pub current_reveal: Option<RevealView>,
    /// Уже вскрытые участники с вердиктами.
    pub revealed: Vec<RevealView>,
}

#[derive(Debug, Serialize)]
pub struct FinalThemeView {
    pub index: usize,
    pub name: String,
    pub removed: bool,
}

#[derive(Debug, Serialize)]
pub struct RevealView {
    pub id: PlayerId,
    pub name: String,
    pub answer: String,
    pub bet: i64,
    /// `None` — вскрывается сейчас (вердикт не вынесен); `Some` — итог.
    pub verdict: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PlayerView {
    pub id: PlayerId,
    pub name: String,
    pub score: i64,
    pub online: bool,
    /// Аватарка игрока (data-URL) или `null` — тогда клиент рисует заглушку.
    pub avatar: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThemeView {
    pub name: String,
    pub cells: Vec<CellView>,
}

#[derive(Debug, Serialize)]
pub struct CellView {
    pub theme: usize,
    pub question: usize,
    pub price: u32,
    pub used: bool,
}

#[derive(Debug, Serialize)]
pub struct CurrentView {
    pub theme: usize,
    pub question: usize,
    pub price: u32,
    /// Тип вопроса: `normal` / `auction` / `cat_in_bag` / `no_risk`.
    pub kind: String,
    /// Одиночный ответ (особый вопрос): гонки кнопок нет.
    pub solo: bool,
    /// Сколько начислят за верный ответ (для аукциона — выигравшая ставка).
    pub reward: i64,
    /// Блоки текущего слайда (вопроса — в `question`/`answering`, ответа — в
    /// `show_answer`). Показываются одновременно.
    pub content: Vec<Content>,
    /// Индекс текущего слайда и общее число слайдов активного списка.
    pub slide: usize,
    pub slide_count: usize,
    /// Открыт ли приём нажатий прямо сейчас.
    pub buzzing_open: bool,
    pub buzzed: Option<PlayerId>,
    /// Игроки, уже ошибшиеся на этом вопросе (им нельзя жать кнопку снова).
    pub locked_out: Vec<PlayerId>,
    /// Игроки, временно заблокированные за фальстарт.
    pub false_started: Vec<PlayerId>,
    /// Правильный ответ — заполняется только в снимке для ведущего.
    pub answer: Option<String>,
}
