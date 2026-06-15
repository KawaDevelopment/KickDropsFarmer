const FALLBACK = "en";

export function t(lang, key) {
  const locales = window.KDF_LOCALES || {};
  const table = locales[lang] || locales[FALLBACK] || {};
  if (table[key] != null) return table[key];
  const fb = locales[FALLBACK] || {};
  return fb[key] != null ? fb[key] : key;
}

export function applyTranslations(lang) {
  document.documentElement.lang = lang;
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(lang, el.getAttribute("data-i18n"));
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.setAttribute("placeholder", t(lang, el.getAttribute("data-i18n-placeholder")));
  });
  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.setAttribute("title", t(lang, el.getAttribute("data-i18n-title")));
  });
}

export const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "es", label: "Español" },
  { code: "fr", label: "Français" },
  { code: "zh", label: "中文" },
  { code: "ko", label: "한국어" },
];
