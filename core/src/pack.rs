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

/// Тип вопроса. Обычный — гонка кнопок; остальные — особые механики.
///
/// В JSON: `"normal"`, `"auction"`, `"cat_in_bag"`, `"no_risk"`.
/// По умолчанию `Normal` — старые паки без поля читаются как обычные.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// Обычный вопрос: гонка кнопок, верно +номинал, неверно −номинал.
    #[default]
    Normal,
    /// Аукцион: игроки торгуются ставками, отвечает поставивший больше всех.
    Auction,
    /// Кот в мешке: выбравший передаёт вопрос игроку (правило — в настройках).
    CatInBag,
    /// Вопрос без риска: отвечает выбравший, при ошибке штрафа нет.
    NoRisk,
}

/// Слайд — единица показа. Все блоки слайда (`items`) показываются
/// одновременно; слайды листаются ведущим по порядку.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slide {
    /// Блоки слайда (текст/медиа) в порядке размещения.
    #[serde(default)]
    pub items: Vec<Content>,
}

impl Slide {
    /// Слайд из готового набора блоков.
    pub fn new(items: Vec<Content>) -> Self {
        Self { items }
    }

    /// Слайд из одного текстового блока.
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            items: vec![Content::Text {
                value: value.into(),
            }],
        }
    }
}

/// Вопрос: тип, стоимость и сценарий из слайдов (до и после момента ответа).
///
/// `question_slides` показываются до ответа игроков, `answer_slides` — после
/// (разбор/правильный ответ). Старые паки (поля `content`/`answer`) читаются
/// через миграцию (см. [`QuestionRepr`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "QuestionRepr")]
pub struct Question {
    /// Стоимость вопроса (очки).
    pub price: u32,
    /// Тип вопроса (обычный/особый). По умолчанию обычный.
    pub kind: QuestionKind,
    /// Слайды вопроса (до момента ответа).
    pub question_slides: Vec<Slide>,
    /// Слайды ответа (после момента ответа).
    pub answer_slides: Vec<Slide>,
}

/// Промежуточное представление для десериализации вопроса.
///
/// Принимает как новый формат (`question_slides`/`answer_slides`), так и старый
/// (`content`/`answer`). При отсутствии слайдов старые поля мигрируются:
/// `content` → один слайд вопроса, `answer` → один текстовый слайд ответа.
#[derive(Deserialize)]
struct QuestionRepr {
    price: u32,
    #[serde(default)]
    kind: QuestionKind,
    #[serde(default)]
    question_slides: Vec<Slide>,
    #[serde(default)]
    answer_slides: Vec<Slide>,
    // --- Устаревшие поля (паки Этапов ≤9) ---
    #[serde(default)]
    content: Vec<Content>,
    #[serde(default)]
    answer: String,
}

impl From<QuestionRepr> for Question {
    fn from(r: QuestionRepr) -> Self {
        let mut question_slides = r.question_slides;
        if question_slides.is_empty() && !r.content.is_empty() {
            question_slides = vec![Slide::new(r.content)];
        }
        let mut answer_slides = r.answer_slides;
        if answer_slides.is_empty() && !r.answer.is_empty() {
            answer_slides = vec![Slide::text(r.answer)];
        }
        Question {
            price: r.price,
            kind: r.kind,
            question_slides,
            answer_slides,
        }
    }
}

impl Question {
    /// Удобный конструктор: один слайд вопроса + один текстовый слайд ответа.
    /// Пустые `content`/`answer` дают пустые списки слайдов.
    pub fn simple(
        price: u32,
        kind: QuestionKind,
        content: Vec<Content>,
        answer: impl Into<String>,
    ) -> Self {
        let answer = answer.into();
        Question {
            price,
            kind,
            question_slides: if content.is_empty() {
                Vec::new()
            } else {
                vec![Slide::new(content)]
            },
            answer_slides: if answer.is_empty() {
                Vec::new()
            } else {
                vec![Slide::text(answer)]
            },
        }
    }

    /// Все блоки слайдов вопроса подряд (плоское представление).
    pub fn question_content(&self) -> Vec<Content> {
        self.question_slides
            .iter()
            .flat_map(|s| s.items.iter().cloned())
            .collect()
    }

    /// Текст ответа: текстовые блоки всех слайдов ответа через пробел.
    pub fn answer_text(&self) -> String {
        self.answer_slides
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(|c| match c {
                Content::Text { value } => Some(value.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Имена всех медиафайлов, на которые ссылаются слайды вопроса и ответа.
    pub fn media_files(&self) -> impl Iterator<Item = &str> {
        self.question_slides
            .iter()
            .chain(self.answer_slides.iter())
            .flat_map(|s| s.items.iter())
            .filter_map(Content::media_file)
    }
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
                    for file in question.media_files() {
                        set.insert(file.to_string());
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
                    questions: vec![Question::simple(
                        100,
                        QuestionKind::Normal,
                        vec![
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
                        "Лев Толстой",
                    )],
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
    fn legacy_content_answer_migrates_to_slides() {
        // Старый формат вопроса: поля content/answer без слайдов.
        let json = r#"{
            "name": "Старый пак",
            "rounds": [{
                "name": "Р1",
                "themes": [{
                    "name": "Т1",
                    "questions": [{
                        "price": 100,
                        "content": [
                            {"type":"text","value":"Вопрос?"},
                            {"type":"image","value":"pic.jpg"}
                        ],
                        "answer": "Ответ"
                    }]
                }]
            }]
        }"#;
        let pack = Pack::from_json_str(json).unwrap();
        let q = &pack.rounds[0].themes[0].questions[0];
        assert_eq!(q.kind, QuestionKind::Normal);
        // content → один слайд вопроса с двумя блоками
        assert_eq!(q.question_slides.len(), 1);
        assert_eq!(q.question_slides[0].items.len(), 2);
        // answer → один текстовый слайд ответа
        assert_eq!(q.answer_slides.len(), 1);
        assert_eq!(q.answer_text(), "Ответ");
        assert_eq!(q.media_files().collect::<Vec<_>>(), vec!["pic.jpg"]);
    }

    #[test]
    fn new_slide_format_roundtrips() {
        let q = Question {
            price: 300,
            kind: QuestionKind::Auction,
            question_slides: vec![
                Slide::new(vec![
                    Content::Text { value: "Слайд 1 текст".into() },
                    Content::Image { value: "a.jpg".into() },
                ]),
                Slide::text("Слайд 2"),
            ],
            answer_slides: vec![Slide::new(vec![
                Content::Text { value: "Это ответ".into() },
                Content::Video { value: "v.webm".into() },
            ])],
        };
        let json = serde_json::to_string(&q).unwrap();
        let back: Question = serde_json::from_str(&json).unwrap();
        assert_eq!(q, back);
    }

    #[test]
    fn slides_take_priority_over_legacy_fields() {
        // Если присутствуют и слайды, и старые поля — используются слайды.
        let json = r#"{
            "price": 100,
            "question_slides": [{"items":[{"type":"text","value":"новое"}]}],
            "content": [{"type":"text","value":"старое"}],
            "answer": "старый ответ"
        }"#;
        let q: Question = serde_json::from_str(json).unwrap();
        assert_eq!(q.question_slides.len(), 1);
        assert_eq!(
            q.question_slides[0].items,
            vec![Content::Text { value: "новое".into() }]
        );
        // answer_slides пуст в новом формате → мигрирует из legacy answer
        assert_eq!(q.answer_text(), "старый ответ");
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
