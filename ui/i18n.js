const FALLBACK = "en";
const tables = {};
let current = FALLBACK;

export const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "es", label: "Español" },
  { code: "fr", label: "Français" },
  { code: "zh", label: "中文" },
  { code: "ko", label: "한국어" },
];

async function loadTable(code) {
  if (tables[code]) return;
  try {
    tables[code] = (await import(`./locales/${code}.js`)).default;
  } catch (_) {
    tables[code] = {};
  }
}

export async function loadLanguage(code) {
  const wanted = LANGUAGES.some((l) => l.code === code) ? code : FALLBACK;
  await Promise.all([loadTable(FALLBACK), loadTable(wanted)]);
  current = wanted;
}

export function t(key, vars) {
  let s = tables[current]?.[key] ?? tables[FALLBACK]?.[key] ?? key;
  if (vars) s = s.replace(/\{(\w+)\}/g, (m, k) => (vars[k] != null ? String(vars[k]) : m));
  return s;
}

export function applyTranslations() {
  document.documentElement.lang = current;
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.setAttribute("placeholder", t(el.dataset.i18nPlaceholder));
  });
  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.setAttribute("title", t(el.dataset.i18nTitle));
  });
}
