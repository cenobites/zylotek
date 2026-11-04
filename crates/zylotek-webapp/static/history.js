// The ask history: finished answers kept in this browser only (localStorage), listed in a
// sidebar on the left, so a visitor can open one again without asking the model twice. The site
// works without it; with scripts off or storage blocked the sidebar simply stays hidden.
//
// Every text from storage goes into the page through textContent. The one exception is a saved
// answer, which is the HTML this site rendered (and escaped) itself, put back as it was; only
// this origin can write that storage, and the CSP runs no inline script.
(() => {
  "use strict";

  const KEY = "zylotek.history.v1";
  const MAX = 50;

  const load = () => {
    try {
      const items = JSON.parse(localStorage.getItem(KEY) || "[]");
      return Array.isArray(items) ? items.filter((i) => i && i.id && i.question) : [];
    } catch {
      return [];
    }
  };

  const store = (items) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(items.slice(0, MAX)));
      return true;
    } catch {
      return false;
    }
  };

  const lang = document.documentElement.lang || undefined;
  const when = (iso) => {
    const date = new Date(iso);
    return Number.isNaN(date.getTime())
      ? ""
      : date.toLocaleString(lang, { dateStyle: "medium", timeStyle: "short" });
  };

  const element = (tag, props = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  };

  // The question being shown: the one just answered, or the one restored from `?h=`.
  let current = new URLSearchParams(location.search).get("h");

  // 1. An answer page runs this once it has streamed to the end (the script is deferred):
  // remember the answer, unless the assistant failed.
  const answer = document.querySelector("main.answer[data-question]");
  if (answer && answer.querySelector(".done") && !answer.querySelector(".failed")) {
    const copy = answer.cloneNode(true);
    copy.querySelectorAll(".thinking, .writing").forEach((node) => node.remove());
    const question = answer.dataset.question;
    const items = load().filter((item) => item.question !== question);
    current = Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
    items.unshift({ id: current, question, at: new Date().toISOString(), html: copy.innerHTML });
    // A full storage drops the oldest answers until the new one fits.
    while (items.length && !store(items)) items.pop();
  }

  // 2. The sidebar: newest first, the current question highlighted.
  const sidebar = document.querySelector("[data-sidebar]");
  const list = sidebar && sidebar.querySelector("[data-history-list]");
  const clear = sidebar && sidebar.querySelector("[data-history-clear]");
  const empty = sidebar && sidebar.querySelector("[data-history-empty]");

  const render = () => {
    const items = load();
    list.replaceChildren();
    for (const item of items) {
      const remove = element("button", {
        type: "button",
        className: "remove",
        textContent: "×",
        title: list.dataset.remove,
      });
      remove.setAttribute("aria-label", `${list.dataset.remove}: ${item.question}`);
      remove.addEventListener("click", () => {
        store(load().filter((other) => other.id !== item.id));
        render();
      });
      const link = element("a", {
        href: `/?h=${encodeURIComponent(item.id)}`,
        textContent: item.question,
        title: `${item.question}\n${when(item.at)}`,
      });
      if (item.id === current) link.setAttribute("aria-current", "page");
      list.append(element("li", { className: item.id === current ? "active" : "" }, link, remove));
    }
    empty.hidden = items.length > 0;
    clear.hidden = items.length === 0;
  };

  if (list) {
    clear.addEventListener("click", () => {
      if (window.confirm(clear.dataset.confirm)) {
        store([]);
        render();
      }
    });
    render();
    sidebar.hidden = false;

    // On a phone the sidebar slides in over the page.
    const toggle = document.querySelector("[data-sidebar-toggle]");
    const open = (yes) => {
      document.body.classList.toggle("sidebar-open", yes);
      if (toggle) toggle.setAttribute("aria-expanded", String(yes));
    };
    if (toggle) {
      toggle.hidden = false;
      toggle.addEventListener("click", () => open(!document.body.classList.contains("sidebar-open")));
    }
    document.querySelector("[data-sidebar-close]")?.addEventListener("click", () => open(false));
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") open(false);
    });
  }

  // 3. `/?h=…` on the home page: put a saved answer back in place of the form.
  const slot = document.querySelector("[data-history-answer]");
  if (slot && current) {
    const item = load().find((other) => other.id === current);
    slot.hidden = false;
    const note = slot.querySelector("[data-history-note]");
    if (item) {
      document.querySelector("[data-home]").hidden = true;
      note.textContent = `${slot.dataset.savedOn} ${when(item.at)}`;
      slot.querySelector("[data-history-body]").innerHTML = item.html;
      document.title = `${item.question} · Zylotek`;
    } else {
      // Gone (removed, or another browser's link): say so above the usual form.
      note.textContent = slot.dataset.missing;
    }
  }
})();
