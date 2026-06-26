// Сетевой игровой режим. Rust-часть клиента — «труба» к серверу: мы шлём ей
// команды через invoke("net_send", ...) и слушаем входящие сообщения через
// событие "net:message". Здесь — вся логика интерфейса игры.
(function () {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  let initialized = false;
  let connected = false;
  let you = { id: null, host: false }; // кто мы: id игрока (или null у ведущего)
  let snap = null;                      // последний снимок состояния
  let serverHost = "127.0.0.1";        // адрес сервера (для URL медиа)
  let mediaBase = "";                   // http://<сервер>:<медиа-порт>/

  const E = {};

  // Человекочитаемые названия фаз.
  const PHASE = {
    lobby: "Лобби",
    picking: "Выбор вопроса",
    question: "Вопрос — жмите кнопку",
    answering: "Ответ игрока",
    round_over: "Раунд завершён",
    final_theme_removal: "Финал — вычёркивание тем",
    final_bets: "Финал — ставки",
    final_answers: "Финал — ответы",
    final_reveal: "Финал — вскрытие",
    game_over: "Игра окончена",
  };

  const isFinalPhase = (p) => p && p.startsWith("final_");

  window.netInit = function () {
    if (initialized) return;
    initialized = true;

    E.host = byId("net-host");
    E.port = byId("net-port");
    E.name = byId("net-name");
    E.role = byId("net-host-role");
    E.connect = byId("net-connect");
    E.disconnect = byId("net-disconnect");
    E.status = byId("net-status");
    E.game = byId("play-game");
    E.meta = byId("play-meta");
    E.players = byId("play-players");
    E.main = byId("play-main");
    E.controls = byId("play-controls");

    E.connect.addEventListener("click", connect);
    E.disconnect.addEventListener("click", disconnect);

    listen("net:message", (e) => onMessage(e.payload));
    listen("net:closed", () => onClosed());
  };

  async function connect() {
    const name = E.name.value.trim();
    if (!name) {
      setStatus("Введите имя.");
      return;
    }
    const host = E.host.value.trim() || "127.0.0.1";
    const port = parseInt(E.port.value, 10) || 7777;
    const isHost = E.role.checked;
    try {
      you = { id: null, host: isHost };
      serverHost = host;
      await invoke("net_connect", { host, port, name, isHost });
      connected = true;
      E.connect.disabled = true;
      E.disconnect.disabled = false;
      setStatus(`Подключение к ${host}:${port}…`);
    } catch (err) {
      setStatus(`Ошибка подключения: ${err}`);
    }
  }

  async function disconnect() {
    await invoke("net_disconnect");
    onClosed();
  }

  function onClosed() {
    connected = false;
    snap = null;
    E.connect.disabled = false;
    E.disconnect.disabled = true;
    E.game.classList.add("hidden");
    setStatus("Отключено.");
  }

  function onMessage(raw) {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    switch (msg.type) {
      case "welcome":
        you = { id: msg.id, host: msg.host };
        mediaBase = `http://${serverHost}:${msg.media_port}/`;
        setStatus(msg.host ? "Подключён как ведущий." : "Подключён как игрок.");
        break;
      case "state":
        snap = msg;
        render();
        break;
      case "error":
        setStatus(`Сервер: ${msg.message}`);
        break;
    }
  }

  function send(obj) {
    invoke("net_send", { line: JSON.stringify(obj) }).catch((e) =>
      setStatus(`Ошибка отправки: ${e}`)
    );
  }

  // ----------------------------- Отрисовка -----------------------------

  function render() {
    if (!snap) return;
    E.game.classList.remove("hidden");

    const phaseLabel = PHASE[snap.phase] || snap.phase;
    const roleLabel = you.host ? "🎙 Ведущий" : "🎮 Игрок";
    const roundInfo = snap.round_count
      ? `Раунд ${snap.round_index + 1}/${snap.round_count}: ${snap.round_name}`
      : "";
    E.meta.textContent = `${roleLabel} · ${phaseLabel}${roundInfo ? " · " + roundInfo : ""}`;

    renderPlayers();
    E.main.innerHTML = "";
    if (you.host) renderHostMain();
    else renderPlayerMain();
    renderControls();
  }

  function renderPlayers() {
    E.players.innerHTML = "<h3>Игроки</h3>";
    if (!snap.players.length) {
      E.players.appendChild(el("p", "Пока никого."));
      return;
    }
    const cur = snap.current;
    for (const p of snap.players) {
      const row = el("div", "");
      row.className = "play-player";
      if (you.id === p.id) row.classList.add("self");
      if (snap.picker === p.id) row.classList.add("picker");
      if (cur && cur.buzzed === p.id) row.classList.add("buzzed");
      const tags = [];
      if (snap.picker === p.id) tags.push("◆");
      if (cur && cur.buzzed === p.id) tags.push("🔔");
      if (!p.online) tags.push("⚪");
      row.textContent = `${p.name}: ${p.score}${tags.length ? "  " + tags.join(" ") : ""}`;
      E.players.appendChild(row);
    }
  }

  // ----------------------------- Экран игрока -----------------------------

  function renderPlayerMain() {
    const cur = snap.current;
    const me = snap.players.find((p) => p.id === you.id);
    if (me) {
      const sc = el("div", `Ваш счёт: ${me.score}`);
      sc.className = "play-myscore";
      E.main.appendChild(sc);
    }

    if (isFinalPhase(snap.phase)) {
      renderFinalPlayer(snap.finale);
      return;
    }

    if (snap.phase === "lobby") {
      E.main.appendChild(bigState("Ожидание начала игры ведущим…"));
      return;
    }
    if (snap.phase === "game_over") {
      E.main.appendChild(gameOverBlock());
      return;
    }
    if (snap.phase === "round_over") {
      E.main.appendChild(bigState("Раунд завершён. Ждём ведущего…"));
      return;
    }

    if (cur) {
      E.main.appendChild(questionBlock(cur));
      if (snap.phase === "question") {
        const locked = (cur.locked_out || []).includes(you.id);
        const buzz = document.createElement("button");
        buzz.className = "buzz";
        if (locked) {
          buzz.textContent = "Вы уже отвечали на этот вопрос";
          buzz.disabled = true;
        } else {
          buzz.textContent = "ОТВЕТИТЬ";
          buzz.addEventListener("click", () => send({ type: "buzz" }));
        }
        E.main.appendChild(buzz);
      } else if (snap.phase === "answering") {
        E.main.appendChild(
          bigState(
            cur.buzzed === you.id
              ? "Вы отвечаете! Ведущий проверяет ответ."
              : `Отвечает: ${nameOf(cur.buzzed)}`
          )
        );
      }
      return;
    }

    // Фаза выбора.
    const myTurn = snap.picker === you.id && snap.phase === "picking";
    E.main.appendChild(buildBoard(myTurn));
    E.main.appendChild(
      bigState(myTurn ? "Ваш ход — выберите вопрос." : `Выбирает: ${nameOf(snap.picker)}`)
    );
  }

  // ----------------------------- Экран ведущего -----------------------------

  function renderHostMain() {
    const cur = snap.current;

    if (isFinalPhase(snap.phase)) {
      renderFinalHost(snap.finale);
      return;
    }

    if (snap.phase === "lobby") {
      E.main.appendChild(bigState("Игроки собираются. Когда все готовы — «Начать игру»."));
      return;
    }
    if (snap.phase === "game_over") {
      E.main.appendChild(gameOverBlock());
      return;
    }

    if (cur) {
      E.main.appendChild(questionBlock(cur));
      if (cur.answer != null) {
        const a = el("div", `Правильный ответ: ${cur.answer}`);
        a.className = "play-answer";
        E.main.appendChild(a);
      }
      if (snap.phase === "answering") {
        E.main.appendChild(bigState(`Отвечает: ${nameOf(cur.buzzed)} — оцените ответ.`));
      } else if (snap.phase === "question") {
        E.main.appendChild(bigState("Ждём, кто нажмёт кнопку…"));
      }
      return;
    }

    // Выбор / конец раунда — табло только для обзора (ведущий не выбирает).
    E.main.appendChild(buildBoard(false));
    if (snap.phase === "picking") {
      E.main.appendChild(bigState(`Выбирает: ${nameOf(snap.picker)}`));
    } else if (snap.phase === "round_over") {
      E.main.appendChild(bigState("Раунд завершён."));
    }
  }

  function renderControls() {
    E.controls.innerHTML = "";
    if (!you.host) return; // управление ходом — у ведущего

    switch (snap.phase) {
      case "lobby":
        E.controls.appendChild(
          ctrl("Начать игру", () => send({ type: "start" }), snap.players.length === 0)
        );
        break;
      case "question":
        E.controls.appendChild(ctrl("Показать ответ (никто не нажал)", () => send({ type: "reveal" })));
        break;
      case "answering":
        E.controls.appendChild(ctrl("Верно", () => send({ type: "judge", correct: true })));
        E.controls.appendChild(ctrl("Неверно", () => send({ type: "judge", correct: false }), false, "danger"));
        break;
      case "round_over":
        E.controls.appendChild(ctrl("Следующий раунд", () => send({ type: "next_round" })));
        break;
      case "final_reveal":
        E.controls.appendChild(ctrl("Верно", () => send({ type: "final_judge", correct: true })));
        E.controls.appendChild(ctrl("Неверно", () => send({ type: "final_judge", correct: false }), false, "danger"));
        break;
    }
  }

  // ----------------------------- Финал -----------------------------

  function renderFinalPlayer(f) {
    if (!f) return;
    if (!f.you_participant) {
      E.main.appendChild(bigState("Вы не участвуете в финале (счёт не положительный). Наблюдайте."));
    }
    switch (snap.phase) {
      case "final_theme_removal": {
        const myTurn = f.remover === you.id;
        E.main.appendChild(el("h3", "Вычёркивание тем"));
        E.main.appendChild(finalThemes(f, myTurn));
        E.main.appendChild(
          bigState(myTurn ? "Ваш ход — вычеркните одну тему." : `Вычёркивает: ${nameOf(f.remover)}`)
        );
        break;
      }
      case "final_bets":
        E.main.appendChild(finalQuestionBlock(f));
        if (f.you_participant) {
          if (f.you_bet != null) {
            E.main.appendChild(bigState(`Ваша ставка принята: ${f.you_bet}. Ждём остальных…`));
          } else {
            E.main.appendChild(betForm());
          }
        }
        E.main.appendChild(bigState(`Ставки сделали: ${f.bets_in}/${f.total}`));
        break;
      case "final_answers":
        E.main.appendChild(finalQuestionBlock(f));
        if (f.you_participant) {
          if (f.you_answered) {
            E.main.appendChild(bigState("Ответ отправлен. Ждём остальных…"));
          } else {
            E.main.appendChild(answerForm());
          }
        }
        E.main.appendChild(bigState(`Ответили: ${f.answers_in}/${f.total}`));
        break;
      case "final_reveal":
        E.main.appendChild(finalQuestionBlock(f));
        E.main.appendChild(revealList(f));
        break;
    }
  }

  function renderFinalHost(f) {
    if (!f) return;
    switch (snap.phase) {
      case "final_theme_removal":
        E.main.appendChild(el("h3", "Вычёркивание тем"));
        E.main.appendChild(finalThemes(f, false));
        E.main.appendChild(bigState(`Вычёркивает: ${nameOf(f.remover)}`));
        break;
      case "final_bets":
        E.main.appendChild(finalQuestionBlock(f));
        if (f.answer != null) E.main.appendChild(answerBox(f.answer));
        E.main.appendChild(bigState(`Игроки делают ставки: ${f.bets_in}/${f.total}`));
        break;
      case "final_answers":
        E.main.appendChild(finalQuestionBlock(f));
        if (f.answer != null) E.main.appendChild(answerBox(f.answer));
        E.main.appendChild(bigState(`Игроки отвечают: ${f.answers_in}/${f.total}`));
        break;
      case "final_reveal":
        E.main.appendChild(finalQuestionBlock(f));
        if (f.answer != null) E.main.appendChild(answerBox(f.answer));
        E.main.appendChild(revealList(f));
        if (f.current_reveal) {
          E.main.appendChild(
            bigState(
              `Сейчас: ${f.current_reveal.name} — «${f.current_reveal.answer}» (ставка ${f.current_reveal.bet}). Оцените ответ.`
            )
          );
        }
        break;
    }
  }

  function finalThemes(f, clickable) {
    const wrap = el("div", "");
    wrap.className = "final-themes";
    for (const t of f.themes) {
      const b = document.createElement("button");
      b.className = "final-theme";
      b.textContent = t.name;
      if (t.removed) {
        b.classList.add("removed");
        b.disabled = true;
      } else if (clickable) {
        b.addEventListener("click", () => send({ type: "remove_theme", theme: t.index }));
      } else {
        b.disabled = true;
      }
      wrap.appendChild(b);
    }
    return wrap;
  }

  function finalQuestionBlock(f) {
    const box = el("div", "");
    box.className = "play-question";
    if (f.chosen_theme) box.appendChild(el("h3", `Тема: ${f.chosen_theme}`));
    for (const item of f.content) {
      if (item.type === "text") box.appendChild(el("p", item.value));
      else box.appendChild(mediaElement(item));
    }
    return box;
  }

  function revealList(f) {
    const wrap = el("div", "");
    wrap.className = "final-reveallist";
    const rows = [...f.revealed];
    if (f.current_reveal) rows.push(f.current_reveal);
    for (const r of rows) {
      const pending = r.verdict === null || r.verdict === undefined;
      const mark = r.verdict === true ? "✓" : r.verdict === false ? "✗" : "…";
      const sign = r.verdict === true ? `+${r.bet}` : r.verdict === false ? `−${r.bet}` : `ставка ${r.bet}`;
      const row = el("div", `${mark} ${r.name}: «${r.answer}» (${sign})`);
      row.className = "final-reveal-row";
      if (pending) row.classList.add("current");
      else if (r.verdict === true) row.classList.add("ok");
      else row.classList.add("bad");
      wrap.appendChild(row);
    }
    return wrap;
  }

  function betForm() {
    const wrap = el("div", "");
    wrap.className = "final-form";
    const me = snap.players.find((p) => p.id === you.id);
    const max = me ? me.score : 1;
    const inp = document.createElement("input");
    inp.type = "number";
    inp.min = "1";
    inp.max = String(max);
    inp.value = "1";
    const b = ctrl(`Поставить (1..${max})`, () => {
      const v = parseInt(inp.value, 10);
      if (!(v >= 1 && v <= max)) {
        setStatus(`Ставка должна быть от 1 до ${max}.`);
        return;
      }
      send({ type: "final_bet", amount: v });
    });
    wrap.appendChild(inp);
    wrap.appendChild(b);
    return wrap;
  }

  function answerForm() {
    const wrap = el("div", "");
    wrap.className = "final-form";
    const inp = document.createElement("input");
    inp.type = "text";
    inp.placeholder = "ваш ответ";
    const b = ctrl("Ответить", () => {
      const t = inp.value.trim();
      if (!t) {
        setStatus("Введите ответ.");
        return;
      }
      send({ type: "final_answer", text: t });
    });
    wrap.appendChild(inp);
    wrap.appendChild(b);
    return wrap;
  }

  function answerBox(text) {
    const a = el("div", `Правильный ответ: ${text}`);
    a.className = "play-answer";
    return a;
  }

  // ----------------------------- Общие блоки -----------------------------

  // Табло; cells кликабельны только если clickable=true.
  function buildBoard(clickable) {
    const table = document.createElement("table");
    table.className = "play-board";

    const head = table.insertRow();
    for (const theme of snap.board) {
      const th = document.createElement("th");
      th.textContent = theme.name;
      head.appendChild(th);
    }

    const rows = Math.max(0, ...snap.board.map((t) => t.cells.length));
    for (let r = 0; r < rows; r++) {
      const tr = table.insertRow();
      for (const theme of snap.board) {
        const td = tr.insertCell();
        const cell = theme.cells[r];
        if (!cell) continue;
        if (cell.used) {
          td.textContent = "—";
          td.className = "used";
        } else {
          const b = document.createElement("button");
          b.textContent = cell.price;
          b.disabled = !clickable;
          if (clickable) {
            b.addEventListener("click", () =>
              send({ type: "pick", theme: cell.theme, question: cell.question })
            );
          }
          td.appendChild(b);
        }
      }
    }
    return table;
  }

  // Блок вопроса (без правильного ответа).
  function questionBlock(cur) {
    const box = el("div", "");
    box.className = "play-question";
    box.appendChild(el("h3", `Вопрос за ${cur.price}`));
    for (const item of cur.content) {
      if (item.type === "text") {
        box.appendChild(el("p", item.value));
      } else {
        box.appendChild(mediaElement(item));
      }
    }
    return box;
  }

  function gameOverBlock() {
    const box = el("div", "");
    box.appendChild(el("h2", "Игра окончена"));
    const sorted = [...snap.players].sort((a, b) => b.score - a.score);
    const ol = document.createElement("ol");
    ol.className = "play-results";
    for (const p of sorted) ol.appendChild(el("li", `${p.name}: ${p.score}`));
    box.appendChild(ol);
    return box;
  }

  function bigState(text) {
    const d = el("div", text);
    d.className = "play-bigstate";
    return d;
  }

  function nameOf(id) {
    const p = snap.players.find((x) => x.id === id);
    return p ? p.name : "?";
  }

  // Создаёт элемент медиа с URL раздачи сервера.
  function mediaElement(item) {
    const url = mediaBase + encodeURIComponent(item.value);
    let elm;
    if (item.type === "image") {
      elm = document.createElement("img");
      elm.src = url;
    } else if (item.type === "video") {
      elm = document.createElement("video");
      elm.src = url;
      elm.controls = true;
      elm.autoplay = true;
    } else if (item.type === "audio") {
      elm = document.createElement("audio");
      elm.src = url;
      elm.controls = true;
      elm.autoplay = true;
    } else {
      return el("p", `[${item.type}] ${item.value}`);
    }
    elm.className = "play-media";
    return elm;
  }

  // ----------------------------- Вспомогательное -----------------------------

  function byId(id) {
    return document.getElementById(id);
  }
  function el(tag, text) {
    const e = document.createElement(tag);
    if (text != null) e.textContent = text;
    return e;
  }
  function ctrl(text, onClick, disabled, cls) {
    const b = document.createElement("button");
    b.textContent = text;
    if (disabled) b.disabled = true;
    if (cls) b.className = cls;
    b.addEventListener("click", onClick);
    return b;
  }
  function setStatus(text) {
    E.status.textContent = text;
  }
})();
