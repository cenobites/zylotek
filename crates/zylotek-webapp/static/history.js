// The ask history: the questions asked in this browser, listed in a sidebar on the left. Each
// entry is only an ask's id, its question and when it was asked; the answer itself is stored by
// the site, so opening an entry (`/asks/<id>`) shows the answer as far as it has got, even one
// still being worked on. The site works without this script; with scripts off or storage blocked
// the sidebar simply stays hidden.
//
// On an ask's page this script runs as soon as the page starts (it is not deferred), so the
// ask is remembered at once, before its answer is in, and leaving early does not lose it.
(() => {
  "use strict";

  const KEY = "zylotek.history.v2";
  const MAX = 50;

  // The first history kept whole answers, which the site now stores itself.
  try {
    localStorage.removeItem("zylotek.history.v1");
  } catch {
    // Storage is off; there is nothing to clean up.
  }

  const load = () => {
    try {
      const items = JSON.parse(localStorage.getItem(KEY) || "[]");
      return Array.isArray(items)
        ? items.filter((i) => i && typeof i.id === "string" && typeof i.question === "string")
        : [];
    } catch {
      return [];
    }
  };

  const store = (items) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(items.slice(0, MAX)));
    } catch {
      // Storage is full or off: the history just does not grow.
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

  // 1. An ask's page: remember the ask, once, in the order it was first asked.
  const page = document.querySelector("main[data-ask-id]");
  const current = page ? page.dataset.askId : null;
  if (page && !load().some((item) => item.id === current)) {
    store([{ id: current, question: page.dataset.question, at: new Date().toISOString() }, ...load()]);
  }

  // 2. The sidebar: newest first, the current ask highlighted. It comes before the page's main
  // content, so it is already there when this runs.
  const sidebar = document.querySelector("[data-sidebar]");
  const list = sidebar && sidebar.querySelector("[data-history-list]");
  if (!list) return;
  const clear = sidebar.querySelector("[data-history-clear]");
  const empty = sidebar.querySelector("[data-history-empty]");

  const render = () => {
    const items = load();
    list.replaceChildren();
    for (const item of items) {
      const remove = element("button", {
        type: "button",
        className:
          "cursor-pointer border-0 bg-transparent px-2 py-0.5 text-muted opacity-0 group-hover/item:opacity-100 focus-visible:opacity-100 group-[.active]/item:opacity-100",
        textContent: "×",
        title: list.dataset.remove,
      });
      remove.setAttribute("aria-label", `${list.dataset.remove}: ${item.question}`);
      remove.addEventListener("click", () => {
        store(load().filter((other) => other.id !== item.id));
        render();
      });
      const link = element("a", {
        className: "min-w-0 flex-1 truncate px-2 py-1.5 text-fg no-underline",
        href: `/asks/${encodeURIComponent(item.id)}`,
        textContent: item.question,
        title: `${item.question}\n${when(item.at)}`,
      });
      if (item.id === current) link.setAttribute("aria-current", "page");
      list.append(
        element(
          "li",
          {
            className:
              "group/item flex items-center rounded-lg hover:bg-line" + (item.id === current ? " active bg-line" : ""),
          },
          link,
          remove,
        ),
      );
    }
    empty.hidden = items.length > 0;
    clear.hidden = items.length === 0;
  };

  clear.addEventListener("click", () => {
    if (window.confirm(clear.dataset.confirm)) {
      store([]);
      render();
    }
  });
  render();
  sidebar.hidden = false;

  // The sidebar slides in over the page.
  const toggle = document.querySelector("[data-sidebar-toggle]");
  const open = (yes) => {
    // The stylesheet slides the sidebar in when the body says it is open.
    if (yes) document.body.dataset.sidebar = "open";
    else delete document.body.dataset.sidebar;
    if (toggle) toggle.setAttribute("aria-expanded", String(yes));
  };
  if (toggle) {
    toggle.hidden = false;
    toggle.addEventListener("click", () => open(document.body.dataset.sidebar !== "open"));
  }
  document.querySelector("[data-sidebar-close]")?.addEventListener("click", () => open(false));
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") open(false);
  });
})();
