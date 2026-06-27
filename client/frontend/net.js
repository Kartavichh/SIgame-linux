// Сетевой игровой режим. Rust-часть клиента — «труба» к серверу: мы шлём ей
// команды через invoke("net_send", ...) и слушаем входящие сообщения через
// событие "net:message". Здесь — вся логика интерфейса игры.
(function () {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  let initialized = false;
  let connected = false;
  let weHostServer = false;             // мы сами подняли локальный сервер партии
  let you = { id: null, host: false }; // кто мы: id игрока (или null у ведущего)
  let snap = null;                      // последний снимок состояния
  let serverHost = "127.0.0.1";        // адрес сервера (для URL медиа)
  let mediaBase = "";                   // http://<сервер>:<медиа-порт>/
  let settingsOpen = false;             // открыто ли меню настроек (оверлей)
  let myAvatar = null;                  // наша аватарка (компактный data-URL) или null

  const E = {};

  // Человекочитаемые названия фаз.
  const PHASE = {
    lobby: "Лобби",
    picking: "Выбор вопроса",
    question: "Вопрос — жмите кнопку",
    auction: "Аукцион — торги",
    cat_give: "Кот в мешке — передача",
    answering: "Ответ игрока",
    show_answer: "Показ ответа",
    round_over: "Раунд завершён",
    final_theme_removal: "Финал — вычёркивание тем",
    final_bets: "Финал — ставки",
    final_answers: "Финал — ответы",
    final_reveal: "Финал — вскрытие",
    game_over: "Игра окончена",
  };

  // Названия особых типов вопросов (для бейджа над вопросом).
  const KIND = {
    auction: "🔨 Аукцион",
    cat_in_bag: "🐱 Кот в мешке",
    no_risk: "🛡 Вопрос без риска",
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
    E.hostBadge = byId("play-host");
    E.players = byId("play-players");
    E.main = byId("play-main");
    E.controls = byId("play-controls");

    E.hostPack = byId("net-host-pack");
    E.hostDemo = byId("net-host-demo");
    E.avatarBtn = byId("net-avatar");
    E.avatarPreview = byId("net-avatar-preview");

    E.connect.addEventListener("click", connect);
    E.disconnect.addEventListener("click", disconnect);
    E.hostPack.addEventListener("click", hostOwnPack);
    E.hostDemo.addEventListener("click", hostDemo);
    E.avatarBtn.addEventListener("click", pickAvatar);
    E.name.addEventListener("input", refreshAvatarPreview);
    refreshAvatarPreview();

    listen("net:message", (e) => onMessage(e.payload));
    listen("net:closed", () => onClosed());
  };

  // Запустить локальный сервер на выбранном файле .sgpack и войти ведущим.
  async function hostOwnPack() {
    const res = await invoke("plugin:dialog|open", {
      options: {
        multiple: false,
        directory: false,
        filters: [{ name: "SIGame pack", extensions: ["sgpack"] }],
      },
    });
    const path = normalizePath(res);
    if (path) hostGame(path);
  }

  // Запустить локальный сервер на встроенном демо-паке и войти ведущим.
  async function hostDemo() {
    try {
      const path = await invoke("demo_pack_path");
      hostGame(path);
    } catch (err) {
      setStatus(`Не удалось взять демо-пак: ${err}`);
    }
  }

  async function hostGame(packPath) {
    const port = parseInt(E.port.value, 10) || 7777;
    const name = E.name.value.trim() || "Ведущий";
    try {
      // Если уже идёт партия / поднят наш сервер — корректно закрываем перед новой.
      // Снимаем флаг заранее, чтобы обработчик закрытия старого соединения не
      // погасил сервер, который мы вот-вот запустим.
      weHostServer = false;
      await invoke("net_disconnect").catch(() => {});
      await invoke("host_stop").catch(() => {});

      setStatus("Запуск сервера…");
      await invoke("host_start", { packPath, port });
      weHostServer = true;

      E.host.value = "127.0.0.1";
      E.role.checked = true;
      you = { id: null, host: true };
      serverHost = "127.0.0.1";
      // Подключаемся к своему серверу с несколькими попытками: ему нужно
      // немного времени, чтобы занять порт.
      await connectWithRetry({ host: "127.0.0.1", port, name, isHost: true, avatar: myAvatar });
      connected = true;
      E.connect.disabled = true;
      E.disconnect.disabled = false;
      setStatus(`Сервер запущен на порту ${port}. Вы — ведущий. Игроки подключаются к вашему IP:${port}.`);
    } catch (err) {
      setStatus(`Не удалось запустить партию: ${err}`);
      await invoke("host_stop").catch(() => {});
      weHostServer = false;
    }
  }

  // Несколько попыток net_connect (сервер мог ещё не успеть занять порт).
  async function connectWithRetry(opts) {
    let lastErr;
    for (let i = 0; i < 12; i++) {
      try {
        await invoke("net_connect", opts);
        return;
      } catch (e) {
        lastErr = e;
        await new Promise((r) => setTimeout(r, 200));
      }
    }
    throw lastErr;
  }

  // Диалог может вернуть строку, массив или объект {path} — приводим к строке.
  function normalizePath(res) {
    if (!res) return null;
    if (typeof res === "string") return res;
    if (Array.isArray(res)) return res.length ? normalizePath(res[0]) : null;
    if (res.path) return res.path;
    return null;
  }

  // ----------------------------- Аватарки -----------------------------

  // Выбрать картинку файлом, уменьшить и сохранить как нашу аватарку.
  async function pickAvatar() {
    try {
      const res = await invoke("plugin:dialog|open", {
        options: {
          multiple: false,
          directory: false,
          filters: [{ name: "Картинка", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
        },
      });
      const path = normalizePath(res);
      if (!path) return;
      const dataUrl = await invoke("read_image_data_url", { path });
      myAvatar = await downscaleImage(dataUrl, 96);
      refreshAvatarPreview();
      // Если уже в партии — сразу сообщаем серверу о смене.
      if (connected) send({ type: "set_avatar", avatar: myAvatar });
    } catch (err) {
      setStatus(`Не удалось загрузить аватар: ${err}`);
    }
  }

  // Уменьшает картинку до квадрата size×size (обрезка по центру), отдаёт data-URL JPEG.
  function downscaleImage(dataUrl, size) {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext("2d");
        const side = Math.min(img.width, img.height);
        const sx = (img.width - side) / 2;
        const sy = (img.height - side) / 2;
        ctx.drawImage(img, sx, sy, side, side, 0, 0, size, size);
        resolve(canvas.toDataURL("image/jpeg", 0.7));
      };
      img.onerror = () => reject("не удалось декодировать картинку");
      img.src = dataUrl;
    });
  }

  // Обновить предпросмотр аватарки рядом с кнопкой.
  function refreshAvatarPreview() {
    if (!E.avatarPreview) return;
    E.avatarPreview.innerHTML = "";
    const name = E.name ? E.name.value : "";
    E.avatarPreview.appendChild(avatarEl(name || "?", myAvatar, 32));
  }

  // Элемент аватарки: картинка либо круг-заглушка с инициалами.
  function avatarEl(name, avatar, size) {
    let node;
    if (avatar) {
      node = document.createElement("img");
      node.src = avatar;
    } else {
      node = el("div", initials(name));
      node.classList.add("avatar-fallback");
      node.style.background = colorFromName(name);
      node.style.fontSize = Math.round(size * 0.42) + "px";
    }
    node.classList.add("avatar");
    node.style.width = size + "px";
    node.style.height = size + "px";
    return node;
  }

  // Инициалы из имени (одна–две буквы).
  function initials(name) {
    const parts = (name || "?").trim().split(/\s+/).filter(Boolean);
    const a = parts[0] ? parts[0][0] : "";
    const b = parts[1] ? parts[1][0] : "";
    return (a + b).toUpperCase() || "?";
  }

  // Устойчивый цвет фона заглушки из имени.
  function colorFromName(name) {
    let h = 0;
    for (const ch of name || "") h = (h * 31 + ch.charCodeAt(0)) % 360;
    return `hsl(${h} 55% 42%)`;
  }

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
      await invoke("net_connect", { host, port, name, isHost, avatar: myAvatar });
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
    if (weHostServer) {
      await invoke("host_stop");
      weHostServer = false;
    }
    onClosed();
  }

  function onClosed() {
    connected = false;
    snap = null;
    E.connect.disabled = false;
    E.disconnect.disabled = true;
    E.game.classList.add("hidden");
    // Если соединение оборвалось само (сервер упал) — снимаем флаг хостинга.
    if (weHostServer) {
      invoke("host_stop").catch(() => {});
      weHostServer = false;
    }
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

    renderHostBadge();
    renderPlayers();
    E.main.innerHTML = "";
    if (you.host) renderHostMain();
    else renderPlayerMain();
    renderControls();

    // Оверлей настроек поверх всего (только в лобби).
    const existing = byId("settings-overlay");
    if (existing) existing.remove();
    if (settingsOpen && snap.phase === "lobby") {
      E.game.appendChild(settingsOverlay(you.host));
    }
  }

  // Текущие настройки с подстановкой значений по умолчанию.
  function currentSettings() {
    const s = snap.settings || {};
    return {
      cat_must_give: s.cat_must_give !== false,
      no_risk_double: !!s.no_risk_double,
      buzz_mode: s.buzz_mode || "manual",
      false_start: !!s.false_start,
      false_start_block_secs: s.false_start_block_secs ?? 3,
      buzz_time_secs: s.buzz_time_secs ?? 5,
      answer_time_secs: s.answer_time_secs ?? 20,
    };
  }

  // Отправить настройки целиком (сервер требует все поля), применив изменение.
  function sendSettings(patch) {
    send(Object.assign({ type: "settings" }, currentSettings(), patch));
  }

  // Бегущая полоса времени (визуальный отсчёт). seconds — длительность.
  function timeBar(seconds) {
    const bar = el("div", "");
    bar.className = "time-bar";
    const fill = el("div", "");
    fill.className = "time-bar-fill";
    fill.style.animationDuration = `${Math.max(1, seconds || 5)}s`;
    bar.appendChild(fill);
    return bar;
  }

  // Бейдж ведущего слева сверху (аватар + ник), как в SIGame.
  function renderHostBadge() {
    E.hostBadge.innerHTML = "";
    const h = snap.host;
    if (!h) return;
    const badge = el("div", "");
    badge.className = "host-badge";
    if (!h.online) badge.classList.add("offline");
    badge.appendChild(avatarEl(h.name, h.avatar, 48));
    const info = el("div", "");
    info.className = "host-badge-info";
    info.appendChild(el("div", "🎙 Ведущий"));
    const nm = el("div", h.name + (h.online ? "" : " ⚪"));
    nm.className = "host-badge-name";
    info.appendChild(nm);
    badge.appendChild(info);
    E.hostBadge.appendChild(badge);
  }

  // Игроки — лентой карточек снизу («места» как в SIGame).
  function renderPlayers() {
    E.players.innerHTML = "";
    if (!snap.players.length) {
      E.players.appendChild(el("div", "Игроков пока нет."));
      return;
    }
    const cur = snap.current;
    for (const p of snap.players) {
      const card = el("div", "");
      card.className = "player-card";
      if (you.id === p.id) card.classList.add("self");
      if (snap.picker === p.id) card.classList.add("picker");
      if (cur && cur.buzzed === p.id) card.classList.add("buzzed");
      if (cur && (cur.locked_out || []).includes(p.id)) card.classList.add("locked");
      if (!p.online) card.classList.add("offline");

      card.appendChild(avatarEl(p.name, p.avatar, 64));

      const nm = el("div", p.name);
      nm.className = "player-card-name";
      card.appendChild(nm);

      const sc = el("div", String(p.score));
      sc.className = "player-card-score";
      card.appendChild(sc);

      const tag = playerTag(p, cur);
      if (tag) {
        const t = el("div", tag);
        t.className = "player-card-tag";
        card.appendChild(t);
      }

      // Ведущий может вручную поправить счёт игрока (✎ в углу карточки).
      if (you.host) {
        const edit = ctrl("✎", () => startScoreEdit(card, p));
        edit.className = "score-edit player-card-edit";
        card.appendChild(edit);
      }
      E.players.appendChild(card);
    }
  }

  // Короткая подпись состояния игрока под счётом.
  function playerTag(p, cur) {
    if (cur && cur.buzzed === p.id) return "🔔 отвечает";
    if (snap.picker === p.id) return "◆ выбирает";
    if (cur && (cur.locked_out || []).includes(p.id)) return "уже отвечал";
    if (!p.online) return "⚪ не в сети";
    return "";
  }

  // Включить редактирование счёта игрока (ведущий).
  function startScoreEdit(row, p) {
    row.innerHTML = "";
    const inp = document.createElement("input");
    inp.type = "number";
    inp.value = String(p.score);
    inp.className = "score-edit-input";
    const save = () => {
      const v = parseInt(inp.value, 10);
      if (!isNaN(v)) send({ type: "set_score", player: p.id, value: v });
    };
    inp.addEventListener("keydown", (e) => { if (e.key === "Enter") save(); });
    row.appendChild(el("span", p.name + ": "));
    row.appendChild(inp);
    row.appendChild(ctrl("✓", save));
    row.appendChild(ctrl("✕", () => render()));
    inp.focus();
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
      E.main.appendChild(settingsButton(false));
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
    if (snap.phase === "auction") {
      renderAuction();
      return;
    }
    if (snap.phase === "cat_give") {
      renderCatGive();
      return;
    }

    if (cur) {
      const s = currentSettings();
      if (snap.phase === "question" && cur.buzzing_open) E.main.appendChild(timeBar(s.buzz_time_secs));
      if (snap.phase === "answering") E.main.appendChild(timeBar(s.answer_time_secs));
      E.main.appendChild(questionBlock(cur));
      if (snap.phase === "question") {
        const locked = (cur.locked_out || []).includes(you.id);
        const blocked = (cur.false_started || []).includes(you.id);
        const buzz = document.createElement("button");
        buzz.className = "buzz";
        if (locked) {
          buzz.textContent = "Вы уже отвечали на этот вопрос";
          buzz.disabled = true;
        } else if (blocked) {
          buzz.textContent = "Фальстарт! Подождите…";
          buzz.disabled = true;
        } else if (cur.buzzing_open) {
          buzz.textContent = "ОТВЕТИТЬ";
          buzz.addEventListener("click", () => send({ type: "buzz" }));
        } else if (s.false_start) {
          buzz.textContent = "ОТВЕТИТЬ (рано — риск фальстарта!)";
          buzz.classList.add("early");
          buzz.addEventListener("click", () => send({ type: "buzz" }));
        } else {
          buzz.textContent = "Ждите сигнала ведущего…";
          buzz.disabled = true;
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
      } else if (snap.phase === "show_answer") {
        E.main.appendChild(bigState("Ведущий показывает ответ…"));
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
      E.main.appendChild(settingsButton(true));
      return;
    }
    if (snap.phase === "game_over") {
      E.main.appendChild(gameOverBlock());
      return;
    }
    if (snap.phase === "auction") {
      renderAuction();
      return;
    }
    if (snap.phase === "cat_give") {
      renderCatGive();
      return;
    }

    if (cur) {
      const s = currentSettings();
      if (snap.phase === "question" && cur.buzzing_open) E.main.appendChild(timeBar(s.buzz_time_secs));
      if (snap.phase === "answering") E.main.appendChild(timeBar(s.answer_time_secs));
      E.main.appendChild(questionBlock(cur));
      if (cur.answer != null) {
        const a = el("div", `Правильный ответ: ${cur.answer}`);
        a.className = "play-answer";
        E.main.appendChild(a);
      }
      if (snap.phase === "answering") {
        const extra = cur.solo ? ` (за ${cur.reward})` : "";
        E.main.appendChild(bigState(`Отвечает: ${nameOf(cur.buzzed)}${extra} — оцените ответ.`));
      } else if (snap.phase === "question") {
        E.main.appendChild(
          bigState(
            cur.buzzing_open
              ? "Кнопки открыты — ждём, кто нажмёт…"
              : "Листайте слайды и нажмите «Открыть кнопки», когда готовы."
          )
        );
      } else if (snap.phase === "show_answer") {
        E.main.appendChild(bigState("Показываете ответ. «Закрыть вопрос», когда закончите."));
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

    const cur = snap.current;
    // Кнопки листания слайдов (вопрос/ответ).
    const slideNav = () => {
      if (!cur) return;
      const atFirst = cur.slide <= 0;
      const atLast = cur.slide >= cur.slide_count - 1;
      E.controls.appendChild(ctrl("◀ Назад", () => send({ type: "prev_slide" }), atFirst));
      E.controls.appendChild(
        ctrl(`Далее ▶  (${cur.slide + 1}/${cur.slide_count})`, () => send({ type: "next_slide" }), atLast)
      );
    };

    switch (snap.phase) {
      case "lobby":
        E.controls.appendChild(
          ctrl("Начать игру", () => send({ type: "start" }), snap.players.length === 0)
        );
        break;
      case "question":
        slideNav();
        if (cur && !cur.buzzing_open) {
          E.controls.appendChild(ctrl("🔔 Открыть кнопки", () => send({ type: "open_buzz" })));
        }
        E.controls.appendChild(ctrl("Показать ответ (никто не нажал)", () => send({ type: "reveal" })));
        E.controls.appendChild(ctrl("Пропустить", () => send({ type: "skip_question" }), false, "danger"));
        break;
      case "answering":
        E.controls.appendChild(ctrl("Верно", () => send({ type: "judge", correct: true })));
        E.controls.appendChild(ctrl("Неверно", () => send({ type: "judge", correct: false }), false, "danger"));
        E.controls.appendChild(ctrl("Пропустить", () => send({ type: "skip_question" })));
        break;
      case "show_answer":
        slideNav();
        E.controls.appendChild(ctrl("Закрыть вопрос ▶", () => send({ type: "close_question" })));
        break;
      case "round_over":
        E.controls.appendChild(ctrl("Следующий раунд", () => send({ type: "next_round" })));
        break;
      case "game_over":
        // Если партию хостим мы — можно тут же поднять новую, выбрав любой пак.
        if (weHostServer) {
          E.controls.appendChild(ctrl("🎮 Новая игра — выбрать пак", () => hostOwnPack()));
          E.controls.appendChild(ctrl("▶ Новая игра — демо-пак", () => hostDemo()));
        }
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
    const me = snap.players.find((p) => p.id === you.id);
    const max = me ? me.score : 1;

    const box = el("div", "");

    const hint = el("div", `Ставка: от 1 до ${max} очков`);
    hint.className = "final-form-hint";
    box.appendChild(hint);

    const wrap = el("div", "");
    wrap.className = "final-form";
    const inp = document.createElement("input");
    inp.type = "number";
    inp.min = "1";
    inp.max = String(max);
    inp.value = "1";
    wrap.appendChild(inp);
    wrap.appendChild(
      ctrl("Сделать ставку", () => {
        const v = parseInt(inp.value, 10);
        if (!(v >= 1 && v <= max)) {
          setStatus(`Ставка должна быть от 1 до ${max}.`);
          return;
        }
        send({ type: "final_bet", amount: v });
      })
    );
    wrap.appendChild(ctrl(`Ва-банк (${max})`, () => send({ type: "final_bet", amount: max }), max < 1));
    box.appendChild(wrap);
    return box;
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

  // ----------------------------- Настройки партии -----------------------------

  // Кнопка открытия меню настроек (в лобби).
  function settingsButton(editable) {
    return ctrl(editable ? "⚙ Настройки партии" : "⚙ Посмотреть настройки", () => {
      settingsOpen = true;
      render();
    });
  }

  // Оверлей-меню настроек. editable=true только у ведущего.
  function settingsOverlay(editable) {
    const s = currentSettings();
    const overlay = el("div", "");
    overlay.id = "settings-overlay";
    overlay.className = "settings-overlay";
    const panel = el("div", "");
    panel.className = "settings-panel";
    overlay.appendChild(panel);

    const head = el("div", "");
    head.className = "settings-head";
    head.appendChild(el("h2", "Настройки партии" + (editable ? "" : " (только просмотр)")));
    head.appendChild(ctrl("✕", () => { settingsOpen = false; render(); }));
    panel.appendChild(head);

    // Чекбокс с подсказкой.
    const checkbox = (key, labelText, hint) => {
      const row = el("label", "");
      row.className = "play-setting";
      const cb = document.createElement("input");
      cb.type = "checkbox";
      cb.checked = !!s[key];
      cb.disabled = !editable;
      if (editable) cb.addEventListener("change", () => sendSettings({ [key]: cb.checked }));
      row.appendChild(cb);
      row.appendChild(document.createTextNode(" " + labelText));
      panel.appendChild(row);
      if (hint) {
        const h = el("div", hint);
        h.className = "play-setting-hint";
        panel.appendChild(h);
      }
    };

    // Числовое поле настройки.
    const number = (key, labelText, min, max, hint) => {
      const row = el("label", "");
      row.className = "play-setting";
      row.appendChild(document.createTextNode(labelText + " "));
      const inp = document.createElement("input");
      inp.type = "number";
      inp.min = String(min);
      inp.max = String(max);
      inp.value = String(s[key]);
      inp.disabled = !editable;
      inp.className = "settings-num";
      if (editable) {
        inp.addEventListener("change", () => {
          let v = parseInt(inp.value, 10);
          if (isNaN(v)) v = min;
          v = Math.max(min, Math.min(max, v));
          inp.value = String(v);
          sendSettings({ [key]: v });
        });
      }
      row.appendChild(inp);
      panel.appendChild(row);
      if (hint) {
        const h = el("div", hint);
        h.className = "play-setting-hint";
        panel.appendChild(h);
      }
    };

    // Режим открытия кнопок (select).
    panel.appendChild(el("h3", "Кнопки и ответ"));
    const modeRow = el("label", "");
    modeRow.className = "play-setting";
    modeRow.appendChild(document.createTextNode("Открытие кнопок: "));
    const sel = document.createElement("select");
    sel.disabled = !editable;
    for (const [val, txt] of [["manual", "Вручную ведущим"], ["after_last_slide", "Автоматически на последнем слайде"]]) {
      const o = document.createElement("option");
      o.value = val;
      o.textContent = txt;
      if (s.buzz_mode === val) o.selected = true;
      sel.appendChild(o);
    }
    if (editable) sel.addEventListener("change", () => sendSettings({ buzz_mode: sel.value }));
    modeRow.appendChild(sel);
    panel.appendChild(modeRow);

    checkbox("false_start", "Фальстарт: нажатие до открытия кнопок блокирует игрока",
      "Выключено — до открытия кнопка просто неактивна.");
    number("false_start_block_secs", "Длительность блока за фальстарт (сек):", 1, 60);
    number("buzz_time_secs", "Время на нажатие (сек):", 1, 120,
      "Используется для бегущей полосы времени.");
    number("answer_time_secs", "Время на ответ (сек):", 1, 300);

    panel.appendChild(el("h3", "Особые вопросы"));
    checkbox("cat_must_give", "Кот в мешке: обязательно отдавать другому игроку",
      "Выключено — выбравший может оставить кота себе.");
    checkbox("no_risk_double", "Вопрос без риска: удвоенная награда",
      "Выключено — обычная награда (номинал).");

    return overlay;
  }

  // ----------------------------- Аукцион -----------------------------

  function renderAuction() {
    const a = snap.auction;
    if (!a) return;
    E.main.appendChild(el("h3", "🔨 Аукцион"));
    E.main.appendChild(
      bigState(
        a.opening
          ? `Открытие торгов. Минимум — номинал ${a.price}. Ходит: ${nameOf(a.current_bidder)}`
          : `Текущая ставка: ${a.high_bid} (${nameOf(a.high_bidder)}). Ходит: ${nameOf(a.current_bidder)}`
      )
    );
    if (a.passed && a.passed.length) {
      E.main.appendChild(el("p", "Спасовали: " + a.passed.map(nameOf).join(", ")));
    }
    // Управление — только у игрока, чей сейчас ход.
    if (!you.host && a.current_bidder === you.id) {
      E.main.appendChild(bidForm(a));
    }
  }

  function bidForm(a) {
    const me = snap.players.find((p) => p.id === you.id);
    const myScore = me ? me.score : 0;
    const min = a.opening ? a.price : a.high_bid + 1;

    const box = el("div", "");

    const hint = el("div", `Ставка: от ${min} до ${myScore} очков`);
    hint.className = "final-form-hint";
    box.appendChild(hint);

    const wrap = el("div", "");
    wrap.className = "final-form";

    const inp = document.createElement("input");
    inp.type = "number";
    inp.min = String(min);
    inp.max = String(myScore);
    inp.value = String(Math.min(Math.max(min, 1), myScore || min));
    wrap.appendChild(inp);

    wrap.appendChild(
      ctrl("Поднять ставку", () => {
        const v = parseInt(inp.value, 10);
        if (!(v >= min && v <= myScore)) {
          setStatus(`Ставка должна быть от ${min} до ${myScore}.`);
          return;
        }
        send({ type: "bid", amount: v });
      }, myScore < min)
    );
    wrap.appendChild(ctrl(`Ва-банк (${myScore})`, () => send({ type: "all_in" }), myScore < 1));
    if (!a.opening) {
      wrap.appendChild(ctrl("Пас", () => send({ type: "pass" }), false, "danger"));
    }
    box.appendChild(wrap);
    return box;
  }

  // ----------------------------- Кот в мешке -----------------------------

  function renderCatGive() {
    const cur = snap.current;
    E.main.appendChild(el("h3", "🐱 Кот в мешке"));
    if (you.host) {
      if (cur) E.main.appendChild(questionBlock(cur));
      if (cur && cur.answer != null) E.main.appendChild(answerBox(cur.answer));
      E.main.appendChild(bigState(`${nameOf(snap.picker)} выбирает, кому отдать кота…`));
      return;
    }
    if (snap.picker === you.id) {
      E.main.appendChild(bigState("Вам достался кот в мешке! Выберите, кому передать вопрос:"));
      E.main.appendChild(giveButtons());
    } else {
      E.main.appendChild(bigState(`${nameOf(snap.picker)} выбирает, кому передать кота…`));
    }
  }

  function giveButtons() {
    const wrap = el("div", "");
    wrap.className = "final-themes";
    const canKeep = snap.settings && !snap.settings.cat_must_give;
    for (const p of snap.players) {
      if (p.id === you.id && !canKeep) continue; // себе нельзя при «обязан отдать»
      const label = p.id === you.id ? `${p.name} (себе)` : p.name;
      wrap.appendChild(ctrl(label, () => send({ type: "give", target: p.id })));
    }
    return wrap;
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

  // Блок текущего слайда (вопроса или ответа).
  function questionBlock(cur) {
    const box = el("div", "");
    box.className = "play-question";
    const showingAnswer = snap.phase === "show_answer";
    if (cur.kind && cur.kind !== "normal" && KIND[cur.kind]) {
      const badge = el("div", KIND[cur.kind]);
      badge.className = "play-kind-badge";
      box.appendChild(badge);
    }
    const title = showingAnswer ? "Ответ" : `Вопрос за ${cur.price}`;
    const slideTag = cur.slide_count > 1 ? `  · слайд ${cur.slide + 1}/${cur.slide_count}` : "";
    box.appendChild(el("h3", title + slideTag));
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
