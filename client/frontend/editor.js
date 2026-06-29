// Обёртка-IIFE: изолирует область видимости от других скриптов (main.js/app.js).
(function () {
// Редактор паков.
const { invoke } = window.__TAURI__.core;

let model = null;     // редактируемый Pack (структура pack.json)
let mediaBase = "";   // http://127.0.0.1:порт/ для превью медиа
let selRound = 0;     // выбранный раунд
let initialized = false;

const E = {
  status: byId("ed-status"),
  rounds: byId("ed-rounds"),
  round: byId("ed-round"),
  name: byId("ed-name"),
  author: byId("ed-author"),
};

// --- Кнопки тулбара ---
byId("ed-new").addEventListener("click", edNew);
byId("ed-open").addEventListener("click", edOpen);
byId("ed-import-siq").addEventListener("click", edImportSiq);
byId("ed-save").addEventListener("click", edSave);
byId("ed-export-siq").addEventListener("click", edExportSiq);
E.name.addEventListener("input", () => { if (model) model.name = E.name.value; });
E.author.addEventListener("input", () => { if (model) model.author = E.author.value; });

// Первый вход в редактор — создаём пустой пак.
window.editorInit = async function () {
  if (initialized) return;
  initialized = true;
  mediaBase = await invoke("media_base_url");
  await edNew();
};

// ---------------- Операции с паком ----------------

async function edNew() {
  model = await invoke("editor_new", { name: "Новый пак", author: "" });
  selRound = 0;
  renderAll();
  setStatus("Создан новый пак.", true);
}

async function edOpen() {
  const path = await openDialog({ filters: [{ name: "SIGame pack", extensions: ["sgpack"] }] });
  if (!path) return;
  try {
    model = await invoke("editor_load", { path });
    selRound = 0;
    renderAll();
    setStatus(`Открыт: ${path}`, true);
  } catch (e) {
    setStatus(`Ошибка: ${e}`);
  }
}

async function edImportSiq() {
  const path = await openDialog({ filters: [{ name: "SIGame .siq", extensions: ["siq"] }] });
  if (!path) return;
  try {
    setStatus("Импорт .siq…");
    const res = await invoke("import_siq", { path });
    model = res.pack;
    selRound = 0;
    renderAll();
    if (res.warnings && res.warnings.length) {
      // Показываем сводку: импорт удался, но часть медиа не нашлась.
      const n = res.warnings.length;
      console.warn("Предупреждения импорта .siq:", res.warnings);
      setStatus(
        `Импортирован .siq (предупреждений: ${n}, см. консоль). ` +
        `Проверьте и «Сохранить как… .sgpack».`
      );
    } else {
      setStatus(`Импортирован .siq. Проверьте и «Сохранить как… .sgpack».`, true);
    }
  } catch (e) {
    setStatus(`Не удалось импортировать .siq: ${e}`);
  }
}

async function edSave() {
  model.name = E.name.value;
  model.author = E.author.value;
  const path = await saveDialog({
    defaultPath: `${model.name || "pack"}.sgpack`,
    filters: [{ name: "SIGame pack", extensions: ["sgpack"] }],
  });
  if (!path) return;
  try {
    await invoke("editor_save", { path, pack: model });
    setStatus(`Сохранено: ${path}`, true);
  } catch (e) {
    setStatus(`Ошибка: ${e}`);
  }
}

async function edExportSiq() {
  model.name = E.name.value;
  model.author = E.author.value;
  const path = await saveDialog({
    defaultPath: `${model.name || "pack"}.siq`,
    filters: [{ name: "SIGame .siq", extensions: ["siq"] }],
  });
  if (!path) return;
  try {
    await invoke("export_siq", { path, pack: model });
    setStatus(`Экспортировано в .siq: ${path}`, true);
  } catch (e) {
    setStatus(`Ошибка экспорта .siq: ${e}`);
  }
}

// Добавить медиа-блок в конкретный слайд (его массив items).
async function addMedia(items) {
  const path = await openDialog({
    filters: [{ name: "Медиа", extensions: ["png", "jpg", "jpeg", "gif", "webp", "webm", "mp4", "mp3", "ogg", "wav"] }],
  });
  if (!path) return;
  try {
    const res = await invoke("editor_add_media", { srcPath: path });
    items.push({ type: inferType(res.filename), value: res.filename });
    renderRound();
  } catch (e) {
    setStatus(`Ошибка добавления медиа: ${e}`);
  }
}

// ---------------- Отрисовка ----------------

function renderAll() {
  E.name.value = model.name || "";
  E.author.value = model.author || "";
  renderRoundTabs();
  renderRound();
}

function renderRoundTabs() {
  E.rounds.innerHTML = "";
  model.rounds.forEach((r, i) => {
    const b = btn(r.name || `Раунд ${i + 1}`, () => {
      selRound = i;
      renderRoundTabs();
      renderRound();
    });
    if (i === selRound) b.classList.add("active");
    E.rounds.appendChild(b);
  });
  E.rounds.appendChild(btn("+ Раунд", () => {
    model.rounds.push({ name: `Раунд ${model.rounds.length + 1}`, is_final: false, themes: [] });
    selRound = model.rounds.length - 1;
    renderRoundTabs();
    renderRound();
  }));
}

function renderRound() {
  E.round.innerHTML = "";
  const round = model.rounds[selRound];
  if (!round) {
    E.round.textContent = "Раундов нет. Нажмите «+ Раунд».";
    return;
  }

  const head = document.createElement("div");
  head.className = "ed-head";
  head.appendChild(label("Раунд:", textInput(round.name, (v) => (round.name = v), "Название раунда")));

  // Чекбокс «финальный раунд».
  const finalChk = document.createElement("input");
  finalChk.type = "checkbox";
  finalChk.checked = !!round.is_final;
  finalChk.addEventListener("change", () => {
    round.is_final = finalChk.checked;
    renderRound();
  });
  head.appendChild(label("Финальный раунд", finalChk));

  head.appendChild(btn("Удалить раунд", () => {
    model.rounds.splice(selRound, 1);
    selRound = Math.max(0, selRound - 1);
    renderRoundTabs();
    renderRound();
  }, "danger"));
  E.round.appendChild(head);

  if (round.is_final) {
    const hint = document.createElement("p");
    hint.className = "status ok";
    hint.textContent =
      "Финал: каждая тема — один вариант для вычёркивания; играется первый вопрос оставшейся темы. Стоимость не используется (ставки делают игроки).";
    E.round.appendChild(hint);
  }

  round.themes.forEach((theme, ti) => E.round.appendChild(renderTheme(round, theme, ti)));
  E.round.appendChild(btn("+ Тема", () => {
    round.themes.push({ name: "Новая тема", questions: [] });
    renderRound();
  }));
}

function renderTheme(round, theme, ti) {
  const card = document.createElement("div");
  card.className = "ed-theme";

  const head = document.createElement("div");
  head.className = "ed-head";
  head.appendChild(label("Тема:", textInput(theme.name, (v) => (theme.name = v), "Название темы")));
  head.appendChild(btn("Удалить тему", () => {
    round.themes.splice(ti, 1);
    renderRound();
  }, "danger"));
  card.appendChild(head);

  theme.questions.forEach((q, qi) => card.appendChild(renderQuestion(theme, q, qi)));
  card.appendChild(btn("+ Вопрос", () => {
    theme.questions.push({
      price: 100,
      kind: "normal",
      question_slides: [{ items: [{ type: "text", value: "" }] }],
      answer_slides: [{ items: [{ type: "text", value: "" }] }],
    });
    renderRound();
  }));
  return card;
}

function renderQuestion(theme, q, qi) {
  const box = document.createElement("div");
  box.className = "ed-question";

  const head = document.createElement("div");
  head.className = "ed-head";
  const price = document.createElement("input");
  price.type = "number";
  price.min = "0";
  price.value = q.price;
  price.className = "ed-price";
  price.addEventListener("input", () => (q.price = parseInt(price.value, 10) || 0));
  head.appendChild(label("Стоимость:", price));

  // Тип вопроса (обычный/особый).
  const kindSel = document.createElement("select");
  kindSel.className = "ed-kind";
  for (const [val, text] of [
    ["normal", "Обычный"],
    ["auction", "Аукцион"],
    ["cat_in_bag", "Кот в мешке"],
    ["no_risk", "Без риска"],
  ]) {
    const opt = document.createElement("option");
    opt.value = val;
    opt.textContent = text;
    kindSel.appendChild(opt);
  }
  kindSel.value = q.kind || "normal";
  kindSel.addEventListener("change", () => (q.kind = kindSel.value));
  head.appendChild(label("Тип:", kindSel));

  head.appendChild(btn("Удалить вопрос", () => {
    theme.questions.splice(qi, 1);
    renderRound();
  }, "danger"));
  box.appendChild(head);

  // На случай старого формата в памяти — гарантируем наличие массивов слайдов.
  if (!Array.isArray(q.question_slides)) q.question_slides = [];
  if (!Array.isArray(q.answer_slides)) q.answer_slides = [];

  box.appendChild(renderSlideSection(q.question_slides, "Слайды вопроса", "Текст вопроса"));
  box.appendChild(renderSlideSection(q.answer_slides, "Слайды ответа", "Текст ответа"));
  return box;
}

// Секция слайдов (вопроса или ответа): список слайдов + кнопка добавления.
function renderSlideSection(slides, title, textPlaceholder) {
  const sec = document.createElement("div");
  sec.className = "ed-slides";

  const h = document.createElement("div");
  h.className = "ed-slides-title";
  h.textContent = title;
  sec.appendChild(h);

  slides.forEach((slide, si) =>
    sec.appendChild(renderSlide(slides, slide, si, textPlaceholder))
  );

  sec.appendChild(btn("+ Слайд", () => {
    slides.push({ items: [] });
    renderRound();
  }, "small"));
  return sec;
}

// Один слайд: шапка с перемещением/удалением, блоки и кнопки добавления блоков.
function renderSlide(slides, slide, si, textPlaceholder) {
  if (!Array.isArray(slide.items)) slide.items = [];
  const card = document.createElement("div");
  card.className = "ed-slide";

  const head = document.createElement("div");
  head.className = "ed-slide-head";
  const cap = document.createElement("span");
  cap.className = "ed-slide-cap";
  cap.textContent = `Слайд ${si + 1}/${slides.length}`;
  head.appendChild(cap);

  head.appendChild(moveBtn("↑", () => move(slides, si, -1), si === 0));
  head.appendChild(moveBtn("↓", () => move(slides, si, +1), si === slides.length - 1));
  head.appendChild(btn("✕ слайд", () => {
    slides.splice(si, 1);
    renderRound();
  }, "danger small"));
  card.appendChild(head);

  slide.items.forEach((item, ci) =>
    card.appendChild(renderSlideItem(slide.items, item, ci, textPlaceholder))
  );

  const tools = document.createElement("div");
  tools.className = "ed-tools";
  tools.appendChild(btn("+ Текст", () => {
    slide.items.push({ type: "text", value: "" });
    renderRound();
  }, "small"));
  tools.appendChild(btn("+ Медиа", () => addMedia(slide.items), "small"));
  card.appendChild(tools);
  return card;
}

// Один блок слайда (текст или медиа) с перемещением и удалением.
function renderSlideItem(items, item, ci, textPlaceholder) {
  const row = document.createElement("div");
  row.className = "ed-item";

  if (item.type === "text") {
    const ta = document.createElement("textarea");
    ta.value = item.value;
    ta.rows = 2;
    ta.placeholder = textPlaceholder;
    ta.addEventListener("input", () => (item.value = ta.value));
    row.appendChild(ta);
  } else {
    const tag = document.createElement("span");
    tag.className = "ed-media-tag";
    tag.textContent = `[${item.type}] ${item.value}`;
    row.appendChild(tag);
    row.appendChild(renderPreview(item));
  }

  const ctrls = document.createElement("div");
  ctrls.className = "ed-item-ctrls";
  ctrls.appendChild(moveBtn("↑", () => move(items, ci, -1), ci === 0));
  ctrls.appendChild(moveBtn("↓", () => move(items, ci, +1), ci === items.length - 1));
  ctrls.appendChild(btn("✕", () => {
    items.splice(ci, 1);
    renderRound();
  }, "danger small"));
  row.appendChild(ctrls);
  return row;
}

// Поменять местами элемент массива с соседним (dir = -1 вверх, +1 вниз).
function move(arr, i, dir) {
  const j = i + dir;
  if (j < 0 || j >= arr.length) return;
  [arr[i], arr[j]] = [arr[j], arr[i]];
  renderRound();
}

// Маленькая кнопка перемещения; disabled = серая и без действия.
function moveBtn(text, onClick, disabled) {
  const b = btn(text, disabled ? () => {} : onClick, "small");
  if (disabled) b.disabled = true;
  return b;
}

function renderPreview(item) {
  const url = mediaBase + encodeURIComponent(item.value);
  let el;
  if (item.type === "image") {
    el = document.createElement("img");
    el.src = url;
  } else if (item.type === "video") {
    el = document.createElement("video");
    el.src = url;
    el.controls = true;
  } else if (item.type === "audio") {
    el = document.createElement("audio");
    el.src = url;
    el.controls = true;
  } else {
    el = document.createElement("span");
  }
  el.className = "ed-preview";
  return el;
}

// ---------------- Файловые диалоги ----------------

async function openDialog(opts) {
  const res = await invoke("plugin:dialog|open", {
    options: { multiple: false, directory: false, ...opts },
  });
  return normalizePath(res);
}

async function saveDialog(opts) {
  const res = await invoke("plugin:dialog|save", { options: { ...opts } });
  return normalizePath(res);
}

// Диалог может вернуть строку, массив или объект {path} — приводим к строке.
function normalizePath(res) {
  if (!res) return null;
  if (typeof res === "string") return res;
  if (Array.isArray(res)) return res.length ? normalizePath(res[0]) : null;
  if (res.path) return res.path;
  return null;
}

// ---------------- Вспомогательное ----------------

function inferType(filename) {
  const ext = (filename.split(".").pop() || "").toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp"].includes(ext)) return "image";
  if (["webm", "mp4", "mkv", "mov", "ogv"].includes(ext)) return "video";
  if (["mp3", "ogg", "oga", "wav", "opus"].includes(ext)) return "audio";
  return "image";
}

function byId(id) {
  return document.getElementById(id);
}

function textInput(value, onChange, placeholder) {
  const i = document.createElement("input");
  i.type = "text";
  i.value = value || "";
  if (placeholder) i.placeholder = placeholder;
  i.addEventListener("input", () => onChange(i.value));
  return i;
}

function label(text, el) {
  const l = document.createElement("label");
  l.append(`${text} `);
  l.appendChild(el);
  return l;
}

function btn(text, onClick, cls) {
  const b = document.createElement("button");
  b.textContent = text;
  if (cls) b.className = cls;
  b.addEventListener("click", onClick);
  return b;
}

function setStatus(text, ok = false) {
  E.status.textContent = text;
  E.status.classList.toggle("ok", ok);
}
})();
