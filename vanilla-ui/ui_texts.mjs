import { cloneTemplate } from "./ui_utils.mjs";

// The UI's text, from the backend's catalog (`get_ui_texts`, keys starting `ui-`): loaded
// once at startup, before anything renders. JS only fills placeholders; the wording, its
// language and its plural forms are the backend's.
//
// - `t(key, { name })`: a label, or a text shown before a command answers.
// - `tCount(key, count, args)`: a text with a count and no response to come from. The
//   backend sends its forms by plural category; `Intl.PluralRules` picks one.
// - `index.html` names its texts with `data-text` (the element's text) and
//   `data-text-tooltip`, `-aria-label`, `-placeholder`, `-title`, `-alt` (that attribute);
//   `applyUiTexts` fills them in, inside `<template>`s too.

let texts = {};
let pluralRules = new Intl.PluralRules("en");
let language = "en";
let languages = [];
let languageChoice = false;
let decimalSeparator = ".";

const TEXT_ATTRIBUTES = [
  ["textTooltip", "data-tooltip"],
  ["textAriaLabel", "aria-label"],
  ["textPlaceholder", "placeholder"],
  ["textTitle", "title"],
  ["textAlt", "alt"]
];

// The only English JS holds: shown when the texts themselves can't be loaded.
const UI_TEXTS_FAILED = "The app couldn't load its texts. Restart it; if this keeps happening, please report it.";

export function setUiTexts(data) {
  texts = data?.texts && typeof data.texts === "object" ? data.texts : {};
  language = data?.language || "en";
  languages = Array.isArray(data?.languages) ? data.languages : [];
  languageChoice = data?.languageChoice === true;
  decimalSeparator = data?.decimalSeparator || ".";
  try {
    pluralRules = new Intl.PluralRules(language);
  } catch {
    pluralRules = new Intl.PluralRules("en");
  }
}

function fill(text, args) {
  return text.replace(/\{([A-Za-z0-9_-]+)\}/g, (match, name) => (name in args ? String(args[name]) : match));
}

// A missing key shows as the key itself, so it's seen (and the test language check catches it).
// The language the texts are in, every language the app has (`{ code, name }`, English
// first), and whether there's one to choose (the backend's `languageChoice`), for the
// language lists in Settings and Quick Start.
export function uiLanguage() {
  return language;
}

export function uiLanguages() {
  return languages;
}

export function uiLanguageChoice() {
  return languageChoice;
}

// `value` with `digits` decimals and the language's decimal separator (the backend's
// `format-decimal-separator`), for numbers JS formats itself: they then read like the
// ones the backend renders. Never for an <input type="number">, which takes `.` only.
export function formatDecimal(value, digits) {
  return Number(value).toFixed(digits).replace(".", decimalSeparator);
}

export function t(key, args = {}) {
  const text = texts[key];
  return typeof text === "string" ? fill(text, args) : key;
}

export function tCount(key, count, args = {}) {
  const forms = texts[key];
  if (!forms || typeof forms !== "object") return key;
  const form = forms[pluralRules.select(count)] ?? forms.other ?? "";
  return fill(form, { ...args, count });
}

export function applyUiTexts(root) {
  const elements = [...root.querySelectorAll("[data-text], [data-text-tooltip], [data-text-aria-label], [data-text-placeholder], [data-text-title], [data-text-alt]")];
  for (const element of elements) {
    if (element.dataset.text) element.textContent = t(element.dataset.text);
    for (const [datasetKey, attribute] of TEXT_ATTRIBUTES) {
      const key = element.dataset[datasetKey];
      if (key) element.setAttribute(attribute, t(key));
    }
  }
  // querySelectorAll doesn't look inside a template's content.
  for (const template of root.querySelectorAll("template")) {
    applyUiTexts(template.content);
  }
}

// Loads the texts and fills in the page. False when they couldn't be loaded: the page then
// shows only the one English line, and the window is shown so the app doesn't seem to hang.
export async function loadUiTexts(ctx) {
  let data;
  try {
    data = await ctx.command("get_ui_texts");
    setUiTexts(data);
  } catch (err) {
    console.error("get_ui_texts failed:", err);
    const line = cloneTemplate(ctx.document, "tplUiTextsFailed");
    line.textContent = UI_TEXTS_FAILED;
    ctx.document.body.replaceChildren(line);
    ctx.invoke("show_window").catch(() => {});
    return false;
  }
  applyUiTexts(ctx.document);
  ctx.document.documentElement.lang = language;
  return true;
}
