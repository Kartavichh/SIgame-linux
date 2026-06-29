# Экспорт `.sgpack` → `.siq` (родной формат SIGame)

Цель: выгружать наши паки в формат оригинального SIGame, чтобы их можно было
открыть в нём (и наоборот — мы уже умеем импортировать, см. [siq-import.md](siq-import.md)).

## Формат-цель: классический (version 4)

Экспортируем в **классический** формат (`version="4"`,
`xmlns="http://vladimirkhil.com/ygpackage3.0.xsd"`) — он самый совместимый и
именно его пишет настоящий SIGame. Новый формат (SIStorage v5) не используем.

Образец взят с реального рабочего пака: служебный `[Content_Types].xml` **не
обязателен** (реальные паки его не содержат и открываются), обычные раунды идут
**без** атрибута `type`, медиа-ссылки — литеральные имена (кириллица/регистр
сохраняются), `marker` — обычный атом.

## Структура

```xml
<?xml version="1.0" encoding="utf-8"?>
<package name="Имя" version="4" date="дд.мм.гггг" xmlns="http://vladimirkhil.com/ygpackage3.0.xsd">
  <info><authors><author>Автор</author></authors></info>
  <rounds>
    <round name="Раунд 1">                  <!-- обычный: без type -->
      <themes><theme name="Тема"><questions>
        <question price="100">
          <scenario>
            <atom>Текст вопроса</atom>
            <atom type="image">@pic.jpg</atom>
            <atom type="marker"></atom>      <!-- граница вопрос/ответ -->
            <atom type="voice">@ans.mp3</atom>
          </scenario>
          <right><answer>Ответ</answer></right>
          <type name="auction"/>             <!-- только для особых -->
        </question>
      </questions></theme></themes>
    </round>
    <round name="Финал" type="final">…</round>
  </rounds>
</package>
```

Медиа кладутся в папки `Images/`, `Audio/`, `Video/` под теми же именами, что
в ссылках.

## Маппинг (инверсия импорта)

Экспорт строится как **точная инверсия** импорта (`core::siq`), чтобы работал
круг `.sgpack → .siq → импорт обратно` без потерь.

| `.sgpack` (наша модель)            | `.siq`                                  |
|------------------------------------|------------------------------------------|
| `Pack.name`                        | `package@name`                           |
| `Pack.author` (через `, `)         | `info/authors/author` (по одному)        |
| `Round` (обычный)                  | `round` (без `type`)                     |
| `Round.is_final = true`            | `round type="final"`                     |
| `Theme.name`                       | `theme@name`                             |
| `Question.price`                   | `question@price`                         |
| блоки `question_slides`            | атомы `scenario` (до `marker`)           |
| → `marker`                         | `<atom type="marker">`                   |
| текст из `answer_slides`           | `right/answer`                           |
| медиа из `answer_slides`           | атомы `scenario` (после `marker`)        |
| `QuestionKind` (≠ Normal)          | `type@name` (см. ниже)                   |

### Блоки → атомы

| `Content`           | `atom`                              | папка     |
|---------------------|-------------------------------------|-----------|
| `Text`              | `<atom>текст</atom>`                | —         |
| `Image`             | `<atom type="image">@имя</atom>`   | `Images/` |
| `Audio`             | `<atom type="voice">@имя</atom>`   | `Audio/`  |
| `Video`             | `<atom type="video">@имя</atom>`   | `Video/`  |

### Ответ

- Текст ответа берём через `Question::answer_text()` (текстовые блоки ответа) и
  пишем в `<right><answer>…</answer></right>`.
- Медиа-блоки ответа идут атомами **после** `marker`.
- Так импорт обратно соберёт ту же структуру: текстовый слайд ответа из
  `right/answer` + медиа-слайды из атомов после маркера.

### Типы вопросов

| `QuestionKind` | `type@name` |
|----------------|-------------|
| `Normal`       | (нет `<type>`) |
| `Auction`      | `auction`   |
| `CatInBag`     | `cat`       |
| `NoRisk`       | `noRisk`    |

## API

- `core`: `export_siq(archive: &PackArchive, path)` и `export_siq_bytes(archive)`.
- Экранирование XML — через `quick_xml::escape::escape`.
- Тест: круг `import → export → import` на синтетическом и реальном паке даёт
  идентичный `Pack`.
