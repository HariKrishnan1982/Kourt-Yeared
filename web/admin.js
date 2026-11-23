"use strict";

/* ---------- helpers (user text always goes through createTextNode) ---------- */
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

const app = document.getElementById("app");
let admin = null;

const fmt = (ms) => (ms ? new Date(ms).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : "Never");
const ago = (ms) => {
  if (!ms) return "Never";
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (s < 60) return "Just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
};
const bytes = (n) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`);
const uptime = (s) => (s < 3600 ? `${Math.floor(s / 60)} min` : s < 86400 ? `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min` : `${Math.floor(s / 86400)} d ${Math.floor((s % 86400) / 3600)} h`);

const OUTCOME = { guilty: "Guilty", not_guilty: "Not guilty", default_judgment: "Guilty by default", dismissed: "Dismissed", mistrial: "Hung jury", abandoned: "Struck off", withdrawn: "Withdrawn" };
const PHASE = { summons: "Waiting for defendant", arguments: "Arguments", voting: "Jury voting", closed: "Closed" };
const ROLE = { plaintiff: "Plaintiff", defendant: "Defendant", plaintiff_lawyer: "Plaintiff's lawyer", defendant_lawyer: "Defense lawyer", jury: "Juror" };
const caseStatus = (c) => (c.phase === "closed" ? OUTCOME[c.outcome] || "Closed" : PHASE[c.phase]);

async function api(method, path, body) {
  const res = await fetch("/api/admin" + path, {
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
    if (res.status === 401 && path !== "/login" && admin) { admin = null; loginView("Your session ended. Log in again."); }
    throw err;
  }
  return data;
}

// Errors that already changed the screen (login, unavailable) shouldn't also toast.
const fail = (ex) => { if (ex.status !== 401 && ex.status !== 404) toast(ex.message, true); };

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
  const old = btn.textContent; btn.textContent = "Copied";
  setTimeout(() => { btn.textContent = old; }, 1600);
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

/* ---------- layout ---------- */
const LINKS = [["/admin", "Overview"], ["/admin/users", "Users"], ["/admin/cases", "Cases"], ["/admin/settings", "Settings"], ["/admin/log", "Activity log"], ["/admin/account", "Account"]];

function shell(...content) {
  const path = location.pathname.replace(/\/+$/, "") || "/admin";
  const current = (href) => (href === "/admin" ? path === "/admin" : path.startsWith(href));
  app.replaceChildren(
    h("header", { class: "adm-top" },
      h("div", { class: "adm-bar" },
        h("a", { class: "mark", href: "/admin", "data-link": "" }, "Clerk's office"),
        h("nav", { "aria-label": "Admin sections" }, LINKS.map(([href, label]) => h("a", { href, "data-link": "", "aria-current": current(href) ? "page" : null }, label))),
        h("div", { class: "who" }, h("span", null, admin.username), h("button", { class: "btn text", type: "button", onclick: logout }, "Log out")))),
    h("main", { id: "main", class: "wrap adm" }, ...content));
}

function table(heads, rows, empty) {
  if (!rows.length) return h("p", { class: "muted" }, empty || "Nothing here yet.");
  return h("div", { class: "tablewrap" }, h("table", null,
    h("thead", null, h("tr", null, heads.map((x) => h("th", { class: x.startsWith("#") ? "num" : null }, x.replace(/^#/, ""))))),
    h("tbody", null, rows.map((r) => h("tr", null, r.map((c, i) => h("td", { class: heads[i].startsWith("#") ? "num" : null }, c)))))));
}

function pager(total, page, per, base) {
  const pages = Math.max(1, Math.ceil(total / per));
  const href = (p) => { const sp = new URLSearchParams(location.search); sp.set("page", p); return `${base}?${sp}`; };
  return h("div", { class: "pager" },
    h("span", { class: "muted small" }, `${total} total \u00b7 page ${page} of ${pages}`),
    page > 1 ? h("a", { class: "btn alt", href: href(page - 1), "data-link": "" }, "Previous") : null,
    page < pages ? h("a", { class: "btn alt", href: href(page + 1), "data-link": "" }, "Next") : null);
}

/* ---------- login ---------- */
function loginView(message) {
  document.title = "Admin login | Kourt Yeared";
  const err = h("p", { class: "err", role: "alert", hidden: !message }, message || "");
  const user = h("input", { type: "text", id: "au", autocomplete: "username", autocapitalize: "none", spellcheck: "false", required: true });
  const pass = h("input", { type: "password", id: "ap", autocomplete: "current-password", required: true });
  const go = h("button", { class: "btn", type: "submit" }, "Log in");
  const form = h("form", {
    class: "sheet stack",
    onsubmit: async (e) => {
      e.preventDefault(); go.disabled = true; err.hidden = true;
      try {
        admin = await api("POST", "/login", { username: user.value, password: pass.value });
        route();
      } catch (ex) { err.textContent = ex.message; err.hidden = false; go.disabled = false; }
    },
  },
    h("h1", null, "Admin login"),
    h("p", { class: "note" }, "This page only works on the computer that runs Kourt Yeared. Your admin password is separate from player accounts."),
    h("div", { class: "field" }, h("label", { for: "au" }, "Admin username"), user),
    h("div", { class: "field" }, h("label", { for: "ap" }, "Password"), pass),
    err, h("div", { class: "field" }, go));
  app.replaceChildren(h("main", { id: "main", class: "login-wrap" }, h("p", { class: "mark" }, "Kourt Yeared"), form));
}

function unavailableView() {
  app.replaceChildren(h("main", { id: "main", class: "login-wrap" }, h("section", { class: "sheet stack" },
    h("h1", null, "Not available here"),
    h("p", null, "The admin page only opens on the computer running the server. Open http://localhost:3000/admin on that computer."))));
}

async function logout() {
  try { await api("POST", "/logout"); } catch (_) { /* already ended */ }
  admin = null; loginView();
}

/* ---------- overview ---------- */
async function overview() {
  document.title = "Overview | Clerk's office";
  let s;
  try { s = await api("GET", "/stats"); } catch (ex) { return fail(ex); }
  const tile = (n, label, warn) => h("div", { class: "tile" + (warn ? " warn" : "") }, h("strong", null, n), h("span", null, label));
  const list = (obj, names) => { const e = Object.entries(obj); return e.length ? h("ul", { class: "pill-list" }, e.map(([k, v]) => h("li", null, names[k] || k, ": ", h("b", null, v)))) : h("p", { class: "muted" }, "None yet."); };
  shell(h("div", { class: "stack" },
    h("h1", null, "Overview"),
    h("div", { class: "tiles" },
      tile(s.users, `Players (${s.new_24h} new today, ${s.new_7d} this week)`),
      tile(s.banned, "Suspended players", s.banned > 0),
      tile(s.open_cases, `Open cases (of ${s.cases})`),
      tile(s.unpaid, "Unpaid treats", s.unpaid > 0),
      tile(s.messages, "Statements made"),
      tile(s.sessions, "Active logins"),
      tile(bytes(s.db_bytes), "Database size"),
      tile(uptime(s.uptime_s), "Server uptime")),
    h("p", null, s.signups_open ? "New accounts are open." : "New accounts are paused.", " ", h("a", { href: "/admin/settings", "data-link": "" }, "Change in settings")),
    h("div", { class: "cols two" },
      h("section", { class: "sheet" }, h("h2", null, "Cases by stage"), list(s.phases, PHASE)),
      h("section", { class: "sheet" }, h("h2", null, "Verdicts so far"), list(s.outcomes, OUTCOME))),
    h("div", { class: "cols two" },
      h("section", null, h("h2", null, "Newest players"),
        table(["Player", "Joined"], s.recent_users.map((u) => [h("a", { href: "/admin/users/" + u.id, "data-link": "" }, u.username), ago(u.created_ms)]), "No players yet.")),
      h("section", null, h("h2", null, "Newest cases"),
        table(["Case", "Filed by", "Status"], s.recent_cases.map((c) => [h("a", { href: "/admin/cases/" + c.id, "data-link": "" }, c.title), c.plaintiff, caseStatus(c)]), "No cases yet.")))));
}

/* ---------- users ---------- */
async function usersView() {
  document.title = "Users | Clerk's office";
  const sp = new URLSearchParams(location.search);
  const q = sp.get("q") || "", page = Number(sp.get("page") || 1);
  let d;
  try { d = await api("GET", "/users?" + new URLSearchParams({ q, page })); } catch (ex) { return fail(ex); }
  const input = h("input", { type: "text", id: "q", value: q, placeholder: "Username or ID", autocomplete: "off" });
  const rows = d.users.map((u) => [
    u.id,
    h("a", { href: "/admin/users/" + u.id, "data-link": "" }, u.username),
    ago(u.created_ms), ago(u.last_login_ms), u.cases, u.owes,
    u.banned ? h("span", { class: "badge red" }, u.ban_reason === "Account deleted by an admin" ? "Deleted" : "Suspended") : h("span", { class: "badge soft" }, "Active"),
  ]);
  shell(h("div", { class: "stack" },
    h("h1", null, "Users"),
    h("form", { class: "toolbar", role: "search", onsubmit: (e) => { e.preventDefault(); nav("/admin/users?" + new URLSearchParams({ q: input.value })); } },
      h("div", { class: "field" }, h("label", { for: "q" }, "Find a player"), input),
      h("button", { class: "btn", type: "submit" }, "Search")),
    table(["#ID", "Username", "Joined", "Last login", "#Cases", "#Owes", "Status"], rows, q ? "No players match that search." : "No players yet."),
    pager(d.total, d.page, d.per_page, "/admin/users")));
}

async function userView(id) {
  let u;
  try { u = await api("GET", "/users/" + id); } catch (ex) { return fail(ex); }
  document.title = `${u.username} | Clerk's office`;
  const act = (path, body, msg) => async (e) => {
    const b = e.currentTarget; b.disabled = true;
    try { await api("POST", `/users/${id}${path}`, body && body()); toast(msg); userView(id); }
    catch (ex) { fail(ex); b.disabled = false; }
  };
  const reason = h("input", { type: "text", id: "reason", maxlength: "200", placeholder: "Reason (shown to them when they try to log in)" });
  const rename = h("input", { type: "text", id: "rename", value: u.username, maxlength: "20", autocomplete: "off" });
  const note = h("textarea", { id: "note", class: "short", maxlength: "500" }, u.admin_note);
  note.value = u.admin_note;
  const status = u.deleted ? h("span", { class: "badge red" }, "Deleted") : u.banned ? h("span", { class: "badge red" }, "Suspended") : h("span", { class: "badge soft" }, "Active");

  const actions = u.deleted
    ? h("p", { class: "muted" }, "This account was deleted. Its cases are kept, and the name is free to register again.")
    : h("div", { class: "actions" },
        u.banned
          ? h("div", { class: "action" }, h("p", { class: "label" }, "Suspended"), h("p", { class: "muted small" }, u.ban_reason ? `Reason: ${u.ban_reason}` : "No reason given."),
              h("button", { class: "btn", type: "button", onclick: act("/unban", null, "Account restored") }, "Restore account"))
          : h("div", { class: "action" }, h("label", { for: "reason" }, "Suspend this player"), reason,
              h("button", { class: "btn red", type: "button", onclick: act("/ban", () => ({ reason: reason.value }), "Account suspended") }, "Suspend")),
        h("div", { class: "action" }, h("p", { class: "label" }, "Logins"), h("p", { class: "muted small" }, `${u.sessions} active ${u.sessions === 1 ? "login" : "logins"}.`),
          h("button", { class: "btn alt", type: "button", onclick: act("/logout", null, "Logged out everywhere") }, "Log out everywhere")),
        h("div", { class: "action" }, h("p", { class: "label" }, "Forgot their password?"),
          h("button", { class: "btn alt", type: "button", onclick: async (e) => {
            if (!confirm(`Issue a temporary password for ${u.username}? Their current password stops working.`)) return;
            e.currentTarget.disabled = true;
            try { const r = await api("POST", `/users/${id}/password`); revealPassword(u, r.password); } catch (ex) { fail(ex); e.currentTarget.disabled = false; }
          } }, "Issue temporary password")),
        h("div", { class: "action" }, h("label", { for: "rename" }, "Username"), rename,
          h("button", { class: "btn alt", type: "button", onclick: act("/rename", () => ({ username: rename.value }), "Username changed") }, "Change username")));

  shell(h("div", { class: "stack" },
    h("p", null, h("a", { href: "/admin/users", "data-link": "" }, "All users")),
    h("div", { class: "row" }, h("h1", null, u.username), status),
    h("div", { class: "cols side" },
      h("div", { class: "stack" },
        h("section", { class: "sheet" }, h("h2", null, "Details"), h("dl", { class: "kv" },
          h("dt", null, "ID"), h("dd", null, u.id),
          h("dt", null, "Joined"), h("dd", null, fmt(u.created_ms)),
          h("dt", null, "Last login"), h("dd", null, fmt(u.last_login_ms)),
          h("dt", null, "Statements"), h("dd", null, u.messages),
          h("dt", null, "Owes"), h("dd", null, `${u.owes} unpaid ${u.owes === 1 ? "treat" : "treats"}`),
          h("dt", null, "Is owed"), h("dd", null, `${u.owed} unpaid ${u.owed === 1 ? "treat" : "treats"}`))),
        h("section", null, h("h2", null, "Cases"),
          table(["Case", "Role", "Status"], u.cases.map((c) => [h("a", { href: "/admin/cases/" + c.id, "data-link": "" }, c.title), ROLE[c.role], caseStatus(c)]), "Not in any case.")),
        h("section", null, h("h2", null, "Latest statements"),
          u.recent_messages.length
            ? h("ul", { class: "msglist" }, u.recent_messages.map((m) => h("li", { class: "msg" + (m.removed ? " gone" : "") }, h("p", { class: "by" }, h("a", { href: "/admin/cases/" + m.case_id, "data-link": "" }, "Case " + m.case_id.toUpperCase()), " \u00b7 ", ago(m.at)), h("p", null, m.body))))
            : h("p", { class: "muted" }, "No statements."))),
      h("div", { class: "stack" },
        h("section", { class: "sheet" }, h("h2", null, "Manage"), actions),
        h("section", { class: "sheet" }, h("label", { for: "note" }, "Private note", h("span", { class: "hint" }, "Only admins see this.")), note,
          h("p", null, h("button", { class: "btn alt", type: "button", onclick: act("/note", () => ({ note: note.value }), "Note saved") }, "Save note"))),
        u.deleted ? null : h("section", { class: "sheet danger" }, h("h2", null, "Delete account"),
          h("p", { class: "small" }, "Removes the player's login and blanks their name. Their cases stay so other players' history isn't broken. This can't be undone."),
          h("button", { class: "btn red", type: "button", onclick: async (e) => {
            const typed = prompt(`Type ${u.username} to delete this account.`);
            if (typed !== u.username) return;
            e.currentTarget.disabled = true;
            try { await api("POST", `/users/${id}/delete`); toast("Account deleted"); userView(id); } catch (ex) { fail(ex); e.currentTarget.disabled = false; }
          } }, "Delete account"))))));
}

function revealPassword(u, pw) {
  shell(h("section", { class: "sheet stack" },
    h("h1", null, "Temporary password"),
    h("p", null, `Give this to ${u.username}. It is shown once and can't be looked up again. They are logged out everywhere.`),
    h("div", { class: "reveal", "data-testid": "temp-password" }, pw),
    h("div", { class: "row" },
      h("button", { class: "btn", type: "button", onclick: (e) => copyText(pw, e.currentTarget) }, "Copy"),
      h("a", { class: "btn alt", href: "/admin/users/" + u.id, "data-link": "" }, "Back to player"))));
}

/* ---------- cases ---------- */
async function casesView() {
  document.title = "Cases | Clerk's office";
  const sp = new URLSearchParams(location.search);
  const q = sp.get("q") || "", phase = sp.get("phase") || "", page = Number(sp.get("page") || 1);
  let d;
  try { d = await api("GET", "/cases?" + new URLSearchParams({ q, phase, page })); } catch (ex) { return fail(ex); }
  const input = h("input", { type: "text", id: "q", value: q, placeholder: "Title, charge, filer or case code", autocomplete: "off" });
  const sel = h("select", { id: "phase", "aria-label": "Stage" },
    [["", "All stages"], ["summons", "Waiting for defendant"], ["arguments", "Arguments"], ["voting", "Jury voting"], ["closed", "Closed"]].map(([v, l]) => h("option", { value: v, selected: v === phase }, l)));
  shell(h("div", { class: "stack" },
    h("h1", null, "Cases"),
    h("form", { class: "toolbar", role: "search", onsubmit: (e) => { e.preventDefault(); nav("/admin/cases?" + new URLSearchParams({ q: input.value, phase: sel.value })); } },
      h("div", { class: "field" }, h("label", { for: "q" }, "Find a case"), input),
      h("div", null, h("label", { for: "phase" }, "Stage"), sel),
      h("button", { class: "btn", type: "submit" }, "Filter")),
    table(["Case", "Filed by", "Against", "Status", "#People", "Filed"],
      d.cases.map((c) => [h("a", { href: "/admin/cases/" + c.id, "data-link": "" }, c.title), c.plaintiff, c.defendant_label, caseStatus(c), c.people, ago(c.created_ms)]),
      "No cases match."),
    pager(d.total, d.page, d.per_page, "/admin/cases")));
}

async function caseView(id) {
  let c;
  try { c = await api("GET", "/cases/" + id); } catch (ex) { return fail(ex); }
  document.title = `${c.title} | Clerk's office`;
  const open = c.phase !== "closed";
  const run = (path, body, msg) => async (e) => {
    const b = e.currentTarget; b.disabled = true;
    try { await api("POST", `/cases/${id}${path}`, body && body()); toast(msg); caseView(id); }
    catch (ex) { fail(ex); b.disabled = false; }
  };
  const f = (name, label, value, max) => h("div", { class: "field" }, h("label", { for: name }, label), h("input", { type: "text", id: name, value, maxlength: max, autocomplete: "off" }));
  const eTitle = f("e-title", "Title", c.title, 80), eCharge = f("e-charge", "Charge", c.charge, 80), eDemand = f("e-demand", "Demand", c.demand, 60);
  const eDetails = h("textarea", { id: "e-details", maxlength: "600" }); eDetails.value = c.details;
  const mins = h("input", { type: "number", id: "mins", min: "1", max: "10080", value: "10", "aria-label": "Minutes to add" });
  const out = h("select", { id: "outcome", "aria-label": "Outcome" }, Object.entries(OUTCOME).map(([v, l]) => h("option", { value: v }, l)));
  const edit = (el) => el.querySelector("input,textarea").value;

  const sentence = c.sentence
    ? h("div", { class: "action" }, h("p", { class: "label" }, "Sentence"), h("p", null, c.sentence, " ", h("span", { class: "badge " + (c.sentence_paid ? "soft" : "red") }, c.sentence_paid ? "Paid" : "Unpaid")),
        h("button", { class: "btn alt", type: "button", onclick: run("/paid", () => ({ paid: !c.sentence_paid }), "Updated") }, c.sentence_paid ? "Mark unpaid" : "Mark paid"))
    : null;

  shell(h("div", { class: "stack" },
    h("p", null, h("a", { href: "/admin/cases", "data-link": "" }, "All cases")),
    h("div", { class: "row" }, h("h1", null, c.title), h("span", { class: "badge" + (open ? "" : " soft") }, caseStatus(c))),
    h("p", { class: "muted small" }, `Case KY-${c.id.toUpperCase()} \u00b7 filed ${fmt(c.created_ms)}`),
    h("div", { class: "cols side" },
      h("div", { class: "stack" },
        h("section", { class: "sheet" }, h("h2", null, "Case"), h("dl", { class: "kv" },
          h("dt", null, "Plaintiff"), h("dd", null, c.plaintiff),
          h("dt", null, "Defendant"), h("dd", null, c.defendant ? `${c.defendant} (named as ${c.defendant_label})` : `${c.defendant_label} (hasn't joined)`),
          h("dt", null, "Charge"), h("dd", null, c.charge),
          h("dt", null, "Stakes"), h("dd", null, `${c.demand} (${c.tier})`),
          open ? [h("dt", null, "Stage ends"), h("dd", null, fmt(c.deadline_ms))] : [h("dt", null, "Closed"), h("dd", null, fmt(c.deadline_ms))],
          c.outcome ? [h("dt", null, "Vote"), h("dd", null, `${c.guilty} guilty, ${c.not_guilty} not guilty`)] : null,
          h("dt", null, "Details"), h("dd", null, c.details || "None"))),
        h("section", null, h("h2", null, "Transcript"),
          c.messages.length
            ? h("ul", { class: "msglist" }, c.messages.map((m) => h("li", { class: "msg msgrow " + m.side + (m.removed ? " gone" : "") },
                h("p", { class: "by" }, m.user, " \u00b7 ", m.kind === "objection" ? "objection" : m.side, " \u00b7 ", ago(m.at)),
                h("p", null, m.body || (m.kind === "objection" ? "(no reason given)" : "")),
                m.removed ? null : h("div", null, h("button", { class: "btn alt", type: "button", onclick: async (e) => {
                  if (!confirm("Remove this message from the transcript?")) return;
                  e.currentTarget.disabled = true;
                  try { await api("POST", `/cases/${id}/messages/${m.id}/remove`); toast("Message removed"); caseView(id); } catch (ex) { fail(ex); e.currentTarget.disabled = false; }
                } }, "Remove")))))
            : h("p", { class: "muted" }, "No statements.")),
        h("section", null, h("h2", null, "People"),
          table(["Player", "Role", "Joined"], c.people.map((p) => [h("a", { href: "/admin/users/" + p.id, "data-link": "" }, p.username), ROLE[p.role], ago(p.joined_ms)]))),
        h("section", null, h("h2", null, "Votes"),
          c.votes.length ? table(["Juror", "Vote"], c.votes.map((v) => [v.user, v.vote === "guilty" ? "Guilty" : "Not guilty"])) : h("p", { class: "muted" }, "No votes cast.")),
        h("section", null, h("h2", null, "Invite links"),
          table(["For", "Status"], c.invites.map((i) => [ROLE[i.role] || i.role, i.role === "jury" ? "Open to many" : i.claimed_by ? `Used by ${i.claimed_by}` : "Not used yet"])))),
      h("div", { class: "stack" },
        h("section", { class: "sheet" }, h("h2", null, "Manage"), h("div", { class: "actions" },
          open ? h("div", { class: "action" }, h("label", { for: "mins" }, "Give more time"), h("div", { class: "row" }, mins, h("span", null, "minutes")),
            h("button", { class: "btn alt", type: "button", onclick: run("/extend", () => ({ minutes: Number(mins.value) }), "Time added") }, "Extend current stage")) : null,
          open ? h("div", { class: "action" }, h("label", { for: "outcome" }, "Close the case now"), out,
            h("button", { class: "btn red", type: "button", onclick: (e) => { if (confirm(`Close this case as "${OUTCOME[out.value]}"? The normal rules for who owes what apply.`)) run("/close", () => ({ outcome: out.value }), "Case closed")(e); } }, "Close case")) : null,
          sentence)),
        h("section", { class: "sheet" }, h("h2", null, "Fix the wording"), eTitle, eCharge, eDemand,
          h("div", { class: "field" }, h("label", { for: "e-details" }, "Details"), eDetails),
          h("button", { class: "btn alt", type: "button", onclick: run("/edit", () => ({ title: edit(eTitle), charge: edit(eCharge), demand: edit(eDemand), details: eDetails.value }), "Case updated") }, "Save changes")),
        h("section", { class: "sheet danger" }, h("h2", null, "Delete case"),
          h("p", { class: "small" }, "Erases the case, transcript, votes and invite links for everyone. This can't be undone."),
          h("button", { class: "btn red", type: "button", onclick: async (e) => {
            const typed = prompt(`Type ${c.id} to delete this case.`);
            if (typed !== c.id) return;
            e.currentTarget.disabled = true;
            try { await api("POST", `/cases/${id}/delete`); toast("Case deleted"); nav("/admin/cases", true); } catch (ex) { fail(ex); e.currentTarget.disabled = false; }
          } }, "Delete case"))))));
}

/* ---------- settings, log, account ---------- */
async function settingsView() {
  document.title = "Settings | Clerk's office";
  let s;
  try { s = await api("GET", "/settings"); } catch (ex) { return fail(ex); }
  const open = h("input", { type: "checkbox", id: "signups", checked: s.signups_open });
  const ann = h("input", { type: "text", id: "ann", value: s.announcement, maxlength: "200", placeholder: "For example: Court is closed on Friday." });
  const max = h("input", { type: "number", id: "max", min: "1", max: "100", value: String(s.max_open_cases) });
  const err = h("p", { class: "err", role: "alert", hidden: true });
  shell(h("div", { class: "stack" }, h("h1", null, "Site settings"),
    h("form", { class: "sheet stack", onsubmit: async (e) => {
      e.preventDefault(); err.hidden = true;
      try { await api("POST", "/settings", { signups_open: open.checked, announcement: ann.value, max_open_cases: Number(max.value) }); toast("Settings saved"); settingsView(); }
      catch (ex) { if (ex.status === 400) { err.textContent = ex.message; err.hidden = false; } else fail(ex); }
    } },
      h("div", { class: "field" }, h("label", { class: "check", for: "signups" }, open, h("span", null, "Allow new accounts", h("small", null, "Turn this off to stop new sign-ups. Existing players can still log in.")))),
      h("div", { class: "field" }, h("label", { for: "ann" }, "Announcement banner", h("span", { class: "hint" }, "Shown at the top of every page. Leave empty for none.")), ann),
      h("div", { class: "field" }, h("label", { for: "max" }, "Open cases per player", h("span", { class: "hint" }, "How many unfinished cases one person can have at once.")), max),
      err, h("div", null, h("button", { class: "btn", type: "submit" }, "Save settings")))));
}

async function logView() {
  document.title = "Activity log | Clerk's office";
  let d;
  try { d = await api("GET", "/log"); } catch (ex) { return fail(ex); }
  shell(h("div", { class: "stack" }, h("h1", null, "Activity log"), h("p", { class: "muted" }, "Everything done from this page, newest first (last 200)."),
    table(["When", "Admin", "Action", "Target", "Detail"], d.log.map((e) => [h("span", { class: "nowrap" }, fmt(e.at)), e.admin, e.action.replaceAll("_", " "), e.target, e.detail]), "Nothing logged yet.")));
}

function accountView() {
  document.title = "Account | Clerk's office";
  const cur = h("input", { type: "password", id: "cur", autocomplete: "current-password", required: true });
  const nw = h("input", { type: "password", id: "new", autocomplete: "new-password", required: true });
  const err = h("p", { class: "err", role: "alert", hidden: true });
  shell(h("div", { class: "stack" }, h("h1", null, "Admin account"),
    h("form", { class: "sheet stack", onsubmit: async (e) => {
      e.preventDefault(); err.hidden = true;
      try { await api("POST", "/password", { current: cur.value, new: nw.value }); cur.value = ""; nw.value = ""; toast("Password changed. Other admin logins were ended."); }
      catch (ex) { if (ex.status === 400) { err.textContent = ex.message; err.hidden = false; } else fail(ex); }
    } },
      h("h2", null, "Change password"),
      h("div", { class: "field" }, h("label", { for: "cur" }, "Current password"), cur),
      h("div", { class: "field" }, h("label", { for: "new" }, "New password", h("span", { class: "hint" }, "At least 12 characters.")), nw),
      err, h("div", null, h("button", { class: "btn", type: "submit" }, "Change password"))),
    h("p", { class: "muted small" }, "Forgot it? Stop the server and start it again with KOURT_ADMIN_PASSWORD set to a new password.")));
}

/* ---------- routing ---------- */
function route() {
  if (!admin) return loginView();
  const p = location.pathname.replace(/\/+$/, "") || "/admin";
  let m;
  if (p === "/admin") return overview();
  if (p === "/admin/users") return usersView();
  if ((m = p.match(/^\/admin\/users\/(\d+)$/))) return userView(Number(m[1]));
  if (p === "/admin/cases") return casesView();
  if ((m = p.match(/^\/admin\/cases\/([a-z0-9]+)$/))) return caseView(m[1]);
  if (p === "/admin/settings") return settingsView();
  if (p === "/admin/log") return logView();
  if (p === "/admin/account") return accountView();
  nav("/admin", true);
}

(async () => {
  try { admin = await api("GET", "/me"); }
  catch (ex) { if (ex.status === 404) return unavailableView(); admin = null; }
  route();
})();
