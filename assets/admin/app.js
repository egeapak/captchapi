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

// Row and tag styling for one field: which of the three classes it is, and whether it is edited.
// Purely presentational — the class names drive the stylesheet, nothing reads them back.
function mark(tr, tag, field, entry) {
  const cls = entry.secret ? "secret" : entry.reloadable ? "live" : "boot";
  const edited = field in draft;
  tr.className = cls + (edited ? " dirty" : "");
  tag.className = "tag " + (edited ? "dirty" : cls);
  tag.textContent = edited ? "modified" : cls;
}

function render(data) {
  state = data;
  const rows = $("rows");
  rows.replaceChildren();

  for (const [field, entry] of Object.entries(data.config)) {
    const tr = document.createElement("tr");

    const k = document.createElement("td");
    k.className = "k";
    k.textContent = field;

    const s = document.createElement("td");
    s.className = "s";
    const tag = document.createElement("span");
    s.appendChild(tag);

    const v = document.createElement("td");
    v.className = "v";
    const input = document.createElement("input");
    input.value = field in draft ? draft[field] : entry.value;
    input.disabled = !entry.reloadable;
    if (entry.reloadable) {
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

  $("ovr").textContent = data.overrides.length
    ? "Runtime overrides active (cleared by a reload): " + data.overrides.join(", ")
    : "No runtime overrides active.";
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
    await load();
    say("Connected.", "ok");
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
