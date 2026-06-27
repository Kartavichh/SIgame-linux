//! Ядро SIGame-RS.
//!
//! Модель паков и (позже) правила игры — чистая логика, не зависящая
//! ни от интерфейса, ни от сети.
//!
//! Модули:
//! - [`pack`] — структуры данных пака (что лежит в `pack.json`);
//! - [`archive`] — чтение/запись файла `.sgpack` (zip + `pack.json` + `media/`);
//! - [`error`] — типы ошибок.

pub mod archive;
pub mod error;
pub mod game;
pub mod pack;
pub mod siq;

pub use archive::PackArchive;
pub use error::{PackError, Result};
pub use siq::import_siq;
pub use game::{
    AuctionState, BuzzMode, CurrentQuestion, FinalState, Game, GameConfig, GameError, GameSettings,
    Phase, Player, PlayerId,
};
pub use pack::{Content, Pack, Question, QuestionKind, Round, Slide, Theme};

/// Версия формата паков `.sgpack`. Пишется в `pack.json` для будущей совместимости.
pub const PACK_FORMAT_VERSION: u32 = 1;
