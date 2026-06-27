// Мост совместимости (Этап 10a): ядро теперь хранит вопрос как слайды
// (question_slides/answer_slides). Просмотр и редактор пока работают со старой
// плоской формой (content/answer) — разворачиваем слайды в неё при загрузке.
// Полная работа со слайдами появится в UI на подэтапах 10d/10e.
window.legacyizePack = function (pack) {
  if (!pack || !Array.isArray(pack.rounds)) return pack;
  for (const round of pack.rounds) {
    for (const theme of round.themes || []) {
      for (const q of theme.questions || []) {
        if (Array.isArray(q.question_slides)) {
          q.content = q.question_slides.flatMap((s) => s.items || []);
          delete q.question_slides;
        }
        if (q.content == null) q.content = [];
        if (Array.isArray(q.answer_slides)) {
          q.answer = q.answer_slides
            .flatMap((s) => s.items || [])
            .filter((it) => it.type === "text")
            .map((it) => it.value)
            .join(" ");
          delete q.answer_slides;
        }
        if (q.answer == null) q.answer = "";
      }
    }
  }
  return pack;
};

// Переключение между режимами «Просмотр», «Редактор» и «Игра».
const sections = {
  view: document.getElementById("view-section"),
  edit: document.getElementById("edit-section"),
  play: document.getElementById("play-section"),
};
const buttons = {
  view: document.getElementById("mode-view"),
  edit: document.getElementById("mode-edit"),
  play: document.getElementById("mode-play"),
};

buttons.view.addEventListener("click", () => setMode("view"));
buttons.edit.addEventListener("click", () => setMode("edit"));
buttons.play.addEventListener("click", () => setMode("play"));

function setMode(mode) {
  for (const key of Object.keys(sections)) {
    sections[key].classList.toggle("hidden", key !== mode);
    buttons[key].classList.toggle("active", key === mode);
  }

  // Ленивая инициализация при первом входе в режим.
  if (mode === "edit" && typeof window.editorInit === "function") {
    window.editorInit();
  }
  if (mode === "play" && typeof window.netInit === "function") {
    window.netInit();
  }
}
