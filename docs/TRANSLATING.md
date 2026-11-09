# Translating the app

Every text the app shows comes from one catalog per language, in
`backend/locales/<language>.ftl`, written in [Fluent](https://projectfluent.org/fluent/guide/).
English (`en.ftl`) is the source: every other language translates its keys.

## Adding a language

1. Copy `backend/locales/en.ftl` to `backend/locales/<code>.ftl`, where `<code>` is the
   language's BCP 47 code (`de`, `fr`, `pt-BR`). The file name is the code.
2. Translate the text after each `=`. Keep the keys (the part before `=`) as they are.
3. Set `settings-language-name` to the language's name in itself ("Deutsch", "Français"):
   Settings' language list shows it.
4. Set `format-decimal-separator` to the language's decimal separator (`.` or `,`).
5. Build the app. The build checks the file (see below), and the new language appears in
   Settings → Language.

A key you leave out falls back to English, so a catalog can be finished bit by bit. Delete
untranslated keys instead of keeping English copies: a copy doesn't follow later changes to
the English text.

## What to translate

- **The text, not the keys or the variables.** `{ $name }`, `{ $count }` and the like are
  filled in by the app. Keep each one, anywhere in the sentence; the comment above a key says
  what each variable holds.
- **Comments** (`# ...`) say where a text shows. They don't need translating.
- **Plurals** select on a number by plural category:

  ```ftl
  usb-warning-count = { $count ->
      [one] { $count } warning
     *[other] { $count } warnings
  }
  ```

  Use your language's categories (`zero`, `one`, `two`, `few`, `many`, `other`, as in the
  [CLDR plural rules](https://www.unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html)),
  and mark one with `*` as the default. A key that doesn't start with `ui-` may also use exact
  numbers (`[0] no tracks`).
- **`ui-` keys** are the texts the window's own labels and buttons show. They may hold text,
  variables and at most one plural on `$count`, using plural categories only (no `[0]`), and
  no other Fluent functions or references.
- **Dates** are shown as `2026-04-07 13:00` in every language, and digits are never grouped.
- **Never translated:** the app's name (DJ USB Tkit), product and format names (rekordbox,
  Mixxx, Essentia, Engine DJ, MP3, FLAC, AIFF, WAV, ALAC, AAC), player models, the PDB/eDB
  table notation in the diagnostics' technical details (`t11`, `num_rl`, `ec=24`), and
  anything written to the USB (playlist names, "Unknown Title"). The log file is always
  English.

## What the build checks

The build fails when a catalog doesn't parse, has a key English doesn't have, uses a
variable English doesn't give that key, or breaks the `ui-` rules above. The error names
the file and the key.

## Checking the layout

Debug builds have a test language, `en-XA`, generated from English: every text in brackets,
accented and about a third longer (`[Ŝéééţţîîñĝš]`). Pick it in Settings → Language to see
which texts are untranslated (no brackets) and where longer text breaks the layout. Releases
don't have it.

## For developers: adding text

All UI text goes through the catalog; English in the UI's JS fails
`backend/tests/ui_text_inventory.rs`, and English in `index.html` fails its HTML check.

- **Rust:** `i18n::tr::<key_with_underscores>(args)` returns an `i18n::Text`, rendered in the
  current language for a response and in English for the log. The build generates one
  function per key, so a wrong key or argument fails at the call site.
- **JS:** `t("ui-...", { name })` for a label or a text shown before a command answers;
  `tCount("ui-...", count)` only for a count with no response to come from. Text with a
  count, or text picked by a decision on response data, comes from the response, built
  with `tr::`.
- **`index.html`:** `data-text="ui-..."`, and `data-text-tooltip`, `-aria-label`,
  `-placeholder`, `-title`, `-alt` for attributes. Text that's never translated gets
  `translate="no"`.
- After adding or changing a `ui-` key, run `UPDATE_UI_TEXTS=1 cargo test -p backend --lib i18n::`
  to refresh the e2e tests' copies of the texts.

### Keys and comments

`[ui-]<area>-<thing>[-<variant>][-<attribute>]`, lowercase kebab-case (ASCII letters,
digits, hyphens).

- **`ui-`** in front of every key JS or `index.html` reads (what `get_ui_texts` sends).
  Keys only Rust uses have no prefix.
- **`<area>`** comes from the list at the top of `en.ftl`: the pages and dialogs, the areas
  only Rust has (`export`, `diagnostics`, `repair`, ...), and shared markup. A new area is
  added to that list.
- **`<thing>`** says what the text is, not where it sits. From an element id: kebab-case it,
  drop type suffixes (`Btn`, `Label`, `Text`) and a leading word the area already gives.
- **`<variant>`** for a state or kind of the same thing: `-none`, `-confirm`, `-failed`.
- **`<attribute>`** last, for text that isn't an element's content: `-tooltip`,
  `-aria-label`, `-placeholder`, `-title`, `-alt`.
- The same English in one area and one use shares a key. Different areas, or uses that
  could translate differently (a button label and a status word), get their own keys.
- Every key with a variable, and every key whose use isn't obvious from its name, gets a
  `#` comment saying where it shows and what each variable is.
- Once a language other than English exists, keys aren't renamed: a rename drops that key
  from every translation.
