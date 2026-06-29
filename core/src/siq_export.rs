//! Экспорт `.sgpack` → `.siq` (классический формат SIGame, version 4).
//!
//! Построен как инверсия импорта (`crate::siq`), чтобы круг
//! `.sgpack → .siq → импорт` не терял данные. Подробности и маппинг —
//! в `docs/siq-export.md`.

use crate::archive::PackArchive;
use crate::error::Result;
use crate::pack::{Content, Pack, QuestionKind};
use quick_xml::escape::escape;
use std::collections::BTreeMap;
use std::io::{Cursor, Write as _};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Экспортировать пак в файл `.siq`.
pub fn export_siq(archive: &PackArchive, path: impl AsRef<Path>) -> Result<()> {
    let bytes = export_siq_bytes(archive)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Собрать `.siq` (ZIP) в память.
pub fn export_siq_bytes(archive: &PackArchive) -> Result<Vec<u8>> {
    // Имя медиафайла -> папка SIGame ("Images"/"Audio"/"Video").
    let mut media_folders: BTreeMap<String, &'static str> = BTreeMap::new();
    let xml = build_content_xml(&archive.pack, &mut media_folders);

    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buf));

        let text_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file("content.xml", text_opts)?;
        zip.write_all(xml.as_bytes())?;

        // Медиа: кладём в Images/Audio/Video. Обычно уже сжаты — храним как есть.
        let media_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, folder) in &media_folders {
            if let Some(bytes) = archive.media.get(name) {
                zip.start_file(format!("{folder}/{name}"), media_opts)?;
                zip.write_all(bytes)?;
            }
            // Если файла нет в архиве — просто пропускаем (ссылка останется битой,
            // но структура пака сохранится; наши паки обычно полные).
        }

        zip.finish()?;
    }
    Ok(buf)
}

/// Сформировать content.xml; попутно собрать список медиа и их папок.
fn build_content_xml(pack: &Pack, media: &mut BTreeMap<String, &'static str>) -> String {
    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
    s.push_str(&format!(
        r#"<package name="{}" version="4" xmlns="http://vladimirkhil.com/ygpackage3.0.xsd">"#,
        attr(&pack.name)
    ));

    // Авторы: Pack.author может быть "А, Б" — разворачиваем в несколько <author>.
    let authors: Vec<&str> = pack
        .author
        .split(',')
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    if !authors.is_empty() {
        s.push_str("<info><authors>");
        for a in authors {
            s.push_str(&format!("<author>{}</author>", text(a)));
        }
        s.push_str("</authors></info>");
    }

    s.push_str("<rounds>");
    for round in &pack.rounds {
        if round.is_final {
            s.push_str(&format!(r#"<round name="{}" type="final">"#, attr(&round.name)));
        } else {
            s.push_str(&format!(r#"<round name="{}">"#, attr(&round.name)));
        }
        s.push_str("<themes>");
        for theme in &round.themes {
            s.push_str(&format!(r#"<theme name="{}">"#, attr(&theme.name)));
            s.push_str("<questions>");
            for q in &theme.questions {
                s.push_str(&format!(r#"<question price="{}">"#, q.price));
                s.push_str("<scenario>");

                // Слайды вопроса -> атомы (каждый блок слайда — один атом).
                for slide in &q.question_slides {
                    for item in &slide.items {
                        push_atom(&mut s, item, media);
                    }
                }
                // Граница вопрос/ответ.
                s.push_str(r#"<atom type="marker"></atom>"#);
                // Медиа-блоки ответа -> атомы после marker (текст идёт в <right>).
                for slide in &q.answer_slides {
                    for item in &slide.items {
                        if !matches!(item, Content::Text { .. }) {
                            push_atom(&mut s, item, media);
                        }
                    }
                }
                s.push_str("</scenario>");

                // Текст ответа.
                let ans = q.answer_text();
                if !ans.is_empty() {
                    s.push_str(&format!("<right><answer>{}</answer></right>", text(&ans)));
                }

                // Тип особого вопроса.
                if let Some(name) = kind_name(q.kind) {
                    s.push_str(&format!(r#"<type name="{name}"/>"#));
                }

                s.push_str("</question>");
            }
            s.push_str("</questions></theme>");
        }
        s.push_str("</themes></round>");
    }
    s.push_str("</rounds></package>");
    s
}

/// Дописать один атом и (для медиа) запомнить файл с его папкой.
fn push_atom(s: &mut String, item: &Content, media: &mut BTreeMap<String, &'static str>) {
    match item {
        Content::Text { value } => {
            s.push_str(&format!("<atom>{}</atom>", text(value)));
        }
        Content::Image { value } => {
            media.insert(value.clone(), "Images");
            s.push_str(&format!(r#"<atom type="image">@{}</atom>"#, text(value)));
        }
        Content::Audio { value } => {
            media.insert(value.clone(), "Audio");
            s.push_str(&format!(r#"<atom type="voice">@{}</atom>"#, text(value)));
        }
        Content::Video { value } => {
            media.insert(value.clone(), "Video");
            s.push_str(&format!(r#"<atom type="video">@{}</atom>"#, text(value)));
        }
    }
}

/// Имя типа для `<type name=…>`; `None` для обычного вопроса (тег не пишется).
fn kind_name(kind: QuestionKind) -> Option<&'static str> {
    match kind {
        QuestionKind::Normal => None,
        QuestionKind::Auction => Some("auction"),
        QuestionKind::CatInBag => Some("cat"),
        QuestionKind::NoRisk => Some("noRisk"),
    }
}

/// Экранировать текст элемента XML.
fn text(s: &str) -> String {
    escape(s).into_owned()
}

/// Экранировать значение атрибута XML.
fn attr(s: &str) -> String {
    escape(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{Content, Question, QuestionKind, Round, Slide, Theme};
    use crate::siq::import_siq;

    fn import_bytes(bytes: &[u8]) -> PackArchive {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), bytes).unwrap();
        import_siq(f.path()).unwrap().0
    }

    fn sample_archive() -> PackArchive {
        let pack = Pack {
            name: "Тест & <пак>".into(),
            author: "Иван, Пётр".into(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![
                Round {
                    name: "Разминка".into(),
                    is_final: false,
                    themes: vec![Theme {
                        name: "Тема".into(),
                        questions: vec![
                            Question {
                                price: 100,
                                kind: QuestionKind::Auction,
                                question_slides: vec![
                                    Slide::text("Что на картинке?"),
                                    Slide::new(vec![Content::Image { value: "pic.jpg".into() }]),
                                ],
                                answer_slides: vec![
                                    // Кавычки и & проверяют экранирование XML + раскрытие сущностей.
                                    Slide::text(r#"Дуэйн "Скала" Джонсон & Ко"#),
                                    Slide::new(vec![Content::Audio { value: "ans.mp3".into() }]),
                                ],
                            },
                            Question::simple(
                                200,
                                QuestionKind::Normal,
                                vec![Content::Text { value: "Просто текст?".into() }],
                                "Да",
                            ),
                        ],
                    }],
                },
                Round {
                    name: "Финал".into(),
                    is_final: true,
                    themes: vec![Theme {
                        name: "Итог".into(),
                        questions: vec![Question::simple(
                            0,
                            QuestionKind::Normal,
                            vec![Content::Text { value: "Финальный вопрос".into() }],
                            "Финальный ответ",
                        )],
                    }],
                },
            ],
        };
        let mut media = BTreeMap::new();
        media.insert("pic.jpg".to_string(), b"IMG".to_vec());
        media.insert("ans.mp3".to_string(), b"SND".to_vec());
        PackArchive { pack, media }
    }

    #[test]
    fn export_then_import_roundtrips() {
        let archive = sample_archive();
        let bytes = export_siq_bytes(&archive).unwrap();
        let back = import_bytes(&bytes);
        // Пак идентичен (экспорт — точная инверсия импорта на его образе).
        assert_eq!(archive.pack, back.pack);
        // Медиа на месте (имена сохранились).
        assert!(back.media.contains_key("pic.jpg"));
        assert!(back.media.contains_key("ans.mp3"));
    }

    #[test]
    fn export_is_valid_siq_xml() {
        let archive = sample_archive();
        let bytes = export_siq_bytes(&archive).unwrap();
        // content.xml читается импортом без ошибок и спецсимволы не сломали XML.
        let back = import_bytes(&bytes);
        assert_eq!(back.pack.name, "Тест & <пак>");
        assert_eq!(back.pack.author, "Иван, Пётр");
        assert!(back.pack.rounds[1].is_final);
    }
}
