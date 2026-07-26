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

function dirty() {
  return Object.keys(draft).length > 0;
}

function syncButtons() {
  $("apply").disabled = !dirty();
  $("revert").disabled = !dirty();
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

    const v = document.createElement("td");
    v.className = "v";
    const input = document.createElement("input");
    input.value = field in draft ? draft[field] : entry.value;
    input.disabled = !entry.reloadable;
    if (entry.reloadable) {
      input.addEventListener("input", () => {
        if (input.value === entry.value) delete draft[field];
        else draft[field] = input.value;
        tag.className = "tag " + (field in draft ? "dirty" : "live");
        tag.textContent = field in draft ? "modified" : "live";
        syncButtons();
      });
    }
    v.appendChild(input);

    const s = document.createElement("td");
    const tag = document.createElement("span");
    if (entry.secret) {
      tag.className = "tag";
      tag.textContent = "secret";
    } else if (entry.reloadable) {
      tag.className = "tag " + (field in draft ? "dirty" : "live");
      tag.textContent = field in draft ? "modified" : "live";
    } else {
      tag.className = "tag";
      tag.textContent = "boot";
    }
    s.appendChild(tag);

    tr.append(k, v, s);
    rows.appendChild(tr);
  }

  $("ovr").textContent = data.overrides.length
    ? "Runtime overrides active (cleared by a reload): " + data.overrides.join(", ")
    : "No runtime overrides active.";
  syncButtons();
}

async function load() {
  render(await call("/config"));
  $("gate").classList.add("hide");
  $("panel").classList.remove("hide");
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
  $("panel").classList.add("hide");
  $("gate").classList.remove("hide");
  say("Locked.", "");
});
