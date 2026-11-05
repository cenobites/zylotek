// Browsing, with a little help: the woods ticked for comparison are remembered while the visitor
// searches and pages (in this tab only), shown in a tray at the bottom, and ticking a filter
// applies it at once. Without this script the page still works: the filters have an Apply
// button and the ticked woods are compared with a button.
(() => {
  "use strict";

  const KEY = "zylotek.compare.v1";
  const MAX = 4;
  // From a comparison, "add another wood" carries the woods already compared into the tray.
  document.querySelector("[data-compare-add]")?.addEventListener("click", () => {
    const woods = [...document.querySelectorAll("[data-compare-wood]")].map((th) => ({ id: th.dataset.compareWood, name: th.dataset.name }));
    try {
      sessionStorage.setItem(KEY, JSON.stringify(woods.slice(0, MAX)));
    } catch {
      // Without storage the browse page simply starts empty.
    }
  });
  const tray = document.querySelector("[data-compare-tray]");
  const form = document.getElementById("filters");
  if (!tray || !form) return;

  const load = () => {
    try {
      const items = JSON.parse(sessionStorage.getItem(KEY) || "[]");
      return Array.isArray(items) ? items.filter((i) => i && i.id && i.name).slice(0, MAX) : [];
    } catch {
      return [];
    }
  };
  const store = () => {
    try {
      sessionStorage.setItem(KEY, JSON.stringify(picked));
    } catch {
      // Picks then last only as long as this page.
    }
  };

  let picked = load();
  const boxes = [...document.querySelectorAll("input[data-compare]")];

  const node = (tag, props = {}, ...children) => {
    const element = Object.assign(document.createElement(tag), props);
    element.append(...children);
    return element;
  };

  const render = () => {
    for (const box of boxes) {
      box.checked = picked.some((wood) => wood.id === box.value);
      // Past the limit the other woods cannot be added until one is taken off.
      box.disabled = !box.checked && picked.length >= MAX;
    }
    tray.replaceChildren();
    tray.hidden = picked.length === 0;
    if (picked.length === 0) return;

    const names = node("span", { className: "flex flex-1 flex-wrap items-center gap-2" });
    for (const wood of picked) {
      const remove = node("button", {
        type: "button",
        className: "grid size-5 cursor-pointer place-items-center rounded-full border-0 bg-transparent text-muted hover:bg-line hover:text-fg",
        textContent: "✕",
      });
      remove.setAttribute("aria-label", `${tray.dataset.clear}: ${wood.name}`);
      remove.addEventListener("click", () => {
        picked = picked.filter((other) => other.id !== wood.id);
        store();
        render();
      });
      names.append(
        node("span", { className: "inline-flex items-center gap-1 rounded-full bg-subtle py-1 pr-1 pl-3 text-sm" }, wood.name, remove),
      );
    }
    const ready = picked.length >= 2;
    const go = node("a", { className: "btn py-2 text-sm", textContent: `${tray.dataset.compare} (${picked.length})` });
    if (ready) {
      go.href = "/compare?" + picked.map((wood) => `id=${encodeURIComponent(wood.id)}`).join("&");
    } else {
      go.setAttribute("aria-disabled", "true");
    }
    const clear = node("button", { type: "button", className: "link-btn text-muted", textContent: tray.dataset.clear });
    clear.addEventListener("click", () => {
      picked = [];
      store();
      render();
    });
    const hint = node("span", {
      className: "text-sm text-muted",
      textContent: picked.length >= MAX ? tray.dataset.full : ready ? "" : tray.dataset.hint,
    });
    tray.append(names, hint, go, clear);
  };

  for (const box of boxes) {
    box.addEventListener("change", () => {
      picked = picked.filter((wood) => wood.id !== box.value);
      if (box.checked && picked.length < MAX) picked.push({ id: box.value, name: box.dataset.name });
      store();
      render();
    });
  }

  // Ticking a filter or choosing a sort order applies it; typing waits for Enter.
  form.addEventListener("change", (event) => {
    if (event.target.matches("input[type=checkbox]:not([data-compare]), select, input[type=number]")) {
      form.requestSubmit();
    }
  });

  // On a phone the filters fold away above the results; their summary opens them again.
  const filters = document.querySelector("[data-filters]");
  if (filters && window.matchMedia("(max-width: 767px)").matches) filters.open = false;

  document.querySelectorAll("[data-no-js]").forEach((element) => {
    element.hidden = true;
  });
  render();
})();
