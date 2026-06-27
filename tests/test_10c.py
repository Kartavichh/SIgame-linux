import socket, json, time, sys, zipfile, io, subprocess, os, signal

PORT = 7800
PACK = "/tmp/claude-1000/-home-kartavich-claude/4674ae32-2a0e-4e65-960b-3830b7f77b55/scratchpad/slides.sgpack"
SERVER = "/home/kartavich/claude/sigame-rs/target/debug/sigame-server"

# --- Собрать слайдовый пак ---
pack = {
    "name": "Слайды",
    "rounds": [{
        "name": "Р1",
        "themes": [{
            "name": "Тема",
            "questions": [
                {
                    "price": 100, "kind": "normal",
                    "question_slides": [
                        {"items": [{"type": "text", "value": "слайд1"}]},
                        {"items": [{"type": "text", "value": "слайд2"}]},
                    ],
                    "answer_slides": [
                        {"items": [{"type": "text", "value": "это ответ"}]},
                    ],
                },
                {
                    "price": 200, "kind": "normal",
                    "question_slides": [{"items": [{"type": "text", "value": "q2"}]}],
                    "answer_slides": [],
                },
            ],
        }],
    }],
}
with zipfile.ZipFile(PACK, "w") as z:
    z.writestr("pack.json", json.dumps(pack, ensure_ascii=False))

srv = subprocess.Popen([SERVER, PACK, "--port", str(PORT)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.7)

failed = False
def expect(cond, msg):
    global failed
    print(("  OK  " if cond else "  !!! FAIL ") + msg)
    if not cond:
        failed = True

class Cli:
    def __init__(self, name, host=False):
        self.name = name
        self.s = socket.create_connection(("127.0.0.1", PORT))
        self.s.settimeout(0.3)
        self.buf = b""
        self.state = None
        self.welcome = None
        self.send({"type": "hello", "name": name, "host": host})
        self.pump(0.3)
    def send(self, obj):
        self.s.sendall((json.dumps(obj) + "\n").encode())
    def pump(self, secs=0.3):
        end = time.time() + secs
        while time.time() < end:
            try:
                data = self.s.recv(65536)
                if not data: break
                self.buf += data
            except socket.timeout:
                pass
            while b"\n" in self.buf:
                line, self.buf = self.buf.split(b"\n", 1)
                if not line.strip(): continue
                m = json.loads(line.decode())
                if m["type"] == "welcome": self.welcome = m
                elif m["type"] == "state": self.state = m
                elif m["type"] == "error": print(f"  [{self.name}] ERROR: {m['message']}")

def pump_all(cs, secs=0.4):
    for c in cs: c.pump(secs)

try:
    host = Cli("Ведущий", host=True)
    p1 = Cli("Аня")
    p2 = Cli("Боб")
    cs = [host, p1, p2]
    pump_all(cs, 0.4)
    ids = {c.name: c.welcome["id"] for c in (p1, p2)}

    print("\n# Полные настройки")
    host.send({"type": "settings", "cat_must_give": True, "no_risk_double": False,
               "buzz_mode": "manual", "false_start": True, "false_start_block_secs": 6,
               "buzz_time_secs": 7, "answer_time_secs": 25})
    pump_all(cs, 0.4)
    st = host.state["settings"]
    expect(st["buzz_mode"] == "manual", "buzz_mode=manual в снимке")
    expect(st["false_start"] is True, "false_start=True")
    expect(st["false_start_block_secs"] == 6, "block_secs=6")
    expect(st["buzz_time_secs"] == 7 and st["answer_time_secs"] == 25, "таймеры в снимке")

    host.send({"type": "start"})
    pump_all(cs, 0.4)
    picker = host.state["picker"]

    print("\n# Выбор вопроса со слайдами (manual)")
    p1.send({"type": "pick", "theme": 0, "question": 0})
    pump_all(cs, 0.4)
    cur = host.state["current"]
    expect(host.state["phase"] == "question", "фаза question")
    expect(cur["slide"] == 0 and cur["slide_count"] == 2, "слайд 0 из 2")
    expect(cur["buzzing_open"] is False, "кнопки закрыты (manual)")
    expect(cur["content"][0]["value"] == "слайд1", "виден слайд1")

    print("\n# Фальстарт (нажатие до открытия кнопок)")
    p2.send({"type": "buzz"})
    pump_all(cs, 0.3)
    cur = host.state["current"]
    expect(host.state["phase"] == "question", "после фальстарта фаза прежняя")
    expect(ids["Боб"] in cur["false_started"], "Боб в false_started")
    # Повторное нажатие, пока держится блок — отклоняется, блок сохраняется.
    p2.send({"type": "buzz"})
    pump_all(cs, 0.3)
    expect(host.state["phase"] == "question", "повторный buzz не сменил фазу")
    expect(ids["Боб"] in host.state["current"]["false_started"], "Боб ещё заблокирован")

    print("\n# Листание слайдов и открытие кнопок")
    host.send({"type": "next_slide"})
    pump_all(cs, 0.3)
    cur = host.state["current"]
    expect(cur["slide"] == 1 and cur["content"][0]["value"] == "слайд2", "перелистнули на слайд2")
    expect(cur["buzzing_open"] is False, "в manual листание не открывает кнопки")
    host.send({"type": "open_buzz"})
    pump_all(cs, 0.3)
    expect(host.state["current"]["buzzing_open"] is True, "кнопки открыты")

    print("\n# Снятие блока по таймеру сервера")
    time.sleep(7.0)  # > block_secs=6
    pump_all(cs, 0.4)
    expect(ids["Боб"] not in host.state["current"]["false_started"], "блок снят таймером")

    print("\n# Ответ и показ слайдов ответа")
    p1.send({"type": "buzz"})
    pump_all(cs, 0.4)
    expect(host.state["phase"] == "answering" and host.state["current"]["buzzed"] == ids["Аня"], "Аня отвечает")
    host.send({"type": "judge", "correct": True})
    pump_all(cs, 0.4)
    cur = host.state["current"]
    expect(host.state["phase"] == "show_answer", "фаза show_answer")
    expect(cur["slide_count"] == 1 and cur["content"][0]["value"] == "это ответ", "виден слайд ответа")
    host.send({"type": "close_question"})
    pump_all(cs, 0.4)
    expect(host.state["phase"] == "picking", "после закрытия — выбор")
    sc = {p["name"]: p["score"] for p in host.state["players"]}
    expect(sc["Аня"] == 100, f"Аня +100 (={sc['Аня']})")
    expect(host.state["picker"] == ids["Аня"], "Аня стала выбирающей")

    print("\n# Вопрос без слайдов ответа -> закрывается сразу")
    p1.send({"type": "pick", "theme": 0, "question": 1})
    pump_all(cs, 0.4)
    host.send({"type": "open_buzz"})
    pump_all(cs, 0.3)
    p1.send({"type": "buzz"})
    pump_all(cs, 0.3)
    host.send({"type": "judge", "correct": True})
    pump_all(cs, 0.4)
    expect(host.state["phase"] in ("picking", "game_over"), "без слайдов ответа -> сразу к выбору/концу")

    print("\nИТОГ:", "ВСЁ ПРОШЛО" if not failed else "ЕСТЬ ОШИБКИ")
finally:
    srv.send_signal(signal.SIGKILL)

sys.exit(1 if failed else 0)
