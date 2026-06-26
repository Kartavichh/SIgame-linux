//! Типы ошибок работы с паками.

use thiserror::Error;

/// Ошибка при загрузке/сохранении/проверке пака.
#[derive(Debug, Error)]
pub enum PackError {
    #[error("ошибка ввода-вывода: {0}")]
    Io(#[from] std::io::Error),

    #[error("ошибка zip-архива: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("ошибка разбора pack.json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("в архиве нет обязательного файла pack.json")]
    MissingManifest,

    #[error("вопрос ссылается на отсутствующий в архиве медиафайл: {0}")]
    MissingMedia(String),
}

/// Удобный псевдоним результата для всего крейта.
pub type Result<T> = std::result::Result<T, PackError>;
