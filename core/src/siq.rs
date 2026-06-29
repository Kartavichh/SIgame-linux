//! Импорт паков `.siq` (родной формат SIGame) в нашу модель `.sgpack`.
//!
//! `.siq` — это ZIP с `content.xml` (манифест) и папками `Images/Audio/Video`.
//! Поддерживаем классический формат (`<scenario>` из `<atom>`, version 3/4);
//! новый формат (`<params>`, version 5) определяем и отклоняем с пояснением.
//! Маппинг и решения описаны в `docs/siq-import.md`.

use crate::archive::PackArchive;
use crate::error::{PackError, Result};
use crate::pack::{Content, Pack, Question, QuestionKind, Round, Slide, Theme};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::Path;

/// Импортировать `.siq` и вернуть готовый архив пака вместе со списком
/// предупреждений (например, о ненайденных медиафайлах — импорт при этом не
/// прерывается).
pub fn import_siq(path: impl AsRef<Path>) -> Result<(PackArchive, Vec<String>)> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    // 1. Читаем content.xml (обязателен).
    let xml = read_content_xml(&mut zip)
        .ok_or_else(|| PackError::Siq("в архиве нет content.xml — это не пак .siq".into()))?;

    // 2. Читаем все медиа в память: ключ — путь в нижнем регистре с раскодированным
    //    именем (например, "images/моя картинка.jpg"), плюс отдельно по базовому имени.
    let (by_path, by_base) = read_media_index(&mut zip)?;

    // 3. Разбираем XML в маленькое дерево и строим пак.
    let root = parse_dom(&xml).map_err(PackError::Siq)?;
    let package = root
        .child("package")
        .ok_or_else(|| PackError::Siq("в content.xml нет корневого <package>".into()))?;

    let mut imp = Importer {
        by_path,
        by_base,
        out_media: BTreeMap::new(),
        interned: HashMap::new(),
        warnings: Vec::new(),
    };
    let pack = imp.build_pack(package)?;

    let archive = PackArchive {
        pack,
        media: imp.out_media,
    };
    Ok((archive, imp.warnings))
}

// ----------------------------- Чтение архива -----------------------------

/// Прочитать content.xml (имя сопоставляем без учёта регистра), убрать BOM.
fn read_content_xml<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
    // Сначала точное имя, потом поиск без учёта регистра.
    let idx = (0..zip.len()).find(|&i| {
        zip.by_index(i)
            .map(|e| e.name().eq_ignore_ascii_case("content.xml"))
            .unwrap_or(false)
    })?;
    let mut entry = zip.by_index(idx).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s.trim_start_matches('\u{feff}').to_string())
}

/// Построить индексы медиа: по полному пути и по базовому имени (оба в нижнем
/// регистре, имена раскодированы из percent-encoding).
fn read_media_index<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<(HashMap<String, Vec<u8>>, HashMap<String, Vec<u8>>)> {
    let mut by_path = HashMap::new();
    let mut by_base = HashMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let raw = entry.name().to_string();
        if raw.ends_with('/') {
            continue; // запись-папка
        }
        let lower = raw.to_ascii_lowercase();
        // Берём только медиапапки SIGame.
        if !(lower.starts_with("images/")
            || lower.starts_with("audio/")
            || lower.starts_with("video/"))
        {
            continue;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        let decoded = percent_decode(&raw).to_ascii_lowercase();
        let base = decoded.rsplit('/').next().unwrap_or(&decoded).to_string();
        by_base.insert(base, buf.clone());
        by_path.insert(decoded, buf);
    }
    Ok((by_path, by_base))
}

// ----------------------------- Построение пака -----------------------------

struct Importer {
    by_path: HashMap<String, Vec<u8>>,
    by_base: HashMap<String, Vec<u8>>,
    out_media: BTreeMap<String, Vec<u8>>,
    /// Уже скопированные медиа: раскодированная ссылка → имя в нашем `media/`.
    interned: HashMap<String, String>,
    warnings: Vec<String>,
}

impl Importer {
    fn build_pack(&mut self, package: &Node) -> Result<Pack> {
        let name = package.attr("name").unwrap_or("Импортированный пак").to_string();
        let author = package
            .child("info")
            .and_then(|i| i.child("authors"))
            .map(|a| {
                a.children_named("author")
                    .map(|n| n.text.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        let mut pack = Pack::new(name);
        pack.author = author;

        let rounds = package.child("rounds");
        if let Some(rounds) = rounds {
            for r in rounds.children_named("round") {
                pack.rounds.push(self.build_round(r)?);
            }
        }
        Ok(pack)
    }

    fn build_round(&mut self, round: &Node) -> Result<Round> {
        let name = round.attr("name").unwrap_or("Раунд").to_string();
        let is_final = round
            .attr("type")
            .map(|t| t.eq_ignore_ascii_case("final"))
            .unwrap_or(false);
        let mut themes = Vec::new();
        if let Some(ts) = round.child("themes") {
            for t in ts.children_named("theme") {
                themes.push(self.build_theme(t)?);
            }
        }
        Ok(Round { name, is_final, themes })
    }

    fn build_theme(&mut self, theme: &Node) -> Result<Theme> {
        let name = theme.attr("name").unwrap_or("Тема").to_string();
        let mut questions = Vec::new();
        if let Some(qs) = theme.child("questions") {
            for q in qs.children_named("question") {
                questions.push(self.build_question(q)?);
            }
        }
        Ok(Theme { name, questions })
    }

    fn build_question(&mut self, q: &Node) -> Result<Question> {
        // Классика (version 3/4): сценарий из <atom>. Новый формат (version 5):
        // контент в <params>. Иначе — пустой вопрос (не падаем).
        if let Some(scenario) = q.child("scenario") {
            return Ok(self.build_question_classic(q, scenario));
        }
        if let Some(params) = q.child("params") {
            return Ok(self.build_question_v5(q, params));
        }
        Ok(empty_question(q))
    }

    /// Классический формат: сценарий из `<atom>`, `marker` делит вопрос/ответ.
    fn build_question_classic(&mut self, q: &Node, scenario: &Node) -> Question {
        let price = q.attr("price").and_then(|p| p.trim().parse().ok()).unwrap_or(0);
        let kind = question_kind(q);

        let mut question_slides: Vec<Slide> = Vec::new();
        let mut answer_media_slides: Vec<Slide> = Vec::new();
        let mut in_answer = false;

        for atom in scenario.children_named("atom") {
            let atype = atom.attr("type").unwrap_or("text").to_ascii_lowercase();
            let value = atom.text.trim().to_string();

            if atype == "marker" {
                in_answer = true;
                continue;
            }

            let slide = match atype.as_str() {
                "image" | "voice" | "video" if value.starts_with('@') => {
                    match self.resolve(&atype, &value[1..]) {
                        Some(c) => Some(Slide::new(vec![c])),
                        None => None, // предупреждение уже добавлено
                    }
                }
                // Текст, «say», а также медиа без ссылки `@` — как текст.
                _ => {
                    if value.is_empty() {
                        None
                    } else {
                        Some(Slide::text(value))
                    }
                }
            };

            if let Some(slide) = slide {
                if in_answer {
                    answer_media_slides.push(slide);
                } else {
                    question_slides.push(slide);
                }
            }
        }

        // Текст ответа из <right><answer>… (несколько — через « / »).
        let answer_text = q
            .child("right")
            .map(|r| {
                r.children_named("answer")
                    .map(|n| n.text.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();

        let mut answer_slides = Vec::new();
        if !answer_text.is_empty() {
            answer_slides.push(Slide::text(answer_text));
        }
        answer_slides.extend(answer_media_slides);

        Question {
            price,
            kind,
            question_slides,
            answer_slides,
        }
    }

    /// Новый формат (version 5): контент в `<params>`. `param name="question"` —
    /// слайды вопроса, `param name="answer"` — медиа ответа, `answerOptions` —
    /// варианты (для вопросов с выбором). Тип ответа из `<right><answer>`.
    fn build_question_v5(&mut self, q: &Node, params: &Node) -> Question {
        let price = q.attr("price").and_then(|p| p.trim().parse().ok()).unwrap_or(0);
        let kind = v5_kind(q, params);

        let mut question_slides = Vec::new();
        if let Some(p) = find_param(params, "question") {
            question_slides.extend(self.content_param_slides(p));
        }
        // Варианты ответа (вопрос с выбором): добавляем подписанными слайдами.
        if let Some(opts) = find_param(params, "answerOptions") {
            for sub in opts.children_named("param") {
                let label = sub.attr("name").unwrap_or("?").to_string();
                for slide in self.content_param_slides(sub) {
                    question_slides.push(relabel(slide, &label));
                }
            }
        }

        // Текст ответа из <right><answer> (несколько — через « / »).
        let answer_text = q
            .child("right")
            .map(|r| {
                r.children_named("answer")
                    .map(|n| n.text.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default();

        let mut answer_slides = Vec::new();
        if !answer_text.is_empty() {
            answer_slides.push(Slide::text(answer_text));
        }
        if let Some(p) = find_param(params, "answer") {
            answer_slides.extend(self.content_param_slides(p));
        }

        Question {
            price,
            kind,
            question_slides,
            answer_slides,
        }
    }

    /// Слайды из `<param type="content">`: каждый `<item>` → отдельный слайд
    /// (текст или медиа). В v5 медиа задаётся `<item type="…">имя` (ссылка).
    fn content_param_slides(&mut self, param: &Node) -> Vec<Slide> {
        let mut slides = Vec::new();
        for item in param.children_named("item") {
            let itype = item.attr("type").unwrap_or("text").to_ascii_lowercase();
            let value = item.text.trim().to_string();
            if value.is_empty() {
                continue;
            }
            let slide = match itype.as_str() {
                "image" | "audio" | "voice" | "video" => {
                    self.resolve(&itype, &value).map(|c| Slide::new(vec![c]))
                }
                _ => Some(Slide::text(value)),
            };
            if let Some(s) = slide {
                slides.push(s);
            }
        }
        slides
    }

    /// Найти медиа по типу атома и ссылке, скопировать в наш `media/`, вернуть
    /// имя файла внутри нашего пака. `None` — файл не найден (накоплено
    /// предупреждение).
    fn resolve(&mut self, atom_type: &str, raw_ref: &str) -> Option<Content> {
        let folder = match atom_type {
            "image" => "images",
            "voice" | "audio" => "audio",
            "video" => "video",
            _ => return None,
        };
        let decoded = percent_decode(raw_ref);
        let stored = self.intern(folder, &decoded)?;
        Some(match atom_type {
            "image" => Content::Image { value: stored },
            "voice" | "audio" => Content::Audio { value: stored },
            "video" => Content::Video { value: stored },
            _ => unreachable!(),
        })
    }

    /// Скопировать байты медиа в `out_media` под безопасным уникальным именем.
    fn intern(&mut self, folder: &str, decoded_ref: &str) -> Option<String> {
        if let Some(name) = self.interned.get(decoded_ref) {
            return Some(name.clone());
        }
        let key = format!("{folder}/{}", decoded_ref.to_ascii_lowercase());
        let base = decoded_ref
            .rsplit('/')
            .next()
            .unwrap_or(decoded_ref)
            .to_ascii_lowercase();
        let bytes = match self.by_path.get(&key).or_else(|| self.by_base.get(&base)) {
            Some(b) => b.clone(),
            None => {
                self.warnings
                    .push(format!("медиафайл не найден в .siq: {decoded_ref}"));
                return None;
            }
        };
        let stored = self.unique_name(&base, &bytes);
        self.out_media.insert(stored.clone(), bytes);
        self.interned.insert(decoded_ref.to_string(), stored.clone());
        Some(stored)
    }

    /// Подобрать безопасное уникальное имя файла для нашего `media/`.
    fn unique_name(&self, base: &str, bytes: &[u8]) -> String {
        let safe = sanitize_name(base);
        match self.out_media.get(&safe) {
            None => safe,
            Some(existing) if existing == bytes => safe, // тот же файл — переиспользуем
            _ => {
                let (stem, ext) = match safe.rsplit_once('.') {
                    Some((s, e)) => (s.to_string(), format!(".{e}")),
                    None => (safe.clone(), String::new()),
                };
                let mut i = 1;
                loop {
                    let cand = format!("{stem}_{i}{ext}");
                    if !self.out_media.contains_key(&cand) {
                        return cand;
                    }
                    i += 1;
                }
            }
        }
    }
}

/// Пустой вопрос (нет сценария): сохраняем хотя бы цену/тип/ответ.
fn empty_question(q: &Node) -> Question {
    let price = q.attr("price").and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let answer = q
        .child("right")
        .and_then(|r| r.child("answer"))
        .map(|n| n.text.trim().to_string())
        .unwrap_or_default();
    Question {
        price,
        kind: question_kind(q),
        question_slides: Vec::new(),
        answer_slides: if answer.is_empty() {
            Vec::new()
        } else {
            vec![Slide::text(answer)]
        },
    }
}

/// Тип вопроса: из дочернего `<type name="…">` или из атрибута `type`.
fn question_kind(q: &Node) -> QuestionKind {
    let name = q
        .child("type")
        .and_then(|t| t.attr("name"))
        .or_else(|| q.attr("type"))
        .unwrap_or("")
        .to_ascii_lowercase();
    match name.as_str() {
        "auction" | "stake" => QuestionKind::Auction,
        "cat" | "bagcat" => QuestionKind::CatInBag,
        "sponsored" | "norisk" | "forall" => QuestionKind::NoRisk,
        _ => QuestionKind::Normal,
    }
}

/// Найти `<param name="…">` среди детей `<params>`.
fn find_param<'a>(params: &'a Node, name: &str) -> Option<&'a Node> {
    params
        .children_named("param")
        .find(|p| p.attr("name") == Some(name))
}

/// Тип вопроса v5: явный тип (если указан), иначе — секрет (есть param `theme`)
/// → «кот в мешке». Остальное — обычный.
fn v5_kind(q: &Node, params: &Node) -> QuestionKind {
    let explicit = question_kind(q);
    if explicit != QuestionKind::Normal {
        return explicit;
    }
    if find_param(params, "theme").is_some() {
        return QuestionKind::CatInBag; // секретный вопрос
    }
    QuestionKind::Normal
}

/// Подписать текстовый слайд варианта меткой («A) …»).
fn relabel(mut slide: Slide, label: &str) -> Slide {
    if let Some(Content::Text { value }) = slide.items.first_mut() {
        *value = format!("{label}) {value}");
    }
    slide
}

/// Привести имя к безопасному для `media/`: только базовое имя, без `..` и слэшей.
fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    let cleaned = cleaned.replace("..", "_");
    if cleaned.is_empty() {
        "media".to_string()
    } else {
        cleaned
    }
}

// ----------------------------- Мини-дерево XML -----------------------------

/// Узел простого дерева, построенного из событий quick-xml.
struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn root() -> Self {
        Node {
            tag: String::new(),
            attrs: Vec::new(),
            text: String::new(),
            children: Vec::new(),
        }
    }

    /// Первый дочерний узел с данным тегом.
    fn child(&self, tag: &str) -> Option<&Node> {
        self.children.iter().find(|n| n.tag == tag)
    }

    /// Все дочерние узлы с данным тегом.
    fn children_named<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |n| n.tag == tag)
    }

    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Разобрать XML в дерево [`Node`] (корень-обёртка, его дети — верхний уровень).
fn parse_dom(xml: &str) -> std::result::Result<Node, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = vec![Node::root()];

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("разбор XML: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => stack.push(node_from_start(&e)?),
            Ok(Event::Empty(e)) => {
                let n = node_from_start(&e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let n = stack.pop().unwrap();
                    stack.last_mut().unwrap().children.push(n);
                }
            }
            Ok(Event::Text(e)) => {
                // decode() переводит из кодировки документа, unescape() раскрывает
                // сущности (&amp; → &, &lt; → < и т. п.).
                let decoded = e.decode().map_err(|err| err.to_string())?;
                let t = quick_xml::escape::unescape(&decoded).map_err(|err| err.to_string())?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            Ok(Event::CData(e)) => {
                let t = String::from_utf8_lossy(&e.into_inner()).into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            // Ссылка на сущность (`&quot;`, `&#34;` и т. п.) — quick-xml отдаёт её
            // отдельным событием, а не внутри текста. Раскрываем сами, иначе
            // экранированные символы (кавычки, &, <) терялись бы.
            Ok(Event::GeneralRef(e)) => {
                if let Some(top) = stack.last_mut() {
                    if let Some(c) = e.resolve_char_ref().map_err(|err| err.to_string())? {
                        top.text.push(c);
                    } else {
                        let name = std::str::from_utf8(&e).unwrap_or("");
                        top.text.push_str(match name {
                            "lt" => "<",
                            "gt" => ">",
                            "amp" => "&",
                            "quot" => "\"",
                            "apos" => "'",
                            _ => "", // незнакомая сущность — пропускаем
                        });
                    }
                }
            }
            _ => {}
        }
    }

    Ok(stack.into_iter().next().unwrap())
}

fn node_from_start(e: &quick_xml::events::BytesStart) -> std::result::Result<Node, String> {
    let tag = local_name(e.name().as_ref());
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| err.to_string())?;
        let key = local_name(a.key.as_ref());
        let val = a.unescape_value().map_err(|err| err.to_string())?.into_owned();
        attrs.push((key, val));
    }
    Ok(Node {
        tag,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}

/// Локальное имя тега/атрибута (без префикса пространства имён).
fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.into_owned(),
    }
}

/// Простое percent-декодирование имён файлов (`%20`, кириллица и т. п.).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    /// Собрать `.siq` из content.xml и набора медиа (имя записи → байты).
    fn make_siq(content_xml: &str, media: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            zip.start_file("content.xml", opts).unwrap();
            zip.write_all(content_xml.as_bytes()).unwrap();
            for (name, bytes) in media {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    fn import_bytes(bytes: &[u8]) -> Result<(PackArchive, Vec<String>)> {
        let f = write_temp(bytes);
        import_siq(f.path())
    }

    const CLASSIC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package name="Мой пак" version="4" xmlns="http://vladimirkhil.com/ygpackage3.0.xsd">
  <info><authors><author>Иван</author><author>Пётр</author></authors></info>
  <rounds>
    <round name="Разминка" type="standart">
      <themes>
        <theme name="Картинки">
          <questions>
            <question price="100">
              <scenario>
                <atom>Что на картинке?</atom>
                <atom type="image">@моя картинка.jpg</atom>
                <atom type="marker"/>
                <atom type="voice">@answer.mp3</atom>
              </scenario>
              <right><answer>Кот</answer><answer>Кошка</answer></right>
              <wrong><answer>Собака</answer></wrong>
              <type name="auction"/>
            </question>
            <question price="200">
              <scenario><atom>Просто текст?</atom></scenario>
              <right><answer>Да</answer></right>
            </question>
          </questions>
        </theme>
      </themes>
    </round>
    <round name="Финал" type="final">
      <themes>
        <theme name="Итог">
          <questions>
            <question price="0">
              <scenario><atom>Финальный вопрос</atom></scenario>
              <right><answer>Финальный ответ</answer></right>
            </question>
          </questions>
        </theme>
      </themes>
    </round>
  </rounds>
</package>"#;

    #[test]
    fn imports_classic_pack() {
        // Имя картинки в архиве — percent-encoded (как делает SIGame).
        let media: &[(&str, &[u8])] = &[
            ("Images/%D0%BC%D0%BE%D1%8F%20%D0%BA%D0%B0%D1%80%D1%82%D0%B8%D0%BD%D0%BA%D0%B0.jpg", b"IMG"),
            ("Audio/answer.mp3", b"SND"),
        ];
        let (archive, warnings) = import_bytes(&make_siq(CLASSIC, media)).unwrap();
        let pack = &archive.pack;

        assert_eq!(pack.name, "Мой пак");
        assert_eq!(pack.author, "Иван, Пётр");
        assert_eq!(pack.rounds.len(), 2);
        assert!(warnings.is_empty(), "не ждали предупреждений: {warnings:?}");

        // Раунд 1, вопрос 100: текст + картинка (вопрос), аудио (ответ).
        let q = &pack.rounds[0].themes[0].questions[0];
        assert_eq!(q.price, 100);
        assert_eq!(q.kind, QuestionKind::Auction);
        // Слайды вопроса: текст + картинка.
        assert_eq!(q.question_slides.len(), 2);
        assert!(matches!(q.question_slides[0].items[0], Content::Text { .. }));
        assert!(matches!(q.question_slides[1].items[0], Content::Image { .. }));
        // Слайды ответа: текст ответа + аудио после marker.
        assert_eq!(q.answer_text(), "Кот / Кошка");
        assert_eq!(q.answer_slides.len(), 2);
        assert!(matches!(q.answer_slides[1].items[0], Content::Audio { .. }));

        // Медиа скопированы в наш архив и совпадают по байтам.
        let img = q.question_slides[1].items[0].media_file().unwrap();
        assert_eq!(archive.media.get(img).map(|b| b.as_slice()), Some(&b"IMG"[..]));

        // Финальный раунд распознан.
        assert!(pack.rounds[1].is_final);

        // Валидатор доволен (все ссылки на месте).
        archive.validate_media().unwrap();
    }

    #[test]
    fn missing_media_becomes_warning_not_error() {
        // Картинку в архив не кладём — должно быть предупреждение, не ошибка.
        let (archive, warnings) = import_bytes(&make_siq(CLASSIC, &[("Audio/answer.mp3", b"SND")])).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("картинка"), "warn: {warnings:?}");
        // Вопрос всё равно импортирован, просто без картинки.
        let q = &archive.pack.rounds[0].themes[0].questions[0];
        assert_eq!(q.question_slides.len(), 1); // только текст
    }

    const V5: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package name="Новый пак" version="5" xmlns="https://github.com/VladimirKhil/SI/blob/master/assets/siq_5.xsd">
  <rounds>
    <round name="Раунд">
      <themes>
        <theme name="Кино">
          <questions>
            <question price="100">
              <params><param name="question" type="content"><item type="image" isRef="True">кадр.jpg</item></param></params>
              <right><answer>Кухня</answer></right>
            </question>
            <question price="200">
              <params>
                <param name="question" type="content"><item type="text">Звук из фильма?</item><item type="audio" isRef="True">snd.mp3</item></param>
                <param name="answer" type="content"><item type="video" isRef="True">ans.mp4</item></param>
              </params>
              <right><answer>Ответ</answer></right>
            </question>
            <question price="300">
              <params>
                <param name="theme" type="content"><item type="text">Секрет</item></param>
                <param name="price" type="numberSet"><numberSet minimum="300" maximum="300" step="0" /></param>
                <param name="selectionMode">any</param>
                <param name="question" type="content"><item type="text">Секретный вопрос</item></param>
              </params>
              <right><answer>Секретный ответ</answer></right>
            </question>
          </questions>
        </theme>
      </themes>
    </round>
    <round name="ФИНАЛ" type="final">
      <themes><theme name="Итог"><questions>
        <question price="0"><params><param name="question" type="content"><item type="text">Финал?</item></param></params><right><answer>Да</answer></right></question>
      </questions></theme></themes>
    </round>
  </rounds>
</package>"#;

    #[test]
    fn imports_v5_pack() {
        let media: &[(&str, &[u8])] = &[
            ("Images/%D0%BA%D0%B0%D0%B4%D1%80.jpg", b"IMG"), // «кадр.jpg» percent-encoded
            ("Audio/snd.mp3", b"SND"),
            ("Video/ans.mp4", b"VID"),
        ];
        let (archive, warnings) = import_bytes(&make_siq(V5, media)).unwrap();
        let pack = &archive.pack;
        assert_eq!(pack.name, "Новый пак");
        assert_eq!(pack.rounds.len(), 2);
        assert!(warnings.is_empty(), "не ждали предупреждений: {warnings:?}");

        let qs = &pack.rounds[0].themes[0].questions;
        // Q100: одна картинка-вопрос, текст ответа.
        assert_eq!(qs[0].price, 100);
        assert!(matches!(qs[0].question_slides[0].items[0], Content::Image { .. }));
        assert_eq!(qs[0].answer_text(), "Кухня");
        // Q200: текст+аудио (вопрос), видео (ответ) из param answer.
        assert_eq!(qs[1].question_slides.len(), 2);
        assert!(matches!(qs[1].question_slides[1].items[0], Content::Audio { .. }));
        assert!(qs[1]
            .answer_slides
            .iter()
            .any(|s| matches!(s.items[0], Content::Video { .. })));
        // Q300: секрет (param theme) → кот в мешке.
        assert_eq!(qs[2].kind, QuestionKind::CatInBag);

        assert!(pack.rounds[1].is_final);
        archive.validate_media().unwrap();
    }

    #[test]
    fn not_a_siq_errors() {
        // ZIP без content.xml.
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zip.start_file("readme.txt", SimpleFileOptions::default()).unwrap();
            zip.write_all(b"hi").unwrap();
            zip.finish().unwrap();
        }
        match import_bytes(&buf) {
            Err(PackError::Siq(msg)) => assert!(msg.contains("content.xml")),
            other => panic!("ожидали ошибку Siq про content.xml, получили {other:?}"),
        }
    }

    #[test]
    fn xml_entities_in_text_are_preserved() {
        // Сущности (&quot; &amp; &lt; &#34;) quick-xml отдаёт отдельными событиями —
        // импортёр должен их раскрывать, а не терять.
        const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package name="A &amp; B" version="4" xmlns="http://vladimirkhil.com/ygpackage3.0.xsd">
  <rounds><round name="Р"><themes><theme name="Т"><questions>
    <question price="100">
      <scenario><atom>Что значит 5 &lt; 6 &amp; &quot;да&quot;?</atom></scenario>
      <right><answer>Дуэйн &quot;Скала&quot; Джонсон &amp; Ко</answer></right>
    </question>
  </questions></theme></themes></round></rounds>
</package>"#;
        let (archive, _) = import_bytes(&make_siq(XML, &[])).unwrap();
        assert_eq!(archive.pack.name, "A & B");
        let q = &archive.pack.rounds[0].themes[0].questions[0];
        assert_eq!(q.question_slides[0].items[0], Content::Text {
            value: r#"Что значит 5 < 6 & "да"?"#.into()
        });
        assert_eq!(q.answer_text(), r#"Дуэйн "Скала" Джонсон & Ко"#);
    }
}
