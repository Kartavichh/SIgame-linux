//! Чтение и запись файла `.sgpack`.
//!
//! `.sgpack` — это zip-архив со структурой:
//! ```text
//! pack.json        — описание пака (см. [`crate::pack::Pack`])
//! media/<файлы>    — медиа, на которые ссылаются вопросы
//! ```

use crate::error::{PackError, Result};
use crate::pack::Pack;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

const MANIFEST: &str = "pack.json";
const MEDIA_DIR: &str = "media/";

/// Пак вместе с байтами его медиафайлов — то, что лежит в одном `.sgpack`.
///
/// Медиа держим в памяти (`имя файла → байты`). Для Этапа 1 этого достаточно;
/// при работе с тяжёлым видео позже можно перейти на потоковое чтение.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackArchive {
    /// Содержимое `pack.json`.
    pub pack: Pack,
    /// Медиафайлы: имя (без префикса `media/`) → байты.
    pub media: BTreeMap<String, Vec<u8>>,
}

impl PackArchive {
    /// Архив с паком и без медиа.
    pub fn new(pack: Pack) -> Self {
        Self {
            pack,
            media: BTreeMap::new(),
        }
    }

    /// Загрузить `.sgpack` из файла.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;

        // 1. Обязательный pack.json.
        let manifest = {
            let mut entry = match archive.by_name(MANIFEST) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => return Err(PackError::MissingManifest),
                Err(other) => return Err(other.into()),
            };
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            s
        };
        let pack = Pack::from_json_str(&manifest)?;

        // 2. Все файлы из media/.
        let mut media = BTreeMap::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            if let Some(rel) = name.strip_prefix(MEDIA_DIR) {
                // Пропускаем саму запись-папку "media/".
                if rel.is_empty() || name.ends_with('/') {
                    continue;
                }
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                media.insert(rel.to_string(), buf);
            }
        }

        Ok(Self { pack, media })
    }

    /// Сохранить `.sgpack` в файл.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let file = File::create(path)?;
        let mut zip = zip::ZipWriter::new(file);

        // pack.json — сжимаем (текст хорошо жмётся).
        let json = self.pack.to_json_string()?;
        let json_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        zip.start_file(MANIFEST, json_opts)?;
        zip.write_all(json.as_bytes())?;

        // Медиа обычно уже сжаты (jpg/webm/mp3) — храним без перекомпрессии.
        let media_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in &self.media {
            zip.start_file(format!("{MEDIA_DIR}{name}"), media_opts)?;
            zip.write_all(bytes)?;
        }

        zip.finish()?;
        Ok(())
    }

    /// Проверить, что все медиа, на которые ссылаются вопросы, есть в архиве.
    pub fn validate_media(&self) -> Result<()> {
        for name in self.pack.media_references() {
            if !self.media.contains_key(&name) {
                return Err(PackError::MissingMedia(name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{Content, Question, Round, Theme};

    fn sample_archive() -> PackArchive {
        let pack = Pack {
            name: "Тест".into(),
            author: "Автор".into(),
            format_version: crate::PACK_FORMAT_VERSION,
            rounds: vec![Round {
                name: "Раунд 1".into(),
                is_final: false,
                themes: vec![Theme {
                    name: "История".into(),
                    questions: vec![
                        Question {
                            price: 100,
                            kind: crate::QuestionKind::Normal,
                            content: vec![Content::Text {
                                value: "Текстовый вопрос?".into(),
                            }],
                            answer: "Ответ".into(),
                        },
                        Question {
                            price: 200,
                            kind: crate::QuestionKind::Normal,
                            content: vec![Content::Image {
                                value: "img1.jpg".into(),
                            }],
                            answer: "Картинка".into(),
                        },
                    ],
                }],
            }],
        };
        let mut media = BTreeMap::new();
        media.insert("img1.jpg".to_string(), vec![1, 2, 3, 4, 5]);
        PackArchive { pack, media }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let original = sample_archive();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sgpack");

        original.save(&path).unwrap();
        let loaded = PackArchive::load(&path).unwrap();

        assert_eq!(original, loaded);
    }

    #[test]
    fn validate_ok_when_media_present() {
        assert!(sample_archive().validate_media().is_ok());
    }

    #[test]
    fn validate_detects_missing_media() {
        let mut archive = sample_archive();
        archive.media.clear(); // убрали img1.jpg, ссылка осталась
        match archive.validate_media() {
            Err(PackError::MissingMedia(name)) => assert_eq!(name, "img1.jpg"),
            other => panic!("ожидали MissingMedia, получили {other:?}"),
        }
    }

    #[test]
    fn load_without_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.sgpack");

        // Делаем zip без pack.json.
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"no manifest here").unwrap();
        zip.finish().unwrap();

        match PackArchive::load(&path) {
            Err(PackError::MissingManifest) => {}
            other => panic!("ожидали MissingManifest, получили {other:?}"),
        }
    }
}
