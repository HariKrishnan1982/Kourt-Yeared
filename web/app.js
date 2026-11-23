"use strict";

/* ---------- tiny helpers ---------- */
// Every piece of user text goes through createTextNode, never innerHTML.
const h = (tag, props, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
};

async function api(method, path, body) {
  const res = await fetch("/api" + path, {
    method,
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", "x-kourt": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  let data = {};
  try { data = await res.json(); } catch (_) { /* empty body */ }
  if (!res.ok) {
    const err = new Error(data.error || "Something went wrong. Try again.");
    err.status = res.status;
    throw err;
  }
  return data;
}

function toast(msg, bad) {
  const t = h("div", { class: "toast" + (bad ? " bad" : "") }, msg);
  document.getElementById("toasts").append(t);
  setTimeout(() => t.remove(), 4200);
}

async function copyText(text, btn) {
  try { await navigator.clipboard.writeText(text); }
  catch (_) {
    const i = h("input", { value: text, style: "position:fixed;top:-9999px;opacity:0", "aria-hidden": "true" });
    document.body.append(i); i.select(); i.setSelectionRange(0, 99999); document.execCommand("copy"); i.remove();
  }
  const old = btn.textContent;
  btn.textContent = "Copied";
  setTimeout(() => { btn.textContent = old; }, 1600);
}

const fmtMin = (m) => {
  const r = (n) => Math.round(n * 10) / 10;
  if (m < 60) return `${r(m)} min`;
  if (m < 1440) { const hrs = Math.floor(m / 60), mm = Math.round(m - hrs * 60); return mm ? `${hrs} h ${mm} min` : `${hrs} h`; }
  const d = Math.floor(m / 1440), hrs = Math.round((m - d * 1440) / 60);
  return hrs ? `${d} d ${hrs} h` : `${d} d`;
};

function fmtLeft(ms) {
  const s = Math.max(0, Math.ceil(ms / 1000));
  if (s >= 86400) return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h left`;
  if (s >= 3600) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m left`;
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")} left`;
}

const caseNo = (id) => "KY-" + id.toUpperCase();

/* ---------- app state and routing ---------- */
const app = document.getElementById("app");
let me = null;
let site = { announcement: "", signups_open: true };
let cleanup = () => {};

async function loadSite() {
  try { site = await api("GET", "/site"); } catch (_) { /* keep the defaults */ }
}

async function loadMe() {
  try { me = await api("GET", "/me"); } catch (_) { me = null; }
}

function nav(path, replace) {
  history[replace ? "replaceState" : "pushState"](null, "", path);
  route();
}

document.addEventListener("click", (e) => {
  const a = e.target.closest("a[data-link]");
  if (!a || e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
  e.preventDefault();
  nav(a.getAttribute("href"));
});
window.addEventListener("popstate", route);

function shell(...content) {
  const who = me
    ? h("div", { class: "who" },
        h("span", null, me.username),
        h("button", { class: "btn text", type: "button", onclick: logout }, "Log out"))
    : null;
  const top = h("header", { class: "top" },
    h("a", { class: "mark", href: "/", "data-link": "" }, "Kourt Yeared"), who);
  const banner = site.announcement ? h("p", { class: "site-banner", role: "status" }, site.announcement) : null;
  app.replaceChildren(...[banner, top, h("main", { id: "main", class: "wrap" }, ...content)].filter(Boolean));
}

async function logout() {
  try { await api("POST", "/logout"); } catch (_) { /* already gone */ }
  me = null;
  nav("/", true);
}

function route() {
  cleanup(); cleanup = () => {};
  const p = location.pathname;
  let m;
  if ((m = p.match(/^\/case\/([a-z0-9]+)$/))) return caseView(m[1]);
  if ((m = p.match(/^\/join\/([A-Za-z0-9_-]+)$/))) return joinView(m[1]);
  if (p === "/new") return me ? newCaseView() : needLogin("/new");
  if (p === "/") return me ? homeView() : landingView();
  nav("/", true);
}

function needLogin(next) {
  sessionStorage.setItem("kourt-next", next);
  nav("/", true);
}

function afterAuth() {
  const next = sessionStorage.getItem("kourt-next");
  sessionStorage.removeItem("kourt-next");
  nav(next && next.startsWith("/") ? next : "/", true);
}

/* ---------- auth form ---------- */
function authForm(onDone) {
  let mode = site.signups_open ? "signup" : "login";
  const box = h("div");
  const render = () => {
    const err = h("p", { class: "err", role: "alert", hidden: true });
    const user = h("input", { type: "text", id: "au", name: "username", autocomplete: "username", autocapitalize: "none", spellcheck: "false", required: true });
    const pass = h("input", { type: "password", id: "ap", name: "password", autocomplete: mode === "signup" ? "new-password" : "current-password", required: true });
    const go = h("button", { class: "btn", type: "submit" }, mode === "signup" ? "Create account" : "Log in");
    const form = h("form", {
      onsubmit: async (e) => {
        e.preventDefault();
        go.disabled = true; err.hidden = true;
        try {
          await api("POST", mode === "signup" ? "/signup" : "/login", { username: user.value, password: pass.value });
          await loadMe();
          onDone();
        } catch (ex) { err.textContent = ex.message; err.hidden = false; go.disabled = false; }
      },
    },
      h("div", { class: "field" }, h("label", { for: "au" }, "Username", mode === "signup" ? h("span", { class: "hint" }, "3 to 20 letters, numbers, dots or underscores. Your friends will see this.") : null), user),
      h("div", { class: "field" }, h("label", { for: "ap" }, "Password", mode === "signup" ? h("span", { class: "hint" }, "At least 8 characters.") : null), pass),
      err,
      h("div", { class: "field" }, go));
    const tab = (m, label) => h("button", { class: "tab", type: "button", role: "tab", "aria-selected": String(mode === m), onclick: () => { mode = m; render(); } }, label);
    box.replaceChildren(
      site.signups_open
        ? h("div", { class: "tabs", role: "tablist" }, tab("signup", "Create account"), tab("login", "Log in"))
        : h("p", { class: "notice" }, "New accounts are paused right now. If you already have one, log in."),
      form);
  };
  render();
  return box;
}

/* ---------- landing ---------- */
function landingView() {
  document.title = "Kourt Yeared";
  const sample = h("article", { class: "sheet summons", "aria-label": "Example summons" },
    h("span", { class: "stamp" }, "Served"),
    h("p", { class: "casenum" }, "Case KY-7H2QF"),
    h("h2", null, "Summons"),
    h("dl", null,
      h("dt", null, "Plaintiff"), h("dd", null, "Kavya"),
      h("dt", null, "Defendant"), h("dd", null, "Arun"),
      h("dt", null, "Charge"), h("dd", null, "Ate the last samosa"),
      h("dt", null, "Stakes"), h("dd", null, "Two samosas"),
      h("dt", null, "Answer within"), h("dd", null, "3 minutes")));
  shell(h("div", { class: "hero" },
    h("div", { class: "intro stack" },
      h("h1", null, "File a case against your friend."),
      h("p", null, "Send them a summons link. They answer it, argue their side, and a jury of your friends decides who owes whom. If they ignore the summons, the court rules against them and they buy the treat."),
      h("section", { class: "sheet", "aria-label": "Account" }, authForm(afterAuth))),
    sample));
}

/* ---------- home ---------- */
const PHASE_TEXT = { summons: "Waiting for the defendant", arguments: "Arguments underway", voting: "Jury is voting" };
const OUTCOME_TEXT = {
  guilty: "Guilty", not_guilty: "Not guilty", default_judgment: "Guilty by default", dismissed: "Dismissed",
  mistrial: "Hung jury", abandoned: "Struck off", withdrawn: "Withdrawn",
};
const ROLE_TEXT = {
  plaintiff: "You filed this", defendant: "You're the defendant", plaintiff_lawyer: "You're the plaintiff's lawyer",
  defendant_lawyer: "You're the defense lawyer", jury: "You're on the jury",
};

async function homeView() {
  document.title = "Your cases | Kourt Yeared";
  await loadMe();
  let list = [];
  try { list = (await api("GET", "/cases")).cases; } catch (_) { /* shown as empty */ }
  const record = [];
  if (me.owes) record.push(h("span", null, `You owe ${me.owes} ${me.owes === 1 ? "treat" : "treats"}`));
  if (me.owed) record.push(h("span", null, `You're owed ${me.owed} ${me.owed === 1 ? "treat" : "treats"}`));
  shell(h("div", { class: "stack" },
    h("div", { class: "row" }, h("h1", null, "Your cases"), h("a", { class: "btn", href: "/new", "data-link": "" }, "File a case")),
    record.length ? h("p", { class: "record" }, record) : null,
    h("section", { class: "sheet" },
      list.length
        ? h("ul", { class: "cases" }, list.map((c) => h("li", null,
            h("a", { class: "case-link", href: "/case/" + c.id, "data-link": "" },
              h("strong", null, c.title),
              h("span", { class: "muted small" },
                `${ROLE_TEXT[c.role]} \u00b7 ${c.phase === "closed" ? OUTCOME_TEXT[c.outcome] || "Closed" : PHASE_TEXT[c.phase]}`)))))
        : h("p", { class: "empty" }, "No cases yet. File one, or open an invite link a friend sent you."))));
}

/* ---------- file a case ---------- */
const PRESETS = { quick: 12, standard: 180, marathon: 4320 };
const splitMin = (m) => { const s = Math.max(2, m / 4), a = m / 2; return { s, a, v: Math.max(1, m - s - a) }; };

function newCaseView() {
  document.title = "File a case | Kourt Yeared";
  let preset = "quick";
  const f = (name, extra) => h("input", { type: "text", id: name, name, autocomplete: "off", ...extra });
  const title = f("title", { maxlength: "80", required: true, placeholder: "The great samosa heist" });
  const charge = f("charge", { maxlength: "80", required: true, placeholder: "Ate the last samosa" });
  const dname = f("defendant_name", { maxlength: "24", required: true, placeholder: "What you call them" });
  const details = h("textarea", { id: "details", maxlength: "600", placeholder: "Dates, places, witnesses. Keep it fun." });
  const demand = f("demand", { maxlength: "60", placeholder: "Leave blank to let the stakes decide" });
  const custom = h("input", { type: "number", id: "custom", min: "6", max: "20160", value: "30", "aria-label": "Custom length in minutes" });
  const customWrap = h("div", { class: "field", hidden: true }, h("label", { for: "custom" }, "Length in minutes", h("span", { class: "hint" }, "6 minutes to 14 days (20160)")), custom);
  const plan = h("p", { class: "muted small", "aria-live": "polite" });
  const err = h("p", { class: "err", role: "alert", hidden: true });
  const go = h("button", { class: "btn", type: "submit" }, "File case");

  const total = () => (preset === "custom" ? Math.max(6, Math.min(20160, Number(custom.value) || 6)) : PRESETS[preset]);
  const refreshPlan = () => {
    const t = splitMin(total());
    plan.textContent = `${fmtMin(total())} in all: ${fmtMin(t.s)} to answer the summons, ${fmtMin(t.a)} of arguments, ${fmtMin(t.v)} for the jury.`;
    customWrap.hidden = preset !== "custom";
  };
  const radio = (group, value, label, sub, checked, onchange) => h("label", { class: "choice" },
    h("input", { type: "radio", name: group, value, checked, onchange }), h("span", null, label, sub ? h("small", null, sub) : null));
  let tier = "snack";
  const tiers = h("fieldset", { class: "choices" }, h("legend", null, "Stakes"),
    radio("tier", "snack", "A snack", "Samosa, chai, chips", true, () => { tier = "snack"; }),
    radio("tier", "meal", "A meal", "Proper food", false, () => { tier = "meal"; }),
    radio("tier", "gift", "A gift", "Something small", false, () => { tier = "gift"; }));
  const lengths = h("fieldset", { class: "choices" }, h("legend", null, "Trial length"),
    radio("len", "quick", "Quick", "About 12 minutes, live", true, () => { preset = "quick"; refreshPlan(); }),
    radio("len", "standard", "Standard", "3 hours", false, () => { preset = "standard"; refreshPlan(); }),
    radio("len", "marathon", "Marathon", "3 days", false, () => { preset = "marathon"; refreshPlan(); }),
    radio("len", "custom", "Custom", "You choose", false, () => { preset = "custom"; refreshPlan(); }));
  custom.addEventListener("input", refreshPlan);
  refreshPlan();

  const form = h("form", {
    class: "sheet stack",
    onsubmit: async (e) => {
      e.preventDefault(); err.hidden = true; go.disabled = true;
      try {
        const body = { title: title.value, charge: charge.value, defendant_name: dname.value, details: details.value, tier, demand: demand.value, preset };
        if (preset === "custom") body.minutes = Number(custom.value);
        const { id } = await api("POST", "/cases", body);
        nav("/case/" + id);
      } catch (ex) { err.textContent = ex.message; err.hidden = false; go.disabled = false; }
    },
  },
    h("div", { class: "field" }, h("label", { for: "title" }, "Name the case"), title),
    h("div", { class: "field" }, h("label", { for: "charge" }, "The charge"), charge),
    h("div", { class: "field" }, h("label", { for: "defendant_name" }, "Who are you suing?", h("span", { class: "hint" }, "Shown on the summons. They'll join with their own account.")), dname),
    h("div", { class: "field" }, h("label", { for: "details" }, "What happened?", h("span", { class: "hint" }, "Optional")), details),
    tiers,
    h("div", { class: "field" }, h("label", { for: "demand" }, "Your exact demand", h("span", { class: "hint" }, "Optional. For example: two samosas and a chai.")), demand),
    lengths, customWrap, plan, err,
    h("div", { class: "row" }, go, h("a", { class: "btn alt", href: "/", "data-link": "" }, "Cancel")));
  shell(h("div", { class: "stack" }, h("h1", null, "File a case"), form));
}

/* ---------- join by invite link ---------- */
const JOIN_COPY = {
  defendant: (p) => ({ lead: `${p.plaintiff} has filed a case against you.`, action: "Answer the summons" }),
  plaintiff_lawyer: (p) => ({ lead: `${p.plaintiff} needs a lawyer.`, action: "Argue for the plaintiff" }),
  defendant_lawyer: (p) => ({ lead: `${p.defendant} needs a lawyer.`, action: "Argue for the defense" }),
  jury: (p) => ({ lead: `${p.plaintiff} is asking you to sit on the jury.`, action: "Join the jury" }),
};

async function joinView(token) {
  await loadMe();
  let peek;
  try { peek = await api("GET", "/join/" + token); }
  catch (ex) { shell(h("section", { class: "sheet stack" }, h("h1", null, "This link doesn't work"), h("p", null, ex.message), h("a", { class: "btn", href: "/", "data-link": "" }, "Go home"))); return; }
  document.title = `${peek.title} | Kourt Yeared`;
  const copy = JOIN_COPY[peek.role](peek);
  const dead = peek.closed ? "This case is already closed." : peek.taken && peek.role !== "jury" ? "Someone already used this link. Ask for a fresh one." : null;
  const doJoin = async (btn) => {
    btn.disabled = true;
    try { const { id } = await api("POST", "/join/" + token); nav("/case/" + id, true); }
    catch (ex) { toast(ex.message, true); btn.disabled = false; }
  };
  const btn = h("button", { class: "btn", type: "button", onclick: (e) => doJoin(e.currentTarget) }, copy.action);
  const summons = h("article", { class: "sheet stack" },
    h("p", { class: "casenum" }, "Summons"),
    h("h1", null, peek.title),
    h("p", null, copy.lead),
    dead ? h("p", { class: "err" }, dead) : me ? btn : h("div", null, h("p", { class: "muted" }, "Log in or create an account to continue. You'll come straight back here."), (sessionStorage.setItem("kourt-next", location.pathname), authForm(afterAuth))));
  shell(summons);
}

/* ---------- the courtroom ---------- */
const STEPS = [["summons", "Summons"], ["arguments", "Arguments"], ["voting", "Vote"], ["closed", "Verdict"]];
const ROLE_LINE = {
  plaintiff: "You are the plaintiff.", defendant: "You are the defendant.", plaintiff_lawyer: "You are arguing for the plaintiff.",
  defendant_lawyer: "You are arguing for the defense.", jury: "You are on the jury.",
};

function caseView(id) {
  let snap = null, ws = null, wsUp = false, stopped = false, retry = 0, offset = 0, lastFetch = 0, composerSig = "", lastLogSig = "", shownVerdict = false;
  const head = h("section", { class: "sheet docket", "aria-label": "Case details" });
  const tracker = h("div");
  const log = h("ol", { class: "log", role: "log", "aria-live": "polite", "aria-label": "Court transcript" });
  const composerBox = h("section", { "aria-label": "Your statement" });
  const statusBox = h("section", { class: "stack", "aria-label": "Court status" });
  const room = h("div", { class: "stack" }, head, tracker, h("div", { class: "stack" }, log), composerBox, statusBox);
  shell(room);

  const apply = (s) => {
    snap = s; lastFetch = Date.now(); offset = s.now_ms - Date.now();
    document.title = `${s.title} | Kourt Yeared`;
    drawHead(); drawTracker(); drawLog(); drawComposer(); drawStatus();
  };

  const refetch = async () => {
    try { apply(await api("GET", "/cases/" + id)); }
    catch (ex) {
      if (ex.status === 401) return needLogin(location.pathname);
      if (ex.status === 404) {
        cleanup();
        shell(h("section", { class: "sheet stack" }, h("h1", null, "Case not found"), h("p", null, "This case does not exist or was removed by the court."), h("a", { class: "btn", href: "/", "data-link": "" }, "Go home")));
        return;
      }
      if (!snap) shell(h("section", { class: "sheet stack" }, h("h1", null, "You're not on this case"), h("p", null, "Open the invite link you were sent to take your seat."), h("a", { class: "btn", href: "/", "data-link": "" }, "Go home")));
    }
  };

  function drawHead() {
    const s = snap;
    const left = s.defendant || s.defendant_label;
    head.replaceChildren(
      h("p", { class: "casenum" }, "Case " + caseNo(s.id)),
      h("h1", null, s.title),
      h("p", { class: "parties" }, s.plaintiff, h("span", { class: "v" }, "v."), left),
      h("dl", { class: "facts" },
        h("dt", null, "Charge"), h("dd", null, s.charge),
        h("dt", null, "Stakes"), h("dd", null, s.demand),
        s.plaintiff_lawyer ? [h("dt", null, "Plaintiff's lawyer"), h("dd", null, s.plaintiff_lawyer)] : null,
        s.defendant_lawyer ? [h("dt", null, "Defense lawyer"), h("dd", null, s.defendant_lawyer)] : null,
        h("dt", null, "Jury"), h("dd", null, s.jury.length ? `${s.jury.length} seated` : "Empty so far")),
      s.details ? h("p", { class: "quote" }, s.details) : null,
      h("p", { class: "muted small" }, ROLE_LINE[s.me.role]));
  }

  function drawTracker() {
    const s = snap, idx = STEPS.findIndex(([k]) => k === s.phase);
    tracker.replaceChildren(
      h("ol", { class: "steps", "aria-label": "Case progress" },
        STEPS.map(([k, label], i) => h("li", { class: i < idx ? "done" : "", "aria-current": i === idx ? "step" : null }, label))),
      s.phase === "closed" ? null : h("p", { class: "clock", id: "clock" }, ""));
    tickClock();
  }

  function tickClock() {
    const el = document.getElementById("clock");
    if (!el || !snap || snap.phase === "closed") return;
    const left = snap.deadline_ms - (Date.now() + offset);
    el.textContent = left > 0 ? fmtLeft(left) : "Time. The court is moving on\u2026";
    if (left < -2500 && Date.now() - lastFetch > 4000) refetch();
  }

  function drawLog() {
    const s = snap;
    const sig = s.messages.length + ":" + (s.messages.length ? s.messages[s.messages.length - 1].id : 0);
    if (sig === lastLogSig) return;
    const nearBottom = window.innerHeight + window.scrollY >= document.body.scrollHeight - 160;
    lastLogSig = sig;
    log.replaceChildren(...(s.messages.length
      ? s.messages.map((m) => m.kind === "objection"
          ? h("li", { class: "objection " + m.side },
              h("span", { class: "stamp" }, "Objection"),
              h("span", null, `${m.user}${m.body ? ": " + m.body : ""}`))
          : m.removed
            ? h("li", { class: "entry removed " + m.side }, h("p", { class: "by" }, m.user), h("p", null, "Removed by the court"))
            : h("li", { class: "entry " + m.side },
                h("p", { class: "by" }, m.user, m.lawyer ? h("span", { class: "tag" }, " (lawyer)") : null),
                h("p", null, m.body)))
      : [h("li", { class: "empty" }, s.phase === "summons" ? "Court is not in session yet." : "No statements yet.")]));
    if (nearBottom && s.messages.length) log.lastElementChild.scrollIntoView({ block: "nearest" });
  }

  // The composer only rebuilds when you gain or lose the floor, so a draft survives every live update.
  function drawComposer() {
    const s = snap;
    const canArgue = s.phase === "arguments" && ["plaintiff", "defendant", "plaintiff_lawyer", "defendant_lawyer"].includes(s.me.role);
    const sig = String(canArgue);
    if (sig === composerSig) return;
    composerSig = sig;
    if (!canArgue) { composerBox.replaceChildren(); return; }
    const box = h("textarea", { id: "say", maxlength: "500", placeholder: "Make your case. Be persuasive, not cruel." });
    const count = h("p", { class: "counter" }, "500 left");
    box.addEventListener("input", () => { count.textContent = `${500 - box.value.length} left`; });
    const send = async (kind, btn) => {
      const body = kind === "objection" ? box.value.slice(0, 140) : box.value;
      if (kind === "statement" && !body.trim()) return toast("Write something first.", true);
      btn.disabled = true;
      try { await api("POST", `/cases/${id}/messages`, { kind, body }); box.value = ""; count.textContent = "500 left"; }
      catch (ex) { toast(ex.message, true); }
      btn.disabled = false;
    };
    composerBox.replaceChildren(h("div", { class: "sheet stack" },
      h("label", { for: "say" }, "Your statement"), box, count,
      h("div", { class: "row" },
        h("button", { class: "btn", type: "button", onclick: (e) => send("statement", e.currentTarget) }, "Submit statement"),
        h("button", { class: "btn alt", type: "button", onclick: (e) => send("objection", e.currentTarget) }, "Object"),
        h("span", { class: "muted small" }, "An objection uses what you typed as the reason (up to 140 characters). Each side gets 3."))));
  }

  function inviteRow(role, label, msg) {
    const token = snap.invites[role];
    if (!token) return null;
    const url = `${location.origin}/join/${token}`;
    const input = h("input", { type: "text", readonly: true, value: url, "aria-label": label, onfocus: (e) => e.target.select() });
    return h("div", { class: "invite" },
      h("span", { class: "label" }, label),
      input,
      h("div", { class: "row" },
        h("button", { class: "btn alt", type: "button", onclick: (e) => copyText(url, e.currentTarget) }, "Copy link"),
        h("a", { class: "btn alt", href: "https://wa.me/?text=" + encodeURIComponent(`${msg} ${url}`), target: "_blank", rel: "noopener noreferrer" }, "Send on WhatsApp")));
  }

  function drawStatus() {
    const s = snap, role = s.me.role, parts = [];
    const invites = [];
    if (s.phase !== "closed") {
      invites.push(
        inviteRow("defendant", `Summons for ${s.defendant_label}`, `You've been summoned to Kourt Yeared. ${s.plaintiff} filed a case against you: "${s.title}". Answer before time runs out:`),
        inviteRow("plaintiff_lawyer", "Link for your lawyer", `I need a lawyer in Kourt Yeared: "${s.title}". Argue my side:`),
        inviteRow("defendant_lawyer", "Link for your lawyer", `I need a lawyer in Kourt Yeared: "${s.title}". Argue my side:`),
        inviteRow("jury", "Jury link", `Sit on the jury in Kourt Yeared: "${s.title}". Hear both sides and vote:`));
    }
    const shown = invites.filter(Boolean);

    if (s.phase === "summons") {
      parts.push(h("div", { class: "sheet stack" },
        h("h2", null, role === "plaintiff" ? "Serve the summons" : "Waiting for the defendant"),
        role === "plaintiff"
          ? h("p", null, `Send ${s.defendant_label} their link. If they haven't joined when the clock runs out, judgment is entered against them and they owe you ${s.demand}.`)
          : h("p", null, `${s.defendant_label} has until the clock runs out to answer. If they don't, the court rules against them.`),
        shown,
        role === "plaintiff" ? h("button", { class: "btn alt", type: "button", onclick: withdraw }, "Withdraw case") : null));
    } else if (s.phase === "arguments") {
      if (role === "jury") parts.push(h("div", { class: "sheet" }, h("h2", null, "Listen closely"), h("p", null, "Voting opens when arguments end. You'll see every statement above.")));
      if (shown.length) parts.push(h("div", { class: "sheet stack" }, h("h2", null, "Bring in more people"), shown));
    } else if (s.phase === "voting") {
      if (role === "jury") {
        const vote = (v) => async (e) => {
          const b = e.currentTarget; b.disabled = true;
          try { await api("POST", `/cases/${id}/vote`, { vote: v }); } catch (ex) { toast(ex.message, true); }
          b.disabled = false;
        };
        parts.push(h("div", { class: "sheet stack" },
          h("h2", null, "Cast your vote"),
          h("div", { class: "votes" },
            h("button", { class: "btn red", type: "button", "aria-pressed": String(s.my_vote === "guilty"), onclick: vote("guilty") }, "Guilty"),
            h("button", { class: "btn", type: "button", "aria-pressed": String(s.my_vote === "not_guilty"), onclick: vote("not_guilty") }, "Not guilty")),
          h("p", { class: "muted small" }, s.my_vote ? "Your vote is in. You can change it until voting ends." : "You haven't voted yet.")));
      }
      parts.push(h("div", { class: "sheet" }, h("h2", null, "The jury is deliberating"),
        h("p", null, `${s.jury_voted} of ${s.jury.length} jurors have voted. The verdict lands when everyone has voted or time runs out.`)));
      if (shown.length && role !== "jury") parts.push(h("div", { class: "sheet stack" }, h("h2", null, "Need more jurors?"), shown));
    } else {
      parts.push(verdictCard());
    }
    statusBox.replaceChildren(...parts);
  }

  async function withdraw() {
    if (!confirm("Withdraw this case? Nobody will owe anything.")) return;
    try { await api("POST", `/cases/${id}/withdraw`); } catch (ex) { toast(ex.message, true); }
  }

  function verdictCard() {
    const s = snap, o = s.outcome, t = s.tally || { guilty: 0, not_guilty: 0 };
    const d = s.defendant || s.defendant_label;
    const cleared = o === "not_guilty" || o === "dismissed";
    const LINES = {
      guilty: `The jury found ${d} guilty, ${t.guilty} to ${t.not_guilty}.`,
      not_guilty: `The jury cleared ${d}, ${t.not_guilty} to ${t.guilty}. This was a false alarm.`,
      default_judgment: `${d} didn't argue their side in time, so judgment is entered against them.`,
      dismissed: `${s.plaintiff}'s side never made its case, so the court dismissed it.`,
      mistrial: "The jury split evenly. Nobody owes anything. Share a chai and call it even.",
      abandoned: "Nobody argued, so the court struck the case off.",
      withdrawn: `${s.plaintiff} withdrew the case before it began.`,
    };
    const first = !shownVerdict; shownVerdict = true;
    const sentence = s.sentence
      ? h("div", { class: "sentence" },
          h("p", { class: "label" }, "Sentence"),
          h("p", null, h("strong", null, s.sentence.text)),
          h("p", { class: "muted small" }, s.sentence.paid ? "Paid. Case settled." : `Owed by ${s.sentence.debtor || s.defendant_label}. Not paid yet.`),
          s.sentence.i_am_owed && !s.sentence.paid ? h("button", { class: "btn", type: "button", onclick: async (e) => { e.currentTarget.disabled = true; try { await api("POST", `/cases/${id}/paid`); } catch (ex) { toast(ex.message, true); } } }, "Mark as paid") : null)
      : null;
    const card = h("article", { class: "sheet verdict" + (first ? " shake" : "") },
      h("h2", null, "The court has ruled"),
      h("span", { class: "stamp big" + (cleared || o === "mistrial" || o === "abandoned" || o === "withdrawn" ? " cleared" : "") + (first ? " slam" : "") }, OUTCOME_TEXT[o] || "Closed"),
      h("p", null, LINES[o] || "This case is closed."),
      sentence,
      h("p", null, h("a", { class: "btn alt", href: me ? "/new" : "/", "data-link": "" }, "File another case")));
    return card;
  }

  function connect() {
    if (stopped) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    try { ws = new WebSocket(`${proto}://${location.host}/api/cases/${id}/ws`); } catch (_) { return; }
    ws.onopen = () => { wsUp = true; retry = 0; };
    ws.onerror = () => { wsUp = false; };
    ws.onmessage = (e) => { try { apply(JSON.parse(e.data)); } catch (_) { /* ignore a bad frame */ } };
    ws.onclose = () => { wsUp = false; if (!stopped) setTimeout(connect, Math.min(8000, 700 * 2 ** retry++)); };
  }

  const clock = setInterval(tickClock, 1000);
  const poll = setInterval(() => { if (!wsUp && !stopped) refetch(); }, 6000);
  cleanup = () => { stopped = true; clearInterval(clock); clearInterval(poll); if (ws) ws.close(); };

  loadMe().then(async () => {
    if (!me) return needLogin(location.pathname);
    await refetch();
    if (snap) connect();
  });
}

/* ---------- boot ---------- */
Promise.all([loadMe(), loadSite()]).then(route);
