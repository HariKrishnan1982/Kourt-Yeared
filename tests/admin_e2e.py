"""Admin API checks. Start the server first:
   KOURT_ADDR=127.0.0.1:3114 KOURT_DB=sqlite:///tmp/adm.db KOURT_ADMIN_USER=clerk \
   KOURT_ADMIN_PASSWORD=clerk-test-pass-123 cargo run
"""
import json, sqlite3, sys, time, urllib.request, urllib.error, http.cookiejar, os, tempfile
BASE = "http://127.0.0.1:3114"
DB = os.environ.get("KOURT_DB_FILE") or (os.path.join(tempfile.gettempdir(), "adm.db") if sys.platform == "win32" else "/tmp/adm.db")
AU, AP = "clerk", "clerk-test-pass-123"

class C:
    def __init__(s, headers=None):
        s.cj = http.cookiejar.CookieJar()
        s.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(s.cj))
        s.extra = headers or {}
    def call(s, method, path, body=None, marker=True, raw=False):
        h = {"Content-Type": "application/json", **s.extra}
        if marker: h["x-kourt"] = "1"
        d = json.dumps(body).encode() if body is not None else None
        r = urllib.request.Request(BASE + path, data=d, method=method, headers=h)
        try:
            with s.op.open(r) as x:
                b = x.read()
                return x.status, (b.decode() if raw else json.loads(b or b"{}"))
        except urllib.error.HTTPError as e:
            t = e.read()
            try: return e.code, json.loads(t)
            except Exception: return e.code, {"raw": t.decode()[:80]}

fails = 0
def ok(name, cond, extra=""):
    global fails
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else "  -> " + str(extra)))
    if not cond: fails += 1

def sql(q, *a):
    d = sqlite3.connect(DB); d.execute(q, a); d.commit(); d.close()
def user(n, pw="correct horse 9"):
    c = C(); s, b = c.call("POST", "/api/signup", {"username": n, "password": pw}); assert s == 200, (n, s, b); return c, b["id"]
def newcase(c, **kw):
    p = dict(title="Admin test case", charge="Testing the court", defendant_name="Dee", details="d", tier="snack", demand="a chai", preset="quick"); p.update(kw)
    s, b = c.call("POST", "/api/cases", p); assert s == 200, (s, b); return b["id"]

# ---------- gate: who can even reach the admin ----------
anon = C()
ok("admin API needs a login", anon.call("GET", "/api/admin/stats")[0] == 401)
ok("admin page is served on this computer", anon.call("GET", "/admin", raw=True)[0] == 200)
ok("admin sub-page is served too", anon.call("GET", "/admin/users/5", raw=True)[0] == 200)
proxied = C({"X-Forwarded-For": "203.0.113.9"})
ok("proxied requests can't see the admin page", proxied.call("GET", "/admin", raw=True)[0] == 404)
ok("proxied requests can't log in", proxied.call("POST", "/api/admin/login", {"username": AU, "password": AP})[0] == 404)
ok("proxied requests can't call admin API", proxied.call("GET", "/api/admin/stats")[0] == 404)
ok("login needs the x-kourt header", C().call("POST", "/api/admin/login", {"username": AU, "password": AP}, marker=False)[0] == 400)
ok("wrong admin password rejected", C().call("POST", "/api/admin/login", {"username": AU, "password": "nope-nope-nope"})[0] == 401)
ok("unknown admin name rejected", C().call("POST", "/api/admin/login", {"username": "root", "password": AP})[0] == 401)
adm = C()
s, b = adm.call("POST", "/api/admin/login", {"username": AU, "password": AP})
ok("admin logs in", s == 200 and b["username"] == AU, (s, b))
ok("admin /me works", adm.call("GET", "/api/admin/me")[1].get("username") == AU)
ok("admin cookie is not a user session", adm.call("GET", "/api/me")[0] == 401)

# ---------- a small world to manage ----------
kav, kid = user("kavya"); arun, aid = user("arun"); jur, jid = user("juror1"); lay, lid = user("lawyer1")
cid = newcase(kav, title="The samosa heist", charge="Ate the last samosa", demand="two samosas")
_, snap = kav.call("GET", "/api/cases/" + cid)
arun.call("POST", "/api/join/" + snap["invites"]["defendant"])
jur.call("POST", "/api/join/" + snap["invites"]["jury"])
kav.call("POST", "/api/cases/" + cid + "/messages", {"kind": "statement", "body": "He did it, I saw him."})
arun.call("POST", "/api/cases/" + cid + "/messages", {"kind": "statement", "body": "<b>Lies</b> and slander"})
sql("UPDATE cases SET deadline_ms=1 WHERE id=?", cid); time.sleep(1.6)
jur.call("POST", "/api/cases/" + cid + "/vote", {"vote": "guilty"})
time.sleep(0.3)
open_case = newcase(kav, title="Still open case")

# ---------- overview ----------
s, st = adm.call("GET", "/api/admin/stats")
ok("stats count users, cases, messages", st["users"] == 4 and st["cases"] == 2 and st["messages"] == 2 and st["open_cases"] == 1, st)
ok("stats show outcomes and phases", st["outcomes"].get("guilty") == 1 and st["phases"].get("closed") == 1)
ok("stats show unpaid treats and db size", st["unpaid"] == 1 and st["db_bytes"] > 0 and st["uptime_s"] >= 0)
ok("stats list recent users and cases", len(st["recent_users"]) == 4 and st["recent_cases"][0]["title"] == "Still open case")

# ---------- users ----------
s, ul = adm.call("GET", "/api/admin/users")
ok("user list shows everyone, newest first", ul["total"] == 4 and ul["users"][0]["username"] == "lawyer1", ul["users"][:1])
ok("user list shows case counts and debts", [u for u in ul["users"] if u["username"] == "arun"][0]["owes"] == 1)
ok("search by name", [u["username"] for u in adm.call("GET", "/api/admin/users?q=ARU")[1]["users"]] == ["arun"])
ok("search by id", [u["username"] for u in adm.call("GET", f"/api/admin/users?q={jid}")[1]["users"]] == ["juror1"])
ok("search treats % and _ literally", adm.call("GET", "/api/admin/users?q=%25")[1]["total"] == 0 and adm.call("GET", "/api/admin/users?q=_")[1]["total"] == 0)
ok("pagination reports page size", ul["per_page"] == 25 and adm.call("GET", "/api/admin/users?page=2")[1]["users"] == [])
s, ud = adm.call("GET", f"/api/admin/users/{aid}")
ok("user detail has cases, messages, debts", ud["username"] == "arun" and len(ud["cases"]) == 1 and ud["messages"] == 1 and ud["owes"] == 1 and ud["sessions"] == 1, ud)
ok("recent messages are shown as plain text data", ud["recent_messages"][0]["body"] == "<b>Lies</b> and slander")
ok("unknown user is 404", adm.call("GET", "/api/admin/users/99999")[0] == 404)

# ban / unban
ok("ban a user with a reason", adm.call("POST", f"/api/admin/users/{aid}/ban", {"reason": "Spamming the court"})[0] == 200)
ok("banned user's session is dead", arun.call("GET", "/api/me")[0] == 401)
s, b = C().call("POST", "/api/login", {"username": "arun", "password": "correct horse 9"})
ok("banned user can't log in and sees the reason", s == 403 and "Spamming the court" in b["error"], b)
ok("wrong password still looks the same for banned users", C().call("POST", "/api/login", {"username": "arun", "password": "wrong wrong 1"})[0] == 401)
ok("banned flag shows in the list", [u for u in adm.call("GET", "/api/admin/users?q=arun")[1]["users"]][0]["banned"] is True)
ok("unban restores access", adm.call("POST", f"/api/admin/users/{aid}/unban")[0] == 200 and C().call("POST", "/api/login", {"username": "arun", "password": "correct horse 9"})[0] == 200)

# sessions, password reset
arun2 = C(); arun2.call("POST", "/api/login", {"username": "arun", "password": "correct horse 9"})
s, b = adm.call("POST", f"/api/admin/users/{aid}/logout")
ok("log a user out everywhere", s == 200 and b["ended"] >= 1 and arun2.call("GET", "/api/me")[0] == 401)
s, b = adm.call("POST", f"/api/admin/users/{jid}/password")
tmp = b.get("password", "")
ok("reset password returns a temporary one", s == 200 and len(tmp) >= 12, b)
ok("old password no longer works", C().call("POST", "/api/login", {"username": "juror1", "password": "correct horse 9"})[0] == 401)
ok("temporary password works", C().call("POST", "/api/login", {"username": "juror1", "password": tmp})[0] == 200)
ok("juror's old session was ended", jur.call("GET", "/api/me")[0] == 401)

# rename / note
ok("rename rejects bad names", adm.call("POST", f"/api/admin/users/{lid}/rename", {"username": "a b"})[0] == 400)
ok("rename rejects reserved prefix", adm.call("POST", f"/api/admin/users/{lid}/rename", {"username": "deleted-5"})[0] == 400)
ok("rename rejects taken names", adm.call("POST", f"/api/admin/users/{lid}/rename", {"username": "KAVYA"})[0] == 409)
ok("rename works", adm.call("POST", f"/api/admin/users/{lid}/rename", {"username": "counsel_one"})[0] == 200 and adm.call("GET", f"/api/admin/users/{lid}")[1]["username"] == "counsel_one")
ok("admin note saves", adm.call("POST", f"/api/admin/users/{lid}/note", {"note": "Friend of Kavya.\nDoes not pay treats."})[0] == 200 and "Does not pay" in adm.call("GET", f"/api/admin/users/{lid}")[1]["admin_note"])
ok("oversized note rejected", adm.call("POST", f"/api/admin/users/{lid}/note", {"note": "x" * 501})[0] == 400)

# delete (anonymise)
extra, xid = user("tempuser")
ok("delete anonymises the account", adm.call("POST", f"/api/admin/users/{xid}/delete")[0] == 200)
d = adm.call("GET", f"/api/admin/users/{xid}")[1]
ok("deleted account is blanked and flagged", d["username"] == f"deleted-{xid}" and d["deleted"] and d["banned"], d)
ok("deleted account can't log in", C().call("POST", "/api/login", {"username": "tempuser", "password": "correct horse 9"})[0] == 401)
ok("deleted account's old session is dead", extra.call("GET", "/api/me")[0] == 401)
ok("deleted account can't be unbanned", adm.call("POST", f"/api/admin/users/{xid}/unban")[0] == 409)
ok("deleted account can't be reset or renamed", adm.call("POST", f"/api/admin/users/{xid}/password")[0] == 409 and adm.call("POST", f"/api/admin/users/{xid}/rename", {"username": "back_again"})[0] == 409)
ok("the name is free to register again", C().call("POST", "/api/signup", {"username": "tempuser", "password": "correct horse 9"})[0] == 200)
ok("stats ignore deleted accounts", adm.call("GET", "/api/admin/stats")[1]["users"] == 5)

# ---------- cases ----------
s, cl = adm.call("GET", "/api/admin/cases")
ok("case list shows both cases", cl["total"] == 2 and cl["cases"][0]["title"] == "Still open case", cl)
ok("filter by phase", [c["title"] for c in adm.call("GET", "/api/admin/cases?phase=closed")[1]["cases"]] == ["The samosa heist"])
ok("search by title, charge, plaintiff and id", all(adm.call("GET", f"/api/admin/cases?q={q}")[1]["total"] == n for q, n in [("samosa", 1), ("Ate+the", 1), ("kavya", 2), (cid, 1), ("zzz", 0)]))
s, cd = adm.call("GET", "/api/admin/cases/" + cid)
ok("case detail shows people, transcript, votes", len(cd["people"]) == 3 and len(cd["messages"]) == 2 and cd["votes"] == [{"user": "juror1", "vote": "guilty"}], cd["votes"])
ok("case detail shows verdict and sentence", cd["outcome"] == "guilty" and cd["sentence"] == "two samosas" and cd["sentence_paid"] is False and cd["guilty"] == 1)
ok("case detail shows who claimed each invite, never the tokens", any(i["claimed_by"] == "arun" for i in cd["invites"]) and "token" not in json.dumps(cd["invites"]))
ok("unknown case is 404", adm.call("GET", "/api/admin/cases/nope")[0] == 404)

ok("mark a sentence paid", adm.call("POST", f"/api/admin/cases/{cid}/paid", {"paid": True})[0] == 200 and adm.call("GET", "/api/admin/cases/" + cid)[1]["sentence_paid"] is True)
ok("and unpaid again", adm.call("POST", f"/api/admin/cases/{cid}/paid", {"paid": False})[0] == 200)
ok("can't mark a case with no sentence", adm.call("POST", f"/api/admin/cases/{open_case}/paid", {"paid": True})[0] == 409)
ok("can't close a closed case", adm.call("POST", f"/api/admin/cases/{cid}/close", {"outcome": "mistrial"})[0] == 409)

before = adm.call("GET", "/api/admin/cases/" + open_case)[1]["deadline_ms"]
ok("extend an open case", adm.call("POST", f"/api/admin/cases/{open_case}/extend", {"minutes": 30})[0] == 200 and adm.call("GET", "/api/admin/cases/" + open_case)[1]["deadline_ms"] == before + 30 * 60000)
ok("extend limits enforced", adm.call("POST", f"/api/admin/cases/{open_case}/extend", {"minutes": 0})[0] == 400 and adm.call("POST", f"/api/admin/cases/{open_case}/extend", {"minutes": 99999})[0] == 400)
ok("can't extend a closed case", adm.call("POST", f"/api/admin/cases/{cid}/extend", {"minutes": 5})[0] == 409)
ok("edit case text", adm.call("POST", f"/api/admin/cases/{open_case}/edit", {"title": "Fixed typo title", "charge": "New charge", "demand": "a samosa", "details": "line1\nline2"})[0] == 200)
_, ks = kav.call("GET", "/api/cases/" + open_case)
ok("edit shows up for the players", ks["title"] == "Fixed typo title" and ks["demand"] == "a samosa")
ok("edit validates input", adm.call("POST", f"/api/admin/cases/{open_case}/edit", {"title": "x", "charge": "ok ok", "demand": "ok"})[0] == 400)
ok("bad outcome rejected", adm.call("POST", f"/api/admin/cases/{open_case}/close", {"outcome": "executed"})[0] == 400)
ok("force-close a case", adm.call("POST", f"/api/admin/cases/{open_case}/close", {"outcome": "dismissed"})[0] == 200)
_, ks = kav.call("GET", "/api/cases/" + open_case)
ok("forced close applies the normal rules (dismissed: plaintiff owes)", ks["phase"] == "closed" and ks["outcome"] == "dismissed" and ks["sentence"]["debtor"] == "kavya", ks.get("sentence"))

# moderation
mid = cd["messages"][1]["id"]
ok("remove a message", adm.call("POST", f"/api/admin/cases/{cid}/messages/{mid}/remove")[0] == 200)
_, ks = kav.call("GET", "/api/cases/" + cid)
ok("players see it replaced", [m for m in ks["messages"] if m["id"] == mid][0]["removed"] is True and "Lies" not in json.dumps(ks["messages"]))
ok("removing a missing message is 404", adm.call("POST", f"/api/admin/cases/{cid}/messages/999999/remove")[0] == 404)
ok("a message can't be removed through another case", adm.call("POST", f"/api/admin/cases/{open_case}/messages/{mid}/remove")[0] == 404)

# delete case
ok("delete a whole case", adm.call("POST", f"/api/admin/cases/{open_case}/delete")[0] == 200)
ok("it is gone for admin and players", adm.call("GET", "/api/admin/cases/" + open_case)[0] == 404 and kav.call("GET", "/api/cases/" + open_case)[0] == 404)
ok("deleting a case with votes and messages works", adm.call("POST", f"/api/admin/cases/{cid}/delete")[0] == 200)
ok("owed counts update after deletion", adm.call("GET", "/api/admin/stats")[1]["unpaid"] == 0)

# ---------- settings ----------
s, se = adm.call("GET", "/api/admin/settings")
ok("settings default", se == {"signups_open": True, "announcement": "", "max_open_cases": 10}, se)
ok("public site info starts empty", C().call("GET", "/api/site")[1] == {"announcement": "", "signups_open": True})
ok("save settings", adm.call("POST", "/api/admin/settings", {"signups_open": False, "announcement": "Court is closed Friday.", "max_open_cases": 1})[0] == 200)
ok("public site shows the announcement", C().call("GET", "/api/site")[1] == {"announcement": "Court is closed Friday.", "signups_open": False})
s, b = C().call("POST", "/api/signup", {"username": "latecomer", "password": "correct horse 9"})
ok("signups can be paused", s == 403 and "paused" in b["error"], b)
ok("existing users can still log in", C().call("POST", "/api/login", {"username": "kavya", "password": "correct horse 9"})[0] == 200)
k2 = C(); k2.call("POST", "/api/login", {"username": "kavya", "password": "correct horse 9"})
newcase(k2, title="First open case")
s, b = k2.call("POST", "/api/cases", dict(title="Second case", charge="Too many", defendant_name="X", tier="snack", preset="quick"))
ok("open-case limit comes from settings", s == 429 and "1 open" in b["error"], b)
ok("settings validate", adm.call("POST", "/api/admin/settings", {"signups_open": True, "announcement": "x" * 201, "max_open_cases": 5})[0] == 400 and adm.call("POST", "/api/admin/settings", {"signups_open": True, "announcement": "", "max_open_cases": 0})[0] == 400)
ok("restore settings", adm.call("POST", "/api/admin/settings", {"signups_open": True, "announcement": "", "max_open_cases": 10})[0] == 200 and C().call("POST", "/api/signup", {"username": "latecomer", "password": "correct horse 9"})[0] == 200)

# ---------- audit log ----------
s, lg = adm.call("GET", "/api/admin/log")
acts = [e["action"] for e in lg["log"]]
ok("audit log records admin actions, newest first", lg["log"][0]["action"] == "change_settings" and all(a in acts for a in ["login", "ban_user", "unban_user", "reset_password", "rename_user", "delete_user", "close_case", "extend_case", "remove_message", "delete_case", "mark_paid"]), acts[:12])
ok("the log never stores temporary passwords", tmp not in json.dumps(lg))

# ---------- admin account ----------
other = C(); other.call("POST", "/api/admin/login", {"username": AU, "password": AP})
ok("wrong current password rejected without ending the session", adm.call("POST", "/api/admin/password", {"current": "bad-bad-bad-1", "new": "a-brand-new-password-1"})[0] == 400 and adm.call("GET", "/api/admin/me")[0] == 200)
ok("short new password rejected", adm.call("POST", "/api/admin/password", {"current": AP, "new": "short"})[0] == 400)
ok("change admin password", adm.call("POST", "/api/admin/password", {"current": AP, "new": "a-brand-new-password-1"})[0] == 200)
ok("this session stays signed in", adm.call("GET", "/api/admin/me")[0] == 200)
ok("other admin sessions were ended", other.call("GET", "/api/admin/me")[0] == 401)
ok("old password is dead, new works", C().call("POST", "/api/admin/login", {"username": AU, "password": AP})[0] == 401 and C().call("POST", "/api/admin/login", {"username": AU, "password": "a-brand-new-password-1"})[0] == 200)
ok("state-changing admin calls need x-kourt", adm.call("POST", f"/api/admin/users/{kid}/logout", marker=False)[0] == 400)
ok("admin logout ends the session", adm.call("POST", "/api/admin/logout")[0] == 200 and adm.call("GET", "/api/admin/me")[0] == 401)
ok("normal users can't use admin API", k2.call("GET", "/api/admin/stats")[0] == 401)

# ---------- brute force ----------
codes = [C().call("POST", "/api/admin/login", {"username": "guesser", "password": f"guess-number-{i}-xx"})[0] for i in range(7)]
ok("admin login locks after repeated failures", codes[:5] == [401] * 5 and codes[5] == 429, codes)
print("\nFAILURES:", fails); sys.exit(1 if fails else 0)
