// Обёртка-IIFE: изолирует область видимости от других скриптов (main.js/editor.js).
(function () {
// Доступ к API Tauri (включён через withGlobalTauri в tauri.conf.json).
const { invoke } = window.__TAURI__.core;

// Текущее состояние клиента.
let state = {
  pack: null,        // структура пака (pack.json)
  mediaBase: "",     // базовый URL медиа-сервера, http://127.0.0.1:порт/
  roundIndex: 0,     // выбранный раунд
  used: new Set(),   // ключи "r-t-q" уже открытых вопросов
  // Текущий открытый вопрос (слайдовый просмотр):
  viewer: null,      // { qSlides, aSlides, showingAnswer, index }
};

// --- Элементы интерфейса ---
const els = {
  status: document.getElementById("status"),
  rounds: document.getElementById("rounds"),
  board: document.getElementById("board"),
  overlay: document.getElementById("overlay"),
  questionContent: document.getElementById("question-content"),
  slidePhase: document.getElementById("slide-phase"),
  slideIndicator: document.getElementById("slide-indicator"),
  btnPrev: document.getElementById("btn-prev-slide"),
  btnNext: document.getElementById("btn-next-slide"),
  btnAnswer: document.getElementById("btn-answer"),
};

// --- Кнопки ---
document.getElementById("btn-demo").addEventListener("click", openDemo);
document.getElementById("btn-open").addEventListener("click", openPackDialog);
els.btnPrev.addEventListener("click", () => moveSlide(-1));
els.btnNext.addEventListener("click", () => moveSlide(1));
els.btnAnswer.addEventListener("click", showAnswer);
document.getElementById("btn-close").addEventListener("click", closeOverlay);

// --- Загрузка пака ---
async function openDemo() {
  try {
    const path = await invoke("demo_pack_path");
    await loadPack(path);
  } catch (e) {
    showError(e);
  }
}

// Открывает системный диалог выбора .sgpack (как в редакторе).
async function openPackDialog() {
  try {
    const path = await openDialog({
      filters: [{ name: "SIGame pack", extensions: ["sgpack"] }],
    });
    if (path) await loadPack(path);
  } catch (e) {
    showError(e);
  }
}

// Нативный диалог открытия файла (плагин tauri-plugin-dialog).
async function openDialog(opts) {
  const res = await invoke("plugin:dialog|open", {
    options: { multiple: false, directory: false, ...opts },
  });
  // Диалог может вернуть строку, массив или объект {path} — приводим к строке.
  if (!res) return null;
  if (typeof res === "string") return res;
  if (Array.isArray(res)) return res.length ? normalizePath(res[0]) : null;
  if (res.path) return res.path;
  return null;
}

function normalizePath(res) {
  if (!res) return null;
  if (typeof res === "string") return res;
  if (res.path) return res.path;
  return null;
}

async function loadPack(path) {
  try {
    setStatus("Загрузка…");
    const pack = await invoke("open_pack", { path });
    state.pack = pack;
    state.mediaBase = await invoke("media_base_url");
    state.roundIndex = 0;
    state.used = new Set();
    setStatus(`Загружен пак: «${pack.name}»`, true);
    renderRounds();
    renderBoard();
  } catch (e) {
    showError(e);
  }
}

// --- Отрисовка вкладок раундов ---
function renderRounds() {
  els.rounds.innerHTML = "";
  state.pack.rounds.forEach((round, i) => {
    const btn = document.createElement("button");
    btn.textContent = round.name || `Раунд ${i + 1}`;
    if (i === state.roundIndex) btn.classList.add("active");
    btn.addEventListener("click", () => {
      state.roundIndex = i;
      renderRounds();
      renderBoard();
    });
    els.rounds.appendChild(btn);
  });
}

// --- Отрисовка табло ---
function renderBoard() {
  els.board.innerHTML = "";
  const round = state.pack.rounds[state.roundIndex];
  if (!round) return;

  round.themes.forEach((theme, t) => {
    const row = document.createElement("div");
    row.className = "theme-row";

    const name = document.createElement("div");
    name.className = "theme-name";
    name.textContent = theme.name;
    row.appendChild(name);

    theme.questions.forEach((q, qi) => {
      const cell = document.createElement("button");
      cell.className = "cell";
      cell.textContent = q.price;
      const key = `${state.roundIndex}-${t}-${qi}`;
      if (state.used.has(key)) cell.classList.add("used");
      cell.addEventListener("click", () => {
        if (state.used.has(key)) return;
        openQuestion(q, key);
      });
      row.appendChild(cell);
    });

    els.board.appendChild(row);
  });
}

// --- Открытие вопроса (слайдовый просмотр) ---
// Вопрос — это последовательность слайдов (question_slides), затем по кнопке
// «Показать ответ» — слайды ответа (answer_slides). Ведущий листает их вручную,
// как в SIGame.
function openQuestion(question, key) {
  state.used.add(key);

  const qSlides = slidesOf(question.question_slides);
  let aSlides = slidesOf(question.answer_slides);
  // Если слайдов ответа нет вовсе — показываем заглушку, чтобы «Показать ответ»
  // всё равно что-то открывал.
  if (aSlides.length === 0) aSlides = [[{ type: "text", value: "—" }]];

  state.viewer = { qSlides, aSlides, showingAnswer: false, index: 0 };
  els.overlay.classList.remove("hidden");
  renderSlide();
}

// Нормализует слайды из пака в массив массивов-блоков: [[item, item], [item]].
// Терпимо относится к отсутствию слайдов/пустым полям.
function slidesOf(slides) {
  if (!Array.isArray(slides)) return [];
  return slides.map((s) => (s && Array.isArray(s.items) ? s.items : []));
}

// Текущий список слайдов (вопрос или ответ).
function activeSlides() {
  const v = state.viewer;
  return v.showingAnswer ? v.aSlides : v.qSlides;
}

// Рисует текущий слайд + обновляет навигацию.
function renderSlide() {
  const v = state.viewer;
  const slides = activeSlides();
  const items = slides[v.index] || [];

  els.questionContent.innerHTML = "";
  for (const item of items) {
    els.questionContent.appendChild(renderContent(item));
  }

  // Подпись фазы и индикатор «слайд N из M».
  const phase = v.showingAnswer ? "Ответ" : "Вопрос";
  els.slidePhase.textContent = phase;
  els.slidePhase.classList.toggle("answer-phase", v.showingAnswer);
  els.slideIndicator.textContent =
    slides.length > 1 ? `слайд ${v.index + 1} из ${slides.length}` : "";

  // Навигация: «Назад» доступна не на первом слайде; «Далее» — не на последнем.
  els.btnPrev.disabled = v.index === 0;
  els.btnNext.disabled = v.index >= slides.length - 1;
  // «Показать ответ» — только пока показываем вопрос.
  els.btnAnswer.classList.toggle("hidden", v.showingAnswer);
}

// Листание в пределах текущей фазы.
function moveSlide(delta) {
  const v = state.viewer;
  if (!v) return;
  const slides = activeSlides();
  const next = v.index + delta;
  if (next < 0 || next >= slides.length) return;
  v.index = next;
  renderSlide();
}

// Переход к слайдам ответа.
function showAnswer() {
  const v = state.viewer;
  if (!v || v.showingAnswer) return;
  v.showingAnswer = true;
  v.index = 0;
  renderSlide();
}

// Создаёт DOM-элемент для одной единицы содержимого.
function renderContent(item) {
  switch (item.type) {
    case "text": {
      const p = document.createElement("p");
      p.textContent = item.value;
      return p;
    }
    case "image": {
      const img = document.createElement("img");
      img.src = mediaUrl(item.value);
      return img;
    }
    case "video": {
      const v = document.createElement("video");
      const url = mediaUrl(item.value);
      v.src = url;
      v.controls = true;
      v.preload = "auto";
      attachMediaDebug(v, item.value, url);
      return v;
    }
    case "audio": {
      const a = document.createElement("audio");
      const url = mediaUrl(item.value);
      a.src = url;
      a.controls = true;
      a.preload = "auto";
      attachMediaDebug(a, item.value, url);
      return a;
    }
    default: {
      const p = document.createElement("p");
      p.textContent = `[неизвестный тип: ${item.type}]`;
      return p;
    }
  }
}

// Диагностика воспроизведения медиа: пишем URL и ошибки в #media-debug.
const MEDIA_ERR = {
  1: "ABORTED (прервано)",
  2: "NETWORK (не удалось получить файл)",
  3: "DECODE (ошибка декодирования)",
  4: "SRC_NOT_SUPPORTED (формат/источник не поддерживается)",
};
function attachMediaDebug(el, filename, url) {
  const dbg = document.getElementById("media-debug");
  dbg.textContent = `URL: ${url}`;
  el.addEventListener("loadedmetadata", () => {
    dbg.textContent = `OK, метаданные получены (${filename}).`;
    dbg.classList.add("ok");
  });
  el.addEventListener("error", () => {
    const code = el.error ? el.error.code : "?";
    dbg.classList.remove("ok");
    dbg.textContent = `Ошибка медиа [${code}]: ${MEDIA_ERR[code] || "неизвестно"}. URL: ${url}`;
    console.error("media error", code, el.error, url);
  });
}

// Имя медиафайла -> URL локального HTTP-сервера (его понимает медиаплеер).
function mediaUrl(filename) {
  return state.mediaBase + encodeURIComponent(filename);
}

function closeOverlay() {
  // Останавливаем возможное воспроизведение, очищая содержимое.
  els.questionContent.innerHTML = "";
  els.overlay.classList.add("hidden");
  state.viewer = null;
  renderBoard(); // обновить «использованные» клетки
}

// --- Вспомогательное ---
function setStatus(text, ok = false) {
  els.status.textContent = text;
  els.status.classList.toggle("ok", ok);
}
function showError(e) {
  setStatus(`Ошибка: ${e}`, false);
  console.error(e);
}
})();
