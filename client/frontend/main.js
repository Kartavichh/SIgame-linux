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
