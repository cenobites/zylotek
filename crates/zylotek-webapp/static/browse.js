// Browsing, with a little help: the woods ticked for comparison are remembered while the visitor
// searches and pages (in this tab only), shown in a tray at the bottom, and ticking a filter
// applies it at once. Without this script the page still works: the filters have an Apply
// button and the ticked woods are compared with a button.
(() => {
  "use strict";

  const KEY = "zylotek.compare.v1";
  const MAX = 4;
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

    const names = node("span", { className: "flex flex-1 flex-wrap gap-2" });
    for (const wood of picked) {
      const remove = node("button", { type: "button", className: "link-btn", textContent: "✕" });
      remove.setAttribute("aria-label", `${tray.dataset.clear}: ${wood.name}`);
      remove.addEventListener("click", () => {
        picked = picked.filter((other) => other.id !== wood.id);
        store();
        render();
      });
      names.append(node("span", { className: "rounded-full border border-line px-3 py-0.5" }, wood.name, " ", remove));
    }
    const ready = picked.length >= 2;
    const go = node("a", { className: "btn", textContent: `${tray.dataset.compare} (${picked.length})` });
    if (ready) {
      go.href = "/compare?" + picked.map((wood) => `id=${encodeURIComponent(wood.id)}`).join("&");
    } else {
      go.setAttribute("aria-disabled", "true");
    }
    const clear = node("button", { type: "button", className: "link-btn", textContent: tray.dataset.clear });
    clear.addEventListener("click", () => {
      picked = [];
      store();
      render();
    });
    const hint = node("span", {
      className: "text-muted",
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

  document.querySelectorAll("[data-no-js]").forEach((element) => {
    element.hidden = true;
  });
  render();
})();
