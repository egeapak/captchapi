// CaptchAPI admin console.
//
// Deliberately dependency-free. The whole page is served from the binary, so every
// byte here is binary size; a framework would cost more than the page is worth.
"use strict";

const $ = (id) => document.getElementById(id);
const API = "/api/v1/admin";

let key = "";
let state = null; // last ConfigResponse from the server
let draft = {}; // field -> edited string, only for live fields

function say(text, kind) {
  const el = $("msg");
  el.textContent = text;
  el.className = "msg on" + (kind ? " " + kind : "");
}

async function call(path, init) {
  const res = await fetch(API + path, {
    ...init,
    headers: {
      "content-type": "application/json",
      authorization: "Bearer " + key,
    },
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.message || res.status + " " + res.statusText);
  return body;
}

function syncButtons() {
  const n = Object.keys(draft).length;
  $("apply").disabled = !n;
  $("revert").disabled = !n;
  $("pend").textContent = n + (n === 1 ? " unsaved edit" : " unsaved edits");
  $("bar").className = "bar" + (n ? " dirty" : "");
}

// Row and tag styling for one field: which class it is, and whether it is edited.
// Purely presentational — the class names drive the stylesheet, nothing reads them back.
//
// `pinned` is a reloadable field this server was started with an explicit value for, so the
// API will refuse to change it. It gets its own tag rather than being lumped in with `boot`,
// because the remedy is different: boot needs a restart, pinned needs the command line or
// environment changed first.
function mark(tr, tag, field, entry) {
  const pinned = entry.reloadable && !entry.editable;
  const cls = entry.secret ? "secret" : pinned ? "pinned" : entry.reloadable ? "live" : "boot";
  const edited = field in draft;
  tr.className = cls + (edited ? " dirty" : "");
  tag.className = "tag " + (edited ? "dirty" : cls);
  tag.textContent = edited ? "modified" : pinned ? "set by " + entry.source : cls;
}

function render(data) {
  state = data;
  const rows = $("rows");
  rows.replaceChildren();

  for (const [field, entry] of Object.entries(data.config)) {
    const tr = document.createElement("tr");

    const k = document.createElement("td");
    k.className = "k";
    k.appendChild(document.createTextNode(field));
    // The server owns the wording, so the console cannot drift from what the binary does.
    if (entry.description) {
        const about = document.createElement("span");
        about.className = "about";
        about.textContent = entry.description;
        k.appendChild(about);
    }

    const s = document.createElement("td");
    s.className = "s";
    const tag = document.createElement("span");
    s.appendChild(tag);

    const v = document.createElement("td");
    v.className = "v";
    const input = document.createElement("input");
    input.value = field in draft ? draft[field] : entry.value;
    // The server is the authority on what it will accept; the console just mirrors it, so
    // the two cannot drift into offering an edit that PATCH would reject.
    input.disabled = !entry.editable;
    if (entry.editable) {
      input.addEventListener("input", () => {
        if (input.value === entry.value) delete draft[field];
        else draft[field] = input.value;
        mark(tr, tag, field, entry);
        syncButtons();
      });
    }
    v.appendChild(input);

    mark(tr, tag, field, entry);

    tr.append(k, v, s);
    rows.appendChild(tr);
  }

  // Boot fields that a restart would change. Stored or not: an edited env file shows up here
  // too, so the wording says what a restart would do rather than naming a cause.
  const waiting = data.pending_restart || [];
  const restart = $("restart");
  restart.replaceChildren();
  if (waiting.length) {
    const label = document.createElement("b");
    label.textContent =
      waiting.length === 1 ? "1 setting needs a restart:" : waiting.length + " settings need a restart:";
    const names = document.createElement("code");
    names.textContent = waiting.join(", ");
    restart.append(label, names);
  }
  restart.classList.toggle("hide", !waiting.length);

  // Only say something when there is something to say — "no overrides active" is the
  // normal state and does not need a line of its own.
  $("ovr").textContent = data.overrides.length
    ? "Overrides active until the next reload: " + data.overrides.join(", ")
    : "";
  $("ovr").classList.toggle("hide", !data.overrides.length);
  syncButtons();
}

// `gated` on <body> switches the page between the centred unlock card and the full console.
function gated(on) {
  document.body.className = on ? "gated" : "";
  $("gate").classList.toggle("hide", !on);
  $("panel").classList.toggle("hide", on);
}

async function load() {
  render(await call("/config"));
  gated(false);
}

$("gate").addEventListener("submit", async (e) => {
  e.preventDefault();
  key = $("key").value;
  try {
    // No success message: the table appearing is the confirmation.
    await load();
    $("msg").className = "msg";
  } catch (err) {
    key = "";
    say(String(err.message || err), "err");
  }
});

$("apply").addEventListener("click", async () => {
  try {
    const sent = Object.keys(draft).join(", ");
    const data = await call("/config", { method: "PATCH", body: JSON.stringify(draft) });
    draft = {};
    render(data);
    say("Applied: " + sent, "ok");
  } catch (err) {
    say(String(err.message || err), "err");
  }
});

$("revert").addEventListener("click", () => {
  draft = {};
  render(state);
  say("Reverted unsaved edits.", "");
});

$("reload").addEventListener("click", async () => {
  try {
    const data = await call("/config/reload", { method: "POST" });
    draft = {};
    render(data);
    say(
      data.message + (data.ignored.length ? " — needs a restart: " + data.ignored.join(", ") : ""),
      data.ignored.length ? "" : "ok",
    );
  } catch (err) {
    say(String(err.message || err), "err");
  }
});

$("cleanup").addEventListener("click", async () => {
  try {
    say((await call("/cleanup", { method: "POST" })).message, "ok");
  } catch (err) {
    say(String(err.message || err), "err");
  }
});

$("lock").addEventListener("click", () => {
  key = "";
  draft = {};
  state = null;
  $("key").value = "";
  gated(true);
  say("Locked.", "");
});
