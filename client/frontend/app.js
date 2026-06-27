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
};

// --- Элементы интерфейса ---
const els = {
  status: document.getElementById("status"),
  rounds: document.getElementById("rounds"),
  board: document.getElementById("board"),
  overlay: document.getElementById("overlay"),
  questionContent: document.getElementById("question-content"),
  answer: document.getElementById("answer"),
  pathInput: document.getElementById("path-input"),
};

// --- Кнопки ---
document.getElementById("btn-demo").addEventListener("click", openDemo);
document.getElementById("btn-open").addEventListener("click", () => {
  const path = els.pathInput.value.trim();
  if (path) loadPack(path);
});
document.getElementById("btn-answer").addEventListener("click", () => {
  els.answer.classList.remove("hidden");
});
document.getElementById("btn-close").addEventListener("click", closeOverlay);

// --- Загрузка пака ---
async function openDemo() {
  try {
    const path = await invoke("demo_pack_path");
    els.pathInput.value = path;
    await loadPack(path);
  } catch (e) {
    showError(e);
  }
}

async function loadPack(path) {
  try {
    setStatus("Загрузка…");
    const pack = window.legacyizePack(await invoke("open_pack", { path }));
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

// --- Открытие вопроса ---
function openQuestion(question, key) {
  state.used.add(key);

  els.questionContent.innerHTML = "";
  for (const item of question.content) {
    els.questionContent.appendChild(renderContent(item));
  }

  els.answer.textContent = `Ответ: ${question.answer || "—"}`;
  els.answer.classList.add("hidden");
  els.overlay.classList.remove("hidden");
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
