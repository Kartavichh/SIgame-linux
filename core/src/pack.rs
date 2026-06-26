//! Модель пака — структуры, соответствующие содержимому `pack.json`.
//!
//! Иерархия: [`Pack`] → [`Round`] → [`Theme`] → [`Question`] → [`Content`].

use crate::error::Result;
use crate::PACK_FORMAT_VERSION;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

fn default_format_version() -> u32 {
    PACK_FORMAT_VERSION
}

/// Пакет вопросов целиком (содержимое `pack.json`).
///
/// Сами медиафайлы здесь НЕ хранятся — только ссылки на их имена
/// (см. [`Content`]). За байты медиа отвечает [`crate::archive::PackArchive`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    /// Название пакета.
    pub name: String,
    /// Автор (необязательно).
    #[serde(default)]
    pub author: String,
    /// Версия формата (для будущей совместимости).
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    /// Раунды по порядку.
    #[serde(default)]
    pub rounds: Vec<Round>,
}

/// Раунд — один экран табло.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub name: String,
    /// Финальный раунд: вместо табло — вычёркивание тем, тайные ставки и ответы.
    /// По умолчанию `false` (обычный раунд) — для совместимости со старыми паками.
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub themes: Vec<Theme>,
}

/// Тема — столбец табло с набором вопросов.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)]
    pub questions: Vec<Question>,
}

/// Вопрос: стоимость, содержимое (текст/медиа) и ответ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Стоимость вопроса (очки).
    pub price: u32,
    /// Содержимое вопроса по порядку показа.
    #[serde(default)]
    pub content: Vec<Content>,
    /// Правильный ответ (для ведущего).
    #[serde(default)]
    pub answer: String,
}

/// Единица содержимого вопроса.
///
/// В JSON сериализуется с тегом `type`, например:
/// `{"type":"text","value":"Кто..."}` или `{"type":"image","value":"img1.jpg"}`.
/// Для медиа `value` — это имя файла внутри `media/` в архиве.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { value: String },
    Image { value: String },
    Video { value: String },
    Audio { value: String },
}

impl Content {
    /// Имя медиафайла, если это медиа (а не текст).
    pub fn media_file(&self) -> Option<&str> {
        match self {
            Content::Text { .. } => None,
            Content::Image { value } | Content::Video { value } | Content::Audio { value } => {
                Some(value)
            }
        }
    }
}

impl Pack {
    /// Новый пустой пак с заданным названием.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            format_version: PACK_FORMAT_VERSION,
            rounds: Vec::new(),
        }
    }

    /// Сериализовать в красиво отформатированный JSON (`pack.json`).
    pub fn to_json_string(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Разобрать `pack.json` из строки.
    pub fn from_json_str(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Множество имён медиафайлов, на которые ссылаются вопросы.
    pub fn media_references(&self) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for round in &self.rounds {
            for theme in &round.themes {
                for question in &theme.questions {
                    for content in &question.content {
                        if let Some(file) = content.media_file() {
                            set.insert(file.to_string());
                        }
                    }
                }
            }
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pack() -> Pack {
        Pack {
            name: "Пробный пакет".into(),
            author: "Автор".into(),
            format_version: PACK_FORMAT_VERSION,
            rounds: vec![Round {
                name: "Раунд 1".into(),
                is_final: false,
                themes: vec![Theme {
                    name: "История".into(),
                    questions: vec![Question {
                        price: 100,
                        content: vec![
                            Content::Text {
                                value: "Кто написал «Войну и мир»?".into(),
                            },
                            Content::Image {
                                value: "tolstoy.jpg".into(),
                            },
                            Content::Audio {
                                value: "hint.mp3".into(),
                            },
                        ],
                        answer: "Лев Толстой".into(),
                    }],
                }],
            }],
        }
    }

    #[test]
    fn json_roundtrip() {
        let pack = sample_pack();
        let json = pack.to_json_string().unwrap();
        let back = Pack::from_json_str(&json).unwrap();
        assert_eq!(pack, back);
    }

    #[test]
    fn content_serializes_with_type_tag() {
        let c = Content::Image {
            value: "x.jpg".into(),
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"type":"image","value":"x.jpg"}"#);
    }

    #[test]
    fn media_references_collects_only_media() {
        let refs = sample_pack().media_references();
        assert!(refs.contains("tolstoy.jpg"));
        assert!(refs.contains("hint.mp3"));
        assert_eq!(refs.len(), 2); // текст не считается
    }

    #[test]
    fn defaults_fill_missing_fields() {
        // Минимальный JSON без author/format_version/rounds.
        let pack = Pack::from_json_str(r#"{"name":"Мини"}"#).unwrap();
        assert_eq!(pack.name, "Мини");
        assert_eq!(pack.author, "");
        assert_eq!(pack.format_version, PACK_FORMAT_VERSION);
        assert!(pack.rounds.is_empty());
    }
}
