import json, sqlite3, time, urllib.request, urllib.error, http.cookiejar, sys
BASE="http://127.0.0.1:3111"; DB="/tmp/e2e.db"
class C:
    def __init__(s):
        s.cj=http.cookiejar.CookieJar(); s.op=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(s.cj))
    def call(s,method,path,body=None,marker=True):
        h={"Content-Type":"application/json"}
        if marker: h["x-kourt"]="1"
        d=json.dumps(body).encode() if body is not None else None
        r=urllib.request.Request(BASE+path,data=d,method=method,headers=h)
        try:
            with s.op.open(r) as x: return x.status,json.loads(x.read() or b"{}")
        except urllib.error.HTTPError as e:
            t=e.read()
            try: return e.code,json.loads(t)
            except: return e.code,{"raw":t.decode()[:80]}
fails=0
def ok(name,cond,extra=""):
    global fails
    print(("PASS " if cond else "FAIL ")+name+(" "+str(extra) if not cond else ""))
    if not cond: fails+=1
def user(n):
    c=C(); s,b=c.call("POST","/api/signup",{"username":n,"password":"correct horse 9"}); assert s==200,(s,b); return c
def setdb(sql,*a):
    d=sqlite3.connect(DB); d.execute(sql,a); d.commit(); d.close()
def wait_phase(c,cid,phase,t=6):
    for _ in range(t*5):
        s,b=c.call("GET","/api/cases/"+cid)
        if b.get("phase")==phase: return b
        time.sleep(0.2)
    return b
def newcase(c,**kw):
    p=dict(title="The samosa heist",charge="Stole the last samosa",defendant_name="Arun",details="Seen near the canteen.",tier="snack",demand="two samosas",preset="quick"); p.update(kw)
    s,b=c.call("POST","/api/cases",p); assert s==200,(s,b); return b["id"]

# --- auth
ok("weak password rejected", C().call("POST","/api/signup",{"username":"bob_x","password":"short"})[0]==400)
ok("bad username rejected", C().call("POST","/api/signup",{"username":"a b","password":"correct horse 9"})[0]==400)
kav=user("kavya"); 
ok("duplicate username (case-insensitive) rejected", C().call("POST","/api/signup",{"username":"KAVYA","password":"correct horse 9"})[0]==409)
ok("me works", kav.call("GET","/api/me")[1].get("username")=="kavya")
ok("anonymous /me is 401", C().call("GET","/api/me")[0]==401)
ok("POST without x-kourt header is refused", C().call("POST","/api/login",{"username":"kavya","password":"x"},marker=False)[0]==400)
ok("wrong password 401", C().call("POST","/api/login",{"username":"kavya","password":"nope nope nope"})[0]==401)
ok("unknown user 401", C().call("POST","/api/login",{"username":"ghost","password":"nope nope nope"})[0]==401)
c2=C(); ok("login works", c2.call("POST","/api/login",{"username":"KaVyA","password":"correct horse 9"})[0]==200)

# --- case creation validation
ok("bad tier rejected", kav.call("POST","/api/cases",dict(title="abc",charge="abc",defendant_name="x",tier="car",preset="quick"))[0]==400)
ok("too-short custom length rejected", kav.call("POST","/api/cases",dict(title="abc",charge="abc",defendant_name="x",tier="snack",preset="custom",minutes=2))[0]==400)
ok("empty title rejected", kav.call("POST","/api/cases",dict(title=" ",charge="abc",defendant_name="x",tier="snack",preset="quick"))[0]==400)

# --- full trial
cid=newcase(kav)
s,snap=kav.call("GET","/api/cases/"+cid)
ok("new case is in summons", snap["phase"]=="summons" and snap["me"]["role"]=="plaintiff")
ok("plaintiff sees defendant/lawyer/jury links only", sorted(snap["invites"])==["defendant","jury","plaintiff_lawyer"], snap["invites"])
stranger=user("stranger")
ok("non-participant cannot read case", stranger.call("GET","/api/cases/"+cid)[0]==404)
arun=user("arun"); lawA=user("lawyer_a"); lawB=user("lawyer_b"); j=[user(f"juror{i}") for i in range(3)]
s,peek=C().call("GET","/api/join/"+snap["invites"]["defendant"])
ok("anonymous can peek at summons", s==200 and peek["role"]=="defendant" and peek["plaintiff"]=="kavya")
ok("garbage token 404", C().call("GET","/api/join/nonsense")[0]==404)
ok("plaintiff using own defendant link is harmless", kav.call("POST","/api/join/"+snap["invites"]["defendant"])[0]==200 and kav.call("GET","/api/cases/"+cid)[1]["me"]["role"]=="plaintiff")
ok("anonymous cannot join", C().call("POST","/api/join/"+snap["invites"]["defendant"])[0]==401)
s,_=arun.call("POST","/api/join/"+snap["invites"]["defendant"]); ok("defendant joins by link", s==200)
ok("second person cannot reuse defendant link", stranger.call("POST","/api/join/"+snap["invites"]["defendant"])[0]==409)
s,a=arun.call("GET","/api/cases/"+cid); ok("joining opens arguments", a["phase"]=="arguments" and a["me"]["role"]=="defendant")
ok("defendant sees defense-lawyer link, not plaintiff's", "defendant_lawyer" in a["invites"] and "plaintiff_lawyer" not in a["invites"])
_,snap=kav.call("GET","/api/cases/"+cid)
ok("plaintiff's defendant link now hidden", "defendant" not in snap["invites"])
ok("plaintiff lawyer joins", lawA.call("POST","/api/join/"+snap["invites"]["plaintiff_lawyer"])[0]==200)
ok("defense lawyer joins", lawB.call("POST","/api/join/"+a["invites"]["defendant_lawyer"])[0]==200)
for x in j: x.call("POST","/api/join/"+snap["invites"]["jury"])
ok("juror cannot argue", j[0].call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"hi"})[0]==403)
ok("stranger cannot argue", stranger.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"hi"})[0]==404)
ok("plaintiff statement", kav.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"<script>alert(1)</script> He took it."})[0]==200)
ok("plaintiff lawyer statement", lawA.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"The samosa was clearly marked."})[0]==200)
ok("defense lawyer statement", lawB.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"No name was on it."})[0]==200)
ok("empty statement rejected", arun.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"   "})[0]==400)
ok("oversize statement rejected", arun.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"x"*501})[0]==400)
ok("unknown kind rejected", arun.call("POST","/api/cases/"+cid+"/messages",{"kind":"shout","body":"x"})[0]==400)
codes=[arun.call("POST","/api/cases/"+cid+"/messages",{"kind":"objection","body":"Hearsay"})[0] for _ in range(4)]
ok("only 3 objections per side", codes==[200,200,200,409], codes)
ok("voting closed during arguments", j[0].call("POST","/api/cases/"+cid+"/vote",{"vote":"guilty"})[0]==409)
_,snap=j[0].call("GET","/api/cases/"+cid); ok("transcript visible to jury with lawyer flag", len(snap["messages"])>=4 and any(m["lawyer"] for m in snap["messages"]))
ok("juror cannot see invite for defendant", list(snap["invites"])==["jury"])
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid)
b=wait_phase(kav,cid,"voting"); ok("clock moves arguments -> voting", b["phase"]=="voting")
ok("cannot post after floor closes", kav.call("POST","/api/cases/"+cid+"/messages",{"kind":"statement","body":"late"})[0]==409)
ok("late juror still seated during voting", user("late_juror").call("POST","/api/join/"+snap["invites"]["jury"])[0]==200)
ok("party cannot vote", kav.call("POST","/api/cases/"+cid+"/vote",{"vote":"guilty"})[0]==403)
ok("bad vote rejected", j[0].call("POST","/api/cases/"+cid+"/vote",{"vote":"maybe"})[0]==400)
j[0].call("POST","/api/cases/"+cid+"/vote",{"vote":"guilty"})
j[1].call("POST","/api/cases/"+cid+"/vote",{"vote":"not_guilty"})
j[1].call("POST","/api/cases/"+cid+"/vote",{"vote":"guilty"})  # changed mind
_,v=j[0].call("GET","/api/cases/"+cid); ok("votes hidden until verdict", v["phase"]=="voting" and v["tally"] is None and v["jury_voted"]==2 and v["my_vote"]=="guilty")
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid)
v=wait_phase(kav,cid,"closed")
ok("verdict is guilty 2-0", v["outcome"]=="guilty" and v["tally"]=={"guilty":2,"not_guilty":0}, v.get("tally"))
ok("sentence is the demand, owed by defendant", v["sentence"]["text"]=="two samosas" and v["sentence"]["debtor"]=="arun" and v["sentence"]["i_am_owed"] is True)
ok("defendant cannot mark paid", arun.call("POST","/api/cases/"+cid+"/paid")[0]==403)
ok("record shows debt", arun.call("GET","/api/me")[1]["owes"]==1 and kav.call("GET","/api/me")[1]["owed"]==1)
ok("plaintiff marks paid", kav.call("POST","/api/cases/"+cid+"/paid")[0]==200 and kav.call("GET","/api/cases/"+cid)[1]["sentence"]["paid"] is True)
ok("debt cleared on record", arun.call("GET","/api/me")[1]["owes"]==0)
ok("no one can vote after close", j[2].call("POST","/api/cases/"+cid+"/vote",{"vote":"guilty"})[0]==409)
ok("no joining closed case", user("toolate").call("POST","/api/join/"+snap["invites"]["jury"])[0]==409)

# --- early verdict when everyone has voted
cid2=newcase(kav,title="Early verdict case"); _,s2=kav.call("GET","/api/cases/"+cid2)
arun.call("POST","/api/join/"+s2["invites"]["defendant"])
kav.call("POST","/api/cases/"+cid2+"/messages",{"kind":"statement","body":"Plaintiff here."}); arun.call("POST","/api/cases/"+cid2+"/messages",{"kind":"statement","body":"Defendant here."})
j[0].call("POST","/api/join/"+s2["invites"]["jury"]); j[1].call("POST","/api/join/"+s2["invites"]["jury"])
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid2); wait_phase(kav,cid2,"voting")
j[0].call("POST","/api/cases/"+cid2+"/vote",{"vote":"not_guilty"}); j[1].call("POST","/api/cases/"+cid2+"/vote",{"vote":"not_guilty"})
_,v=kav.call("GET","/api/cases/"+cid2); ok("everyone voted -> immediate verdict, plaintiff owes false-alarm fee", v["phase"]=="closed" and v["outcome"]=="not_guilty" and v["sentence"]["debtor"]=="kavya", v.get("outcome"))

# --- hung jury
cid3=newcase(kav,title="Hung jury case"); _,s3=kav.call("GET","/api/cases/"+cid3)
arun.call("POST","/api/join/"+s3["invites"]["defendant"])
kav.call("POST","/api/cases/"+cid3+"/messages",{"kind":"statement","body":"P"}); arun.call("POST","/api/cases/"+cid3+"/messages",{"kind":"statement","body":"D"})
j[0].call("POST","/api/join/"+s3["invites"]["jury"]); j[1].call("POST","/api/join/"+s3["invites"]["jury"])
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid3); wait_phase(kav,cid3,"voting")
j[0].call("POST","/api/cases/"+cid3+"/vote",{"vote":"guilty"}); j[1].call("POST","/api/cases/"+cid3+"/vote",{"vote":"not_guilty"})
_,v=kav.call("GET","/api/cases/"+cid3); ok("tied jury -> mistrial, nobody owes", v["outcome"]=="mistrial" and v["sentence"] is None, v.get("outcome"))

# --- default judgment: defendant never shows
cid4=newcase(kav,title="No-show case"); _,s4=kav.call("GET","/api/cases/"+cid4); setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid4)
v=wait_phase(kav,cid4,"closed"); ok("no-show -> default judgment with treat owed", v["outcome"]=="default_judgment" and v["sentence"]["text"]=="two samosas" and v["sentence"]["i_am_owed"] is True, v.get("outcome"))
ok("no-show link now dead", user("late2").call("POST","/api/join/"+s4["invites"]["defendant"])[0]==409)

# --- defendant silent in arguments -> default judgment ; plaintiff silent -> dismissed
cid5=newcase(kav,title="Silent defendant"); _,s5=kav.call("GET","/api/cases/"+cid5)
arun.call("POST","/api/join/"+s5["invites"]["defendant"]); kav.call("POST","/api/cases/"+cid5+"/messages",{"kind":"statement","body":"He did it."})
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid5); v=wait_phase(kav,cid5,"closed")
ok("silent defendant -> default judgment", v["outcome"]=="default_judgment")
cid6=newcase(kav,title="Silent plaintiff"); _,s6=kav.call("GET","/api/cases/"+cid6)
arun.call("POST","/api/join/"+s6["invites"]["defendant"]); arun.call("POST","/api/cases/"+cid6+"/messages",{"kind":"statement","body":"I am innocent."})
setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid6); v=wait_phase(kav,cid6,"closed")
ok("silent plaintiff -> dismissed, plaintiff owes", v["outcome"]=="dismissed" and v["sentence"]["debtor"]=="kavya")
cid7=newcase(kav,title="Both silent"); _,s7=kav.call("GET","/api/cases/"+cid7)
arun.call("POST","/api/join/"+s7["invites"]["defendant"]); setdb("UPDATE cases SET deadline_ms=1 WHERE id=?",cid7); v=wait_phase(kav,cid7,"closed")
ok("both silent -> abandoned, nobody owes", v["outcome"]=="abandoned" and v["sentence"] is None)

# --- withdraw
cid8=newcase(kav,title="Withdrawn"); ok("only plaintiff can withdraw", arun.call("POST","/api/cases/"+cid8+"/withdraw")[0] in (403,404))
ok("plaintiff withdraws", kav.call("POST","/api/cases/"+cid8+"/withdraw")[0]==200 and kav.call("GET","/api/cases/"+cid8)[1]["outcome"]=="withdrawn")
cid9=newcase(kav,title="Too late to withdraw"); _,s9=kav.call("GET","/api/cases/"+cid9); arun.call("POST","/api/join/"+s9["invites"]["defendant"])
ok("cannot withdraw once defendant answered", kav.call("POST","/api/cases/"+cid9+"/withdraw")[0]==409)

# --- concurrency: two people race for one defendant link
cid10=newcase(kav,title="Race"); _,s10=kav.call("GET","/api/cases/"+cid10)
import threading
res=[]; racers=[user(f"racer{i}") for i in range(6)]
def go(r): res.append(r.call("POST","/api/join/"+s10["invites"]["defendant"])[0])
th=[threading.Thread(target=go,args=(r,)) for r in racers]; [t.start() for t in th]; [t.join() for t in th]
ok("exactly one racer wins the defendant seat", sorted(res)==[200]+[409]*5, res)

# --- listing & static & headers
_,lst=kav.call("GET","/api/cases"); ok("case list works", len(lst["cases"])>=8)
r=urllib.request.urlopen(BASE+"/case/abc"); ok("page routes fall back to index.html", r.status==200 and "html" in r.headers["content-type"])
ok("CSP + referrer policy set", "default-src 'self'" in r.headers["content-security-policy"] and r.headers["referrer-policy"]=="no-referrer")
try: urllib.request.urlopen(BASE+"/api/nope"); ok("unknown api route 404",False)
except urllib.error.HTTPError as e: ok("unknown api route 404", e.code==404)
try: urllib.request.urlopen(BASE+"/../etc/passwd"); 
except Exception: pass
ok("logout clears session", (lambda c:(c.call("POST","/api/logout"), c.call("GET","/api/me")[0])[1])(c2)==401)
print("\nFAILURES:",fails); sys.exit(1 if fails else 0)
