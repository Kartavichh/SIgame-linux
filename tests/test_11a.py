"""Сквозной тест Этапа 11a: аватарки игроков и ведущего проходят через сервер.

Покрывает:
  * hello с полем avatar — сервер сохраняет и отдаёт его в снимке;
  * игрок без аватарки получает avatar == null (клиент рисует заглушку);
  * снимок содержит блок host (имя + аватарка + online);
  * команда set_avatar меняет аватарку «на лету» и видна всем;
  * set_avatar с null убирает аватарку.
"""
import socket, json, time, subprocess

PORT = 7802
DEMO = "/home/kartavich/claude/sigame-rs/demo/demo.sgpack"
SERVER = "/home/kartavich/claude/sigame-rs/target/debug/sigame-server"

failed = False
def expect(cond, msg):
    global failed
    print(("  OK  " if cond else "  !!! FAIL ") + msg)
    if not cond:
        failed = True

class Cli:
    def __init__(self, name, host=False, avatar=None):
        self.name = name
        self.s = socket.create_connection(("127.0.0.1", PORT))
        self.s.settimeout(0.3)
        self.buf = b""
        self.state = None
        self.send({"type": "hello", "name": name, "host": host, "avatar": avatar})
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
                if m["type"] == "state": self.state = m
    def player(self, name):
        for p in self.state["players"]:
            if p["name"] == name:
                return p
        return None

AV1 = "data:image/png;base64,AAAA"   # «аватарка» игрока
AV2 = "data:image/png;base64,BBBB"   # ведущего
AV3 = "data:image/png;base64,CCCC"   # новая аватарка игрока (смена на лету)

srv = subprocess.Popen([SERVER, DEMO, "--port", str(PORT)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(0.7)

try:
    host = Cli("Ведущий", host=True, avatar=AV2)
    p1 = Cli("Аня", avatar=AV1)
    p2 = Cli("Боря")  # без аватарки
    for c in (host, p1, p2): c.pump(0.3)

    print("== Аватарки в снимке ==")
    expect(host.player("Аня") and host.player("Аня")["avatar"] == AV1,
           "аватарка Ани видна ведущему")
    expect(p2.player("Аня") and p2.player("Аня")["avatar"] == AV1,
           "аватарка Ани видна другому игроку")
    expect(host.player("Боря") and host.player("Боря")["avatar"] is None,
           "у Бори аватарка null (будет заглушка)")

    print("== Блок ведущего ==")
    h = p1.state.get("host")
    expect(h is not None, "снимок содержит блок host")
    expect(h and h["name"] == "Ведущий", "имя ведущего в снимке")
    expect(h and h["avatar"] == AV2, "аватарка ведущего в снимке")
    expect(h and h["online"] is True, "ведущий online")

    print("== Смена аватарки на лету ==")
    p1.send({"type": "set_avatar", "avatar": AV3})
    for c in (host, p1, p2): c.pump(0.3)
    expect(p2.player("Аня") and p2.player("Аня")["avatar"] == AV3,
           "новая аватарка Ани разослана всем")

    print("== Сброс аватарки (null) ==")
    p1.send({"type": "set_avatar", "avatar": None})
    for c in (host, p1, p2): c.pump(0.3)
    expect(host.player("Аня") and host.player("Аня")["avatar"] is None,
           "аватарка Ани убрана (снова заглушка)")

finally:
    srv.send_signal(2)
    time.sleep(0.2)
    srv.kill()

print("\nИТОГ:", "ЕСТЬ ОШИБКИ" if failed else "ВСЁ ОК")
exit(1 if failed else 0)
