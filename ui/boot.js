(function () {
  try {
    const ui = JSON.parse(localStorage.getItem("kdf-ui") || "null");
    if (!ui) return;
    const root = document.documentElement;
    if (ui.style === "retro" || ui.style === "minimal") root.setAttribute("data-style", ui.style);
    if (ui.mode === "dark" || ui.mode === "light") root.setAttribute("data-mode", ui.mode);
  } catch (_) {
    return;
  }
})();
